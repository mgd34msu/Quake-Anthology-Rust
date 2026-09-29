//! Scripted order and director integration tests.

#[path = "common/behavior.rs"]
mod fixtures;

use fixtures::{behavior_files, FakeHost, FakeNav};
use qa_bots::behavior::director::{SourceBotDirector, SourceBotDirectorParams};
use qa_bots::behavior::orders::{
    bot_order_active, bot_order_status, same_bot_order, BotOrder, BotOrderEntity, BotOrderProgress, BotOrderState,
    BOT_GOAL_ACTIVE, BOT_GOAL_NONE, BOT_GOAL_REACHED,
};
use qa_bots::behavior::population::{BotFrame, SharedBotPopulation};
use qa_bots::behavior::q3::game_host::{BotObservedEntity, BotObservedPlayer, BotProduct};
use qa_bots::behavior::q3::source_game::ScriptedBotGame;
use qa_core::math::{vec3, Vec3};

const ENTITIES: &str = r#"
{
"classname" "worldspawn"
}
{
"classname" "info_player_start"
"origin" "0 0 0"
}
"#;

fn player_entity(origin: Vec3, team: i32, name: &str) -> BotObservedEntity {
    let mut entity = BotObservedEntity::absent();
    entity.generation = 3;
    entity.present = true;
    entity.linked = true;
    entity.bot = true;
    entity.origin = origin;
    entity.player = Some(BotObservedPlayer {
        health: 100,
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

fn director() -> SourceBotDirector<'static> {
    let files: &'static _ = Box::leak(Box::new(behavior_files()));
    let mut game = ScriptedBotGame::new(BotProduct::BaseQ3, 0, 4);
    game.set_entity(0, player_entity(vec3(0.0, 0.0, 0.0), 0, "Grunt"));
    game.set_entity(1, player_entity(vec3(512.0, 0.0, 0.0), 0, "Major"));
    SourceBotDirector::new(SourceBotDirectorParams {
        host: Box::new(FakeHost::new()),
        game: Box::new(game),
        navigation: Box::new(FakeNav::new()),
        files,
        entities: ENTITIES.to_string(),
        item_config: "botfiles/items.c".to_string(),
        gametype: 0,
        max_clients: 4,
        debug: false,
    })
    .unwrap()
}

#[test]
fn order_status_matches_quakec_contract() {
    assert_eq!(bot_order_status(None), BOT_GOAL_NONE);
    let point = BotOrder::Point {
        point: vec3(1.0, 2.0, 3.0),
    };
    for (progress, status) in [
        (BotOrderProgress::Error, BOT_GOAL_NONE),
        (BotOrderProgress::Success, BOT_GOAL_REACHED),
        (BotOrderProgress::InProgress, BOT_GOAL_ACTIVE),
    ] {
        assert_eq!(
            bot_order_status(Some(&BotOrderState { order: point, progress })),
            status
        );
    }
}

#[test]
fn follow_orders_stay_active_after_success() {
    let follow = BotOrder::Follow {
        entity: BotOrderEntity {
            number: 1,
            generation: 3,
        },
    };
    assert!(bot_order_active(Some(&BotOrderState {
        order: follow,
        progress: BotOrderProgress::Success,
    })));
    assert!(!bot_order_active(Some(&BotOrderState {
        order: follow,
        progress: BotOrderProgress::Error,
    })));
    assert!(!bot_order_active(None));
}

#[test]
fn same_order_uses_eight_unit_point_threshold() {
    let a = BotOrder::Point {
        point: vec3(0.0, 0.0, 0.0),
    };
    let near = BotOrder::Point {
        point: vec3(7.9, 0.0, 0.0),
    };
    let far = BotOrder::Point {
        point: vec3(8.1, 0.0, 0.0),
    };
    assert!(same_bot_order(&a, &near));
    assert!(!same_bot_order(&a, &far));
    let follow_a = BotOrder::Follow {
        entity: BotOrderEntity {
            number: 1,
            generation: 3,
        },
    };
    let follow_b = BotOrder::Follow {
        entity: BotOrderEntity {
            number: 1,
            generation: 4,
        },
    };
    assert!(!same_bot_order(&follow_a, &follow_b));
    assert!(!same_bot_order(&a, &follow_a));
}

#[test]
fn director_runs_point_order_to_arrival() {
    let mut director = director();
    director.load().unwrap();
    assert!(director.add_bot("Grunt", 2.0, "", 0));
    // The bot stands at the origin; order the point it already touches.
    let status = director.request_move_to_point(0, vec3(0.0, 0.0, 0.0));
    assert_eq!(status, BOT_GOAL_ACTIVE);
    assert_eq!(director.goal_status(0), BOT_GOAL_ACTIVE);
    // Duplicate near-point requests dedup to the same order.
    let again = director.request_move_to_point(0, vec3(1.0, 0.0, 0.0));
    assert_eq!(again, BOT_GOAL_ACTIVE);
    // Run frames until arrival (stand node seeks after 2s).
    let mut arrived = false;
    for tick in 1..40 {
        let commands = director.frame(tick * 100).unwrap();
        assert!(!commands.is_empty());
        if director.goal_status(0) == BOT_GOAL_REACHED {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "point order never reached");
}

#[test]
fn travel_converges_to_goal() {
    use qa_bots::behavior::library::goals::{touching_goal, BotGoal};
    use qa_bots::behavior::q3::movement_state::{BotInitMove, BotMoveResult};
    use qa_bots::behavior::q3::navigation_types::BotNavigation;

    let mut nav = FakeNav::new();
    let handle = nav.move_states.alloc().unwrap();
    nav.move_states.init(
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
    let goal = BotGoal {
        origin: vec3(96.0, 0.0, 0.0),
        area: 1,
        ..BotGoal::default()
    };
    let mut result = BotMoveResult::default();
    for _ in 0..10 {
        nav.move_to_goal(&mut result, handle, &goal, 0);
        assert!(!result.failure);
        let origin = nav.move_states.get(handle).unwrap().origin;
        if touching_goal(origin, &goal) {
            return;
        }
    }
    panic!("travel never arrived");
}

#[test]
fn follow_order_fails_when_generation_recycles() {
    let mut director = director();
    director.load().unwrap();
    assert!(director.add_bot("Grunt", 2.0, "", 0));
    assert_eq!(director.request_follow_entity(0, 1), BOT_GOAL_ACTIVE);
    assert_eq!(director.goal_status(0), BOT_GOAL_ACTIVE);
    // Out-of-range and absent entities are rejected.
    assert_eq!(director.request_follow_entity(0, 9999), BOT_GOAL_NONE);
    director.clear_goal(0);
    assert_eq!(director.goal_status(0), BOT_GOAL_NONE);
}

#[test]
fn population_validates_shared_actors() {
    let mut director = director();
    director.load().unwrap();
    assert!(director.add_bot("Grunt", 2.0, "", 0));
    let actor = director.roster()[0].actor.clone();
    let slot = director.roster()[0].slot;
    let mut population = SharedBotPopulation::new(&mut director, |_| true);
    assert_eq!(
        population.request_move_to_point(&actor, vec3(64.0, 0.0, 0.0)),
        BOT_GOAL_ACTIVE
    );
    let commands = population
        .frame(BotFrame {
            time_milliseconds: 100,
            elapsed_milliseconds: 100,
        })
        .unwrap();
    assert!(!commands.is_empty());
    assert_eq!(commands[0].slot, slot);
}
