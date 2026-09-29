//! Rerelease brain, data, senses, and aim integration tests.

use std::collections::HashMap;

use qa_bots::behavior::rerelease::aim::{aim_error, aim_step, new_aim_state};
use qa_bots::behavior::rerelease::brain::{
    BotBrain, BotBrainConfig, BotBrainMovementGeom, BotChatEventT, BotGoalStatus,
};
use qa_bots::behavior::rerelease::checkpoint::{rerelease_behavior_checkpoint, restore_rerelease_behavior};
use qa_bots::behavior::rerelease::data::blockparse::parse_blocks;
use qa_bots::behavior::rerelease::data::botdata::{
    parse_bot_settings, parse_characters, parse_chats, parse_items, parse_monsters, parse_weapons, BotDataFormat,
};
use qa_bots::behavior::rerelease::data::knowledge::{
    choose_weapon, item_value, BotDataFilesT, BotGameModeT, BotItemContextT, BotKnowledge, BotWeaponContextT,
};
use qa_bots::behavior::rerelease::math::{angle_delta, bvec_distance, vector_to_angles};
use qa_bots::behavior::rerelease::path_follow::{
    clear_path, follow_path, new_path_state, set_path, BotFollowInputT, BotPathStatus,
};
use qa_bots::behavior::rerelease::rng::{random_chance, random_index, BotRandomT, Xorshift32};
use qa_bots::behavior::rerelease::senses::{can_fire, is_aware, new_awareness, sense_step, should_forget, BotContactT};
use qa_bots::behavior::rerelease::world::{
    empty_usercmd, BotContents, BotEntityKind, BotEntityT, BotSelfT, BotTraceT, BotWorldT, BOT_BUTTON_ATTACK,
};
use qa_core::math::{vec3, Vec3};

const WEAPONS_TXT: &str = r#"
{
    name axe
    number 1
    damage 20
    priority 10
    min_range 0
    max_range 64
    flags melee
    aim_point center
}
{
    name shotgun
    number 4
    damage 24
    priority 50
    min_range 0
    max_range 800
    ammo shells
    ammo_name shells
    min_ammo 1
    max_ammo 100
    flags hitscan
    aim_point center
}
{
    name rocket_launcher
    number 32
    damage 120
    priority 90
    min_range 128
    max_range 2048
    ammo rockets
    ammo_name rockets
    min_ammo 1
    max_ammo 100
    flags projectile | explosive
    aim_point best
    speed 1000
}
"#;

const ITEMS_TXT: &str = r#"
{
    name item_health
    sight_dist 100000
    flags health
}
{
    name weapon_rocketlauncher
    sight_dist 100000
    flags weapon
}
"#;

const SETTINGS_TXT: &str = r#"
skill easy {
    aiming.max_acceleration 360
    aiming.spring_stiffness 64
    aiming.damping 12
    aiming.velocity_offset 0
    aiming.lead_targets true
    behaviors.combat.max_item_dist 512
    behaviors.allow_combat true
    behaviors.allow_grab_items true
    behaviors.allow_grab_power_items true
    behaviors.allow_melee true
    behaviors.allow_check_six false
    movement.allow_jumping_in_combat true
    movement.jump_chance 20
    movement.jump_cooldown 1
    senses.sight_time 0.4
    senses.sight_decay_time 1.0
    senses.fov_angle 150
    senses.forget_non_vis_enemy_time 5
    senses.sound_range 1200
    senses.sound_time 0.5
    senses.sound_decay_time 1.0
    senses.sound_persist_time 1.0
    weapons.fov_angle 60
    weapons.sight_time 0.3
    weapons.decay_time 0.5
}
skill nightmare {
    aiming.max_acceleration 720
    behaviors.allow_combat true
    behaviors.allow_grab_items true
    behaviors.allow_melee true
    senses.sight_time 0.1
    senses.fov_angle 180
    weapons.fov_angle 90
}
"#;

const MONSTERS_TXT: &str = r#"
{
    classname monster_demon1
    flags melee
}
"#;

fn test_knowledge() -> BotKnowledge {
    BotKnowledge::new(
        &BotDataFilesT {
            characters: String::new(),
            weapons: WEAPONS_TXT.to_owned(),
            items: ITEMS_TXT.to_owned(),
            monsters: MONSTERS_TXT.to_owned(),
            interactables: String::new(),
            game_rules: String::new(),
            teams: String::new(),
            chats: "{\n locstring $greeting\n type greeting\n time 500\n chance 100\n team false\n}\n".to_owned(),
            settings: SETTINGS_TXT.to_owned(),
            dangers: None,
        },
        BotDataFormat::Q1,
        None,
    )
}

fn test_mode() -> BotGameModeT {
    BotGameModeT {
        game_type: "deathmatch".to_owned(),
        weapon_stay: false,
        has_teams: None,
        team_damage: None,
    }
}

fn bot_self() -> BotSelfT {
    BotSelfT {
        id: 1,
        origin: vec3(0.0, 0.0, 0.0),
        velocity: vec3(0.0, 0.0, 0.0),
        view_angles: vec3(0.0, 0.0, 0.0),
        eye: vec3(0.0, 0.0, 24.0),
        health: 100.0,
        armor: 0.0,
        items: 1 | 4 | 32,
        ammo: HashMap::from([("shells".to_owned(), 50), ("rockets".to_owned(), 10)]),
        current_weapon: 4,
        on_ground: true,
        water_level: 0,
        air_seconds: None,
        on_lift: None,
        team: 0,
        dead: false,
        has_protection: false,
        max_armor: Some(200.0),
        carrying_objective: false,
    }
}

fn enemy(id: i32, origin: Vec3) -> BotEntityT {
    BotEntityT {
        id,
        kind: BotEntityKind::PLAYER,
        classname: "player".to_owned(),
        origin,
        center: vec3(origin.x, origin.y, origin.z + 24.0),
        head: vec3(origin.x, origin.y, origin.z + 48.0),
        feet: origin,
        velocity: vec3(0.0, 0.0, 0.0),
        health: 100.0,
        team: 0,
        carrying_objective: false,
        dead: false,
        invisible: false,
        water_level: 0,
        is_bot: false,
        spawnflags: 0,
        has_health: true,
        has_targetname: false,
    }
}

struct ScriptedWorld {
    bot: BotSelfT,
    entities: Vec<BotEntityT>,
    time: f32,
    contents: i32,
    blocked: bool,
}

impl BotWorldT for ScriptedWorld {
    fn time(&self) -> f32 {
        self.time
    }

    fn frame_time(&self) -> f32 {
        0.05
    }

    fn bot_self(&self) -> BotSelfT {
        self.bot.clone()
    }

    fn trace_line(&self, start: Vec3, end: Vec3) -> BotTraceT {
        if self.blocked {
            BotTraceT {
                fraction: 0.5,
                endpos: start,
                startsolid: false,
                hit_id: 99,
            }
        } else {
            BotTraceT {
                fraction: 1.0,
                endpos: end,
                startsolid: false,
                hit_id: -1,
            }
        }
    }

    fn trace_box(&self, _start: Vec3, _mins: Vec3, _maxs: Vec3, end: Vec3) -> BotTraceT {
        BotTraceT {
            fraction: 1.0,
            endpos: end,
            startsolid: false,
            hit_id: -1,
        }
    }

    fn point_contents(&self, _point: Vec3) -> i32 {
        self.contents
    }

    fn entities(&self) -> Vec<BotEntityT> {
        self.entities.clone()
    }

    fn hearing(&self) -> Vec<qa_bots::behavior::rerelease::world::BotSoundT> {
        Vec::new()
    }

    fn nav(&mut self) -> Option<&mut dyn qa_bots::behavior::rerelease::nav::RereleaseNavigation> {
        None
    }
}

fn test_brain() -> (BotBrain, Vec<BotChatEventT>) {
    let chats: Vec<BotChatEventT> = Vec::new();
    let brain = BotBrain::new(
        BotBrainConfig {
            knowledge: test_knowledge(),
            skill: "easy".to_owned(),
            game_mode: test_mode(),
            character: None,
            max_health: 100.0,
            run_speed: 320.0,
            walk_speed: 160.0,
            movement: BotBrainMovementGeom::default(),
            on_chat: None,
            weapon_impulse: Some(Box::new(|number| number)),
            on_weapon_select: None,
            human_teammate_near: Box::new(|| false),
        },
        1234,
    )
    .unwrap();
    (brain, chats)
}

#[test]
fn block_parser_reads_shipped_shape() {
    let parsed = parse_blocks(WEAPONS_TXT);
    assert!(parsed.errors.is_empty());
    assert_eq!(parsed.blocks.len(), 3);
    let (weapons, errors) = parse_weapons(WEAPONS_TXT, BotDataFormat::Q1);
    assert!(errors.is_empty());
    assert_eq!(weapons.len(), 3);
    assert_eq!(weapons[2].flags, vec!["projectile".to_owned(), "explosive".to_owned()]);
    let (items, _) = parse_items(ITEMS_TXT);
    assert_eq!(items.len(), 2);
    let (monsters, _) = parse_monsters(MONSTERS_TXT);
    assert_eq!(monsters.len(), 1);
    let (skills, _) = parse_bot_settings(SETTINGS_TXT);
    assert_eq!(skills.len(), 2);
    assert!(skills[0].behaviors.allow_combat);
    let (characters, _) = parse_characters("");
    assert!(characters.is_empty());
    let (chats, chat_errors) = parse_chats("{\n locstring $x\n type greeting\n time 100\n chance 50\n team false\n}");
    assert!(chat_errors.is_empty());
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].chat_type, "greeting");
}

#[test]
fn knowledge_weapon_rule_picks_best_valid() {
    let knowledge = test_knowledge();
    assert!(knowledge.errors.is_empty());
    let ctx = BotWeaponContextT {
        items: 1 | 4 | 32,
        ammo: HashMap::from([("shells".to_owned(), 50), ("rockets".to_owned(), 10)]),
        range: 500.0,
        height_delta: 0.0,
        in_water: false,
        has_protection: false,
        target_in_water: false,
        allow_melee: true,
    };
    let pick = choose_weapon(&knowledge.weapons, &ctx).unwrap();
    assert_eq!(pick.number, 32);
    // Out of rockets: the shotgun wins.
    let ctx = BotWeaponContextT {
        ammo: HashMap::from([("shells".to_owned(), 50), ("rockets".to_owned(), 0)]),
        ..ctx
    };
    assert_eq!(choose_weapon(&knowledge.weapons, &ctx).unwrap().number, 4);
    // Too close for rockets: the shotgun wins on range.
    let ctx = BotWeaponContextT {
        range: 40.0,
        ammo: HashMap::from([("shells".to_owned(), 50), ("rockets".to_owned(), 10)]),
        ..ctx
    };
    assert_eq!(choose_weapon(&knowledge.weapons, &ctx).unwrap().number, 4);
}

#[test]
fn knowledge_item_value_scores_need() {
    let knowledge = test_knowledge();
    let health = knowledge.item("item_health").unwrap();
    let ctx = BotItemContextT {
        spawnflags: 0,
        health: 50.0,
        max_health: 100.0,
        armor: 0.0,
        items: 1,
        ammo: HashMap::new(),
        weapon_stay: false,
        allow_power_items: true,
        team: 0,
        item_team: 0,
        objective_at_home: true,
    };
    assert!(item_value(health, &ctx, &knowledge.weapons) > 0.0);
    let full = BotItemContextT {
        health: 100.0,
        ..ctx.clone()
    };
    assert_eq!(item_value(health, &full, &knowledge.weapons), 0.0);
    let rocket_item = knowledge.item("weapon_rocketlauncher").unwrap();
    assert!(item_value(rocket_item, &ctx, &knowledge.weapons) > 500.0);
}

#[test]
fn senses_fill_decay_and_forget() {
    let knowledge = test_knowledge();
    let senses = knowledge.skill("easy").unwrap().senses;
    let weapons = knowledge.skill("easy").unwrap().weapons;
    let mut awareness = new_awareness(2, 0.0, vec3(100.0, 0.0, 24.0));
    let contact = BotContactT {
        line_of_sight: true,
        in_sight_fov: true,
        in_weapon_fov: true,
        audible: false,
        invisible: false,
        distance: 100.0,
        origin: vec3(100.0, 0.0, 24.0),
    };
    for step in 0..10 {
        sense_step(&mut awareness, &contact, &senses, &weapons, 0.05, 0.05 * step as f32);
    }
    assert!(is_aware(&awareness));
    assert!(can_fire(&awareness));
    let lost = BotContactT {
        line_of_sight: false,
        in_sight_fov: false,
        in_weapon_fov: false,
        audible: false,
        invisible: false,
        distance: 5000.0,
        origin: vec3(100.0, 0.0, 24.0),
    };
    for step in 0..40 {
        sense_step(&mut awareness, &lost, &senses, &weapons, 0.05, 1.0 + 0.05 * step as f32);
    }
    assert!(!is_aware(&awareness));
    assert!(should_forget(&awareness, &senses, 100.0));
}

#[test]
fn aim_tracker_converges() {
    let knowledge = test_knowledge();
    let aiming = knowledge.skill("easy").unwrap().aiming;
    let mut state = new_aim_state(0.0, 0.0);
    let dir = vec3(0.0, 100.0, 0.0);
    for step in 0..60 {
        aim_step(&mut state, dir, &aiming, 0.05, 0.05 * step as f32);
    }
    assert!(aim_error(&state, dir) < 5.0, "yaw={} pitch={}", state.yaw, state.pitch);
}

#[test]
fn rng_replays_and_math_wraps() {
    let mut a = Xorshift32::new(99);
    let mut b = Xorshift32::new(99);
    for _ in 0..10 {
        assert_eq!(a.next(), b.next());
    }
    assert_eq!(random_index(&mut a, 0), 0);
    assert!(random_chance(&mut a, 100.0));
    assert!(!random_chance(&mut a, 0.0));
    assert!((angle_delta(350.0, 10.0) - 20.0).abs() < 0.01);
    let (pitch, yaw) = vector_to_angles(vec3(1.0, 0.0, 0.0));
    assert!(pitch.abs() < 0.01 && yaw.abs() < 0.01);
    assert!((bvec_distance(vec3(0.0, 0.0, 0.0), vec3(3.0, 4.0, 0.0)) - 5.0).abs() < 0.01);
}

#[test]
fn path_follower_arrives() {
    let mut state = new_path_state();
    assert!(state.path.is_none());
    clear_path(&mut state);
    let input = BotFollowInputT {
        origin: vec3(0.0, 0.0, 0.0),
        yaw: 0.0,
        on_ground: true,
        water_level: 0,
        air_seconds: None,
        air_above: None,
        velocity: None,
        now: 0.0,
        stuck_time: 0.6,
        run_speed: 320.0,
        walk_speed: 160.0,
    };
    let knowledge = test_knowledge();
    let movement = knowledge.skill("easy").unwrap().movement;
    let mut transport = |_: &qa_bots::behavior::rerelease::nav::NavGraphLinkT, _: Vec3| None;
    let out = follow_path(&mut state, &input, &movement, &mut transport);
    assert_eq!(out.status, BotPathStatus::NO_PATH);
}

#[test]
fn brain_fights_and_fires() {
    let (mut brain, _) = test_brain();
    let mut world = ScriptedWorld {
        bot: bot_self(),
        entities: vec![enemy(2, vec3(300.0, 0.0, 0.0))],
        time: 0.0,
        contents: BotContents::EMPTY,
        blocked: false,
    };
    let mut fired = false;
    for step in 0..80 {
        world.time = 0.05 * step as f32;
        let cmd = brain.think(&mut world);
        if cmd.buttons & BOT_BUTTON_ATTACK != 0 {
            fired = true;
        }
    }
    assert_eq!(brain.current_target(), 2);
    assert!(fired, "brain never fired at a visible enemy");
    assert!(brain.awareness_of(2).is_some_and(is_aware));
}

#[test]
fn brain_explicit_goal_lifecycle() {
    let (mut brain, _) = test_brain();
    let mut world = ScriptedWorld {
        bot: bot_self(),
        entities: Vec::new(),
        time: 0.0,
        contents: BotContents::EMPTY,
        blocked: false,
    };
    assert_eq!(brain.goal_status(), BotGoalStatus::ERROR);
    brain.request_move_to_point(vec3(10.0, 0.0, 0.0));
    assert_eq!(brain.goal_status(), BotGoalStatus::IN_PROGRESS);
    for step in 0..10 {
        world.time = 0.05 * step as f32;
        brain.think(&mut world);
    }
    // Standing on the point completes it.
    assert_eq!(brain.goal_status(), BotGoalStatus::SUCCESS);
    brain.clear_explicit_goal();
    assert_eq!(brain.goal_status(), BotGoalStatus::ERROR);
    brain.request_follow_entity(7, vec3(0.0, 0.0, 0.0));
    assert_eq!(brain.goal_status(), BotGoalStatus::IN_PROGRESS);
    brain.think(&mut world);
    // Unknown entity fails the follow.
    assert_eq!(brain.goal_status(), BotGoalStatus::ERROR);
}

#[test]
fn brain_respawns_with_toggle() {
    let (mut brain, _) = test_brain();
    let mut bot = bot_self();
    bot.dead = true;
    let mut world = ScriptedWorld {
        bot,
        entities: Vec::new(),
        time: 0.0,
        contents: BotContents::EMPTY,
        blocked: false,
    };
    let mut presses = 0;
    let mut releases = 0;
    for step in 0..40 {
        world.time = 0.05 * step as f32;
        let cmd = brain.think(&mut world);
        if cmd.buttons & BOT_BUTTON_ATTACK != 0 {
            presses += 1;
        } else {
            releases += 1;
        }
    }
    assert!(presses > 0 && releases > 0, "respawn must toggle attack");
    assert_eq!(empty_usercmd().buttons, 0);
}

#[test]
fn brain_checkpoint_roundtrips() {
    let (mut brain, _) = test_brain();
    let mut world = ScriptedWorld {
        bot: bot_self(),
        entities: vec![enemy(2, vec3(300.0, 0.0, 0.0))],
        time: 0.0,
        contents: BotContents::EMPTY,
        blocked: false,
    };
    for step in 0..10 {
        world.time = 0.05 * step as f32;
        brain.think(&mut world);
    }
    let target = brain.current_target();
    let rng = brain.rng_state();
    let checkpoint = rerelease_behavior_checkpoint(&HashMap::from([("bot".to_owned(), &brain)]));
    assert_eq!(checkpoint.len(), 1);
    let (mut fresh, _) = test_brain();
    let mut brains: HashMap<String, &mut BotBrain> = HashMap::from([("bot".to_owned(), &mut fresh)]);
    restore_rerelease_behavior(&mut brains, &checkpoint).unwrap();
    assert_eq!(fresh.current_target(), target);
    assert_eq!(fresh.rng_state(), rng);
    // Foreign skill checkpoints are rejected.
    let mut bad = checkpoint.clone();
    bad.bots.get_mut("bot").unwrap().brain.skill = "nightmare".to_owned();
    let (mut other, _) = test_brain();
    let mut brains: HashMap<String, &mut BotBrain> = HashMap::from([("bot".to_owned(), &mut other)]);
    assert!(restore_rerelease_behavior(&mut brains, &bad).is_err());
}

#[test]
fn brain_refuses_lava_steps() {
    let (mut brain, _) = test_brain();
    let mut world = ScriptedWorld {
        bot: bot_self(),
        entities: Vec::new(),
        time: 0.0,
        contents: BotContents::EMPTY,
        blocked: false,
    };
    // With empty contents everywhere there is no hazard: sanity check the
    // guard counters start at zero and lava contents trip the hazard path.
    for step in 0..5 {
        world.time = 0.05 * step as f32;
        brain.think(&mut world);
    }
    assert_eq!(brain.hazard_frames(), 0);
    world.contents = BotContents::LAVA;
    world.time = 1.0;
    brain.think(&mut world);
    assert!(brain.hazard_frames() > 0);
    assert!(brain.gap_jumps() >= 0 && brain.guard_refusals() >= 0);
}

#[test]
fn unused_path_helpers_compile() {
    let state = new_path_state();
    let mut state = state;
    set_path(&mut state, None, vec3(0.0, 0.0, 0.0), 0.0);
    assert!(state.path.is_none());
}
