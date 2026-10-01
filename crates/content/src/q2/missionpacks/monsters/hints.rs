//! Rogue hint paths (`src/content/q2/missionpacks/monsters/hints.ts`).
//!
//! Rogue g_newai.c and rerelease rogue/g_rogue_newai.cpp hint paths,
//! plus g_ai pursuit. GPL-2.0-or-later.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{length3, sub3, vec3, Vec3};

use crate::q2::foundation::checkpoint::save_q2_actor;
use crate::q2::foundation::host::{
    Q2Edition, Q2Entity, Q2GameServices, Q2Mode, Q2Solid, Q2SpawnFn, Q2TraceRequest, SpawnModule,
};
use crate::q2::foundation::monsters::ai::{health, vector_angles, visible, MASK_OPAQUE};
use crate::q2::foundation::monsters::perception::{found_target, hunt_target};
use crate::q2::foundation::monsters::types::{MonsterContext, Q2MonsterHintHooks};
use crate::q2::support::contracts::TouchContact;

/// Hint endpoint flag.
const HINT_ENDPOINT: i32 = 1;
/// Maximum hint chains.
const MAX_HINT_CHAINS: usize = 100;
/// Hold forever (`Number(0x7fffffffffffffffn) / 1000`).
const HOLD_FOREVER: f64 = 9.223372036854776e15;

/// Rogue hints state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RogueHintsState {
    /// Paths present.
    pub present: bool,
    /// Chain starts.
    pub starts: Vec<ActorId>,
    /// Nodes by actor.
    pub nodes: HashMap<ActorId, HintNodeState>,
    /// Pursuers by actor.
    pub pursuers: HashMap<ActorId, HintPursuerState>,
}

/// Saved hint node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintNodeState {
    /// Chain index.
    pub chain: i32,
    /// Next node.
    pub next: Option<ActorId>,
}

/// Saved hint pursuer.
#[derive(Debug, Clone, PartialEq)]
pub struct HintPursuerState {
    /// Goal.
    pub goal: Option<ActorId>,
    /// Last time.
    pub last_time: f64,
}

/// Hint path initialization (`Q2HintPathInitialization`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintPathInitialization {
    /// Paths present.
    pub present: bool,
    /// Chain count.
    pub chains: usize,
    /// Issues.
    pub issues: Vec<String>,
}

/// Rogue hints checkpoint (`Q2RogueHintsCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct RogueHintsCheckpoint {
    /// Version.
    pub version: i32,
    /// Paths present.
    pub present: bool,
    /// Chain starts.
    pub starts: Vec<SavedActorId>,
    /// Nodes.
    pub nodes: Vec<SavedHintNode>,
    /// Pursuers.
    pub monsters: Vec<SavedHintPursuer>,
}

/// Saved hint node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedHintNode {
    /// Actor.
    pub actor: SavedActorId,
    /// Chain index.
    pub chain: i32,
    /// Next node.
    pub next: Option<SavedActorId>,
}

/// Saved hint pursuer.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedHintPursuer {
    /// Actor.
    pub actor: SavedActorId,
    /// Goal.
    pub goal: Option<SavedActorId>,
    /// Last time.
    pub last_time: f64,
}

/// Rogue hint hooks (`Q2RogueHints` hooks).
pub struct RogueHints;

impl Q2MonsterHintHooks for RogueHints {
    fn run(&mut self, context: &mut MonsterContext, distance: f64) -> bool {
        run_hints(context, distance)
    }

    fn check_lost(&mut self, context: &mut MonsterContext) -> bool {
        check_lost(context)
    }

    fn stop(&mut self, context: &mut MonsterContext) {
        stop_hints(context);
    }
}

/// Hint path touch (`hint_path_touch`).
pub fn hint_path_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.monsters.states.contains_key(&contact.other) {
        return;
    }
    let mut context = MonsterContext::new(contact.other.clone(), game);
    touch_hints(&actor, &mut context);
}

/// Spawn a hint path (`spawn`).
pub fn spawn_hint_path(actor: ActorId, game: &mut Q2GameServices) -> bool {
    if game.require_entity(&actor).classname != "hint_path" {
        return false;
    }
    if game.options.mode == Q2Mode::Deathmatch {
        game.remove_actor(actor);
        return true;
    }
    let (target, targetname) = {
        let entity = game.require_entity(&actor);
        (entity.target.clone(), entity.targetname.clone())
    };
    if target.is_empty() && targetname.is_empty() {
        let origin = game.body_of(actor.clone()).origin;
        game.host
            .diagnostic(&format!("unlinked hint_path at {} {} {}", origin.x, origin.y, origin.z));
        game.remove_actor(actor);
        return true;
    }
    game.mission_monsters
        .hints
        .nodes
        .insert(actor.clone(), HintNodeState { chain: -1, next: None });
    let entity = game.require_entity_mut(&actor);
    entity.touch = Some(hint_path_touch);
    entity.server_flags |= 1;
    entity.visible = false;
    let mut body = game.body_of(actor.clone());
    body.bounds = qa_core::math::Bounds {
        min: vec3(-8.0, -8.0, -8.0),
        max: vec3(8.0, 8.0, 8.0),
    };
    game.write_body(actor.clone(), &body, false);
    game.set_solid(actor.clone(), Q2Solid::Trigger);
    game.show(actor);
    true
}

/// Hint path spawn module.
pub fn hint_path_module() -> SpawnModule {
    let mut callbacks = crate::q2::foundation::callbacks::Q2CallbackDefinitions::default();
    callbacks.touch.insert("hint_path_touch", hint_path_touch);
    SpawnModule {
        spawn: spawn_hint_path as Q2SpawnFn,
        item_name: |_| None,
        callbacks,
    }
}

/// Source slot of an actor for deterministic ordering.
fn source_slot(game: &mut Q2GameServices, actor: &ActorId) -> u32 {
    game.host
        .actors()
        .source_of(actor)
        .map(|(_, slot)| slot)
        .unwrap_or_else(|| actor.slot())
}

/// Finalize hint paths (`finalize`).
pub fn finalize_hint_paths(game: &mut Q2GameServices) -> HintPathInitialization {
    game.mission_monsters.hints.starts.clear();
    game.mission_monsters.hints.nodes.clear();
    let mut hints: Vec<ActorId> = game
        .entities
        .values()
        .filter(|entity| entity.classname == "hint_path")
        .map(|entity| entity.actor.id().clone())
        .collect();
    hints.sort_by_key(|actor| source_slot(game, actor));
    game.mission_monsters.hints.present = !hints.is_empty();
    let mut issues = Vec::new();
    for hint in &hints {
        game.mission_monsters
            .hints
            .nodes
            .insert(hint.clone(), HintNodeState { chain: -1, next: None });
        let entity = game.require_entity(hint).clone();
        if entity.spawnflags & HINT_ENDPOINT == 0 || entity.target.is_empty() {
            continue;
        }
        if !entity.targetname.is_empty() {
            let origin = game.body_of(hint.clone()).origin;
            let message = format!(
                "Hint path at {} {} {} marked as endpoint with both target ({}) and targetname ({})",
                origin.x, origin.y, origin.z, entity.target, entity.targetname
            );
            game.host.diagnostic(&message);
            issues.push(message);
        } else if game.mission_monsters.hints.starts.len() < MAX_HINT_CHAINS {
            game.mission_monsters.hints.starts.push(hint.clone());
        }
    }
    let starts = game.mission_monsters.hints.starts.clone();
    for (chain, actor) in starts.iter().enumerate() {
        if !game.entities.contains_key(actor) {
            continue;
        }
        game.mission_monsters.hints.nodes.insert(
            actor.clone(),
            HintNodeState {
                chain: chain as i32,
                next: None,
            },
        );
        let mut current = actor.clone();
        let mut visited = HashSet::from([actor.clone()]);
        loop {
            let target = game.require_entity(&current).target.clone();
            if target.is_empty() {
                break;
            }
            let matches = game.targets(&target);
            if matches.len() > 1 {
                let origin = game.body_of(current.clone()).origin;
                let count = game.mission_monsters.hints.starts.len();
                let message = format!(
                    "Forked hint path at {} {} {} detected for chain {count}, target {target}",
                    origin.x, origin.y, origin.z
                );
                game.host.diagnostic(&message);
                issues.push(message);
                if let Some(root) = game.mission_monsters.hints.nodes.get_mut(actor) {
                    root.next = None;
                }
                break;
            }
            let Some(next) = matches.first().cloned() else {
                break;
            };
            let taken = game
                .mission_monsters
                .hints
                .nodes
                .get(&next)
                .is_some_and(|node| node.next.is_some());
            if visited.contains(&next) || taken {
                let origin = game.body_of(next.clone()).origin;
                let targetname = game.require_entity(&next).targetname.clone();
                let count = game.mission_monsters.hints.starts.len();
                let message = format!(
                    "Circular hint path at {} {} {} detected for chain {count}, targetname {targetname}",
                    origin.x, origin.y, origin.z
                );
                game.host.diagnostic(&message);
                issues.push(message);
                if let Some(root) = game.mission_monsters.hints.nodes.get_mut(actor) {
                    root.next = None;
                }
                break;
            }
            if !game.mission_monsters.hints.nodes.contains_key(&current) {
                panic!("Hint chain lost its current node during initialization");
            }
            if let Some(node) = game.mission_monsters.hints.nodes.get_mut(&current) {
                node.next = Some(next.clone());
            }
            game.mission_monsters.hints.nodes.insert(
                next.clone(),
                HintNodeState {
                    chain: chain as i32,
                    next: None,
                },
            );
            visited.insert(next.clone());
            current = next;
        }
    }
    HintPathInitialization {
        present: game.mission_monsters.hints.present,
        chains: game.mission_monsters.hints.starts.len(),
        issues,
    }
}

/// Pursuer state (`source`).
fn pursuer<'a>(game: &'a mut Q2GameServices, actor: &ActorId) -> &'a mut HintPursuerState {
    game.mission_monsters
        .hints
        .pursuers
        .entry(actor.clone())
        .or_insert(HintPursuerState {
            goal: None,
            last_time: 0.0,
        })
}

/// Chain entities from a start (`chain`).
fn chain(game: &mut Q2GameServices, start: &ActorId) -> Vec<ActorId> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut actor = Some(start.clone());
    while let Some(current) = actor {
        if visited.contains(&current) || !game.entities.contains_key(&current) {
            break;
        }
        visited.insert(current.clone());
        result.push(current.clone());
        actor = game
            .mission_monsters
            .hints
            .nodes
            .get(&current)
            .and_then(|node| node.next.clone());
    }
    result
}

/// Find a chain start (`findStart`).
pub fn find_hint_start(entity: &Q2Entity, game: &mut Q2GameServices) -> Option<ActorId> {
    let forward = !entity.target.is_empty();
    let mut visited = HashSet::from([entity.actor.id().clone()]);
    let mut current = entity.clone();
    let mut last: Option<Q2Entity> = None;
    loop {
        let name = if forward {
            current.target.clone()
        } else {
            current.targetname.clone()
        };
        if name.is_empty() {
            break;
        }
        let next = if forward {
            game.targets(&name).first().cloned()
        } else {
            let mut candidates: Vec<ActorId> = game
                .entities
                .values()
                .filter(|candidate| candidate.target == name)
                .map(|candidate| candidate.actor.id().clone())
                .collect();
            candidates.sort_by_key(|actor| source_slot(game, actor));
            candidates.first().cloned()
        };
        let Some(next) = next else { break };
        if visited.contains(&next) {
            return None;
        }
        visited.insert(next.clone());
        let found = game.require_entity(&next).clone();
        last = Some(found.clone());
        current = found;
    }
    last.filter(|entity| entity.spawnflags & HINT_ENDPOINT != 0)
        .map(|entity| entity.actor.id().clone())
}

/// Other chain end (`otherEnd`).
pub fn other_hint_end(entity: &Q2Entity, game: &mut Q2GameServices) -> Option<ActorId> {
    find_hint_start(entity, game)
}

/// Whether a node is visible from an actor (`visibleFrom`).
fn visible_from(game: &mut Q2GameServices, actor: &ActorId, target: &ActorId) -> bool {
    let body = game.host.bodies().read(actor);
    let Some(body) = body else { return false };
    let destination = game.body_of(target.clone()).origin;
    let view_height = game.entities.get(actor).map(|entity| entity.view_height).unwrap_or(22);
    let target_height = game.require_entity(target).view_height;
    game.host
        .trace(&Q2TraceRequest {
            start: Vec3 {
                x: body.origin.x,
                y: body.origin.y,
                z: body.origin.z + view_height as f32,
            },
            end: Vec3 {
                x: destination.x,
                y: destination.y,
                z: destination.z + target_height as f32,
            },
            bounds: None,
            ignore: Some(actor.clone()),
            mask: MASK_OPAQUE,
            exclude: Vec::new(),
        })
        .fraction
        == 1.0
}

/// Check hint paths (`check`).
pub fn check_hints(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let classname = context.entity().classname.clone();
    if !context.game.mission_monsters.hints.present
        || enemy.is_none()
        || context.state().stand_ground
        || classname == "monster_turret"
        || context.game.options.edition == Q2Edition::Rerelease && context.state().pathing.is_some()
    {
        return false;
    }
    let enemy = enemy.expect("hint enemy");
    let foe = context.game.host.bodies().read(&enemy);
    let Some(foe) = foe else { return false };
    let origin = context.game.body_of(actor.clone()).origin;
    let starts = context.game.mission_monsters.hints.starts.clone();
    let mut all = Vec::new();
    for start in &starts {
        all.extend(chain(&mut *context.game, start));
    }
    let mut monster_nodes = Vec::new();
    for node in &all {
        let node_origin = context.game.body_of(node.clone()).origin;
        if length3(sub3(origin, node_origin)) <= 512.0 && visible(context, Some(node)) {
            monster_nodes.push(node.clone());
        }
    }
    let represented: HashSet<Option<i32>> = monster_nodes
        .iter()
        .map(|node| {
            context
                .game
                .mission_monsters
                .hints
                .nodes
                .get(node)
                .map(|node| node.chain)
        })
        .collect();
    let mut target_nodes = Vec::new();
    for node in &all {
        let node_chain = context
            .game
            .mission_monsters
            .hints
            .nodes
            .get(node)
            .map(|node| node.chain);
        let node_origin = context.game.body_of(node.clone()).origin;
        if represented.contains(&node_chain)
            && length3(sub3(foe.origin, node_origin)) <= 512.0
            && visible_from(&mut *context.game, &enemy, node)
        {
            target_nodes.push(node.clone());
        }
    }
    if target_nodes.is_empty() {
        return false;
    }
    let target_chains: HashSet<Option<i32>> = target_nodes
        .iter()
        .map(|node| {
            context
                .game
                .mission_monsters
                .hints
                .nodes
                .get(node)
                .map(|node| node.chain)
        })
        .collect();
    let mut start: Option<ActorId> = None;
    // Native Rogue and rerelease never update closest_range in either
    // loop; retain the native last-eligible source order here.
    for node in &monster_nodes {
        let node_chain = context
            .game
            .mission_monsters
            .hints
            .nodes
            .get(node)
            .map(|node| node.chain);
        if target_chains.contains(&node_chain) {
            start = Some(node.clone());
        }
    }
    let Some(start) = start else { return false };
    let chain_index = context
        .game
        .mission_monsters
        .hints
        .nodes
        .get(&start)
        .map(|node| node.chain);
    let mut destination: Option<ActorId> = None;
    for node in &target_nodes {
        let node_chain = context
            .game
            .mission_monsters
            .hints
            .nodes
            .get(node)
            .map(|node| node.chain);
        let node_origin = context.game.body_of(node.clone()).origin;
        if node_chain == chain_index && length3(sub3(origin, node_origin)) < 10000000.0 {
            destination = Some(node.clone());
        }
    }
    let Some(destination) = destination else { return false };
    pursuer(&mut *context.game, &actor).goal = Some(destination);
    go_hints(context, &start);
    true
}

/// Check lost (`checkLost`).
pub fn check_lost(context: &mut MonsterContext) -> bool {
    let now = context.game.host.now();
    let actor = context.actor().clone();
    if context.state().trail_time + 5.0 > now || pursuer(&mut *context.game, &actor).last_time + 10.0 > now {
        return false;
    }
    pursuer(&mut *context.game, &actor).last_time = now;
    check_hints(context)
}

/// Go to a hint point (`go`).
pub fn go_hints(context: &mut MonsterContext, point: &ActorId) {
    let actor = context.actor().clone();
    let yaw = vector_angles(sub3(
        context.game.body_of(point.clone()).origin,
        context.game.body_of(actor.clone()).origin,
    ))
    .y;
    context.state_mut().ideal_yaw = f64::from(yaw);
    context.entity_mut().goal = Some(point.clone());
    context.state_mut().move_target = Some(point.clone());
    context.state_mut().pause_time = 0.0;
    context.state_mut().hint_path = true;
    context.state_mut().sound_target = None;
    context.state_mut().pursuit_last_seen = false;
    context.state_mut().pursue_next = false;
    context.state_mut().pursue_temporary = false;
    let now = context.game.host.now();
    context.state_mut().search_time = now;
    context.run();
}

/// Stop hint pursuit (`stop`).
pub fn stop_hints(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.entity_mut().goal = None;
    context.state_mut().move_target = None;
    let now = context.game.host.now();
    let state = pursuer(&mut *context.game, &actor);
    state.last_time = now;
    state.goal = None;
    context.state_mut().hint_path = false;
    let enemy = context.entity().enemy.clone();
    if enemy
        .as_ref()
        .is_some_and(|enemy| context.game.host.actors().is_live(enemy))
        && health(&mut *context.game, enemy.as_ref()) >= 1.0
    {
        if visible(context, None) {
            found_target(context);
        } else {
            hunt_target(context);
        }
        return;
    }
    context.entity_mut().enemy = None;
    let now = context.game.host.now();
    context.state_mut().pause_time = if context.game.options.edition == Q2Edition::Classic {
        now + 100000000.0
    } else {
        HOLD_FOREVER
    };
    context.stand();
}

/// Run hint pursuit (`run`).
pub fn run_hints(context: &mut MonsterContext, distance: f64) -> bool {
    if !context.state().hint_path {
        return false;
    }
    context.move_to_goal(distance);
    let actor = context.actor().clone();
    if !context.game.host.actors().is_live(&actor) {
        return true;
    }
    let enemy = context.entity().enemy.clone();
    if enemy
        .as_ref()
        .is_none_or(|enemy| !context.game.host.actors().is_live(enemy))
    {
        context.entity_mut().enemy = None;
        stop_hints(context);
        return true;
    }
    let enemy = enemy.expect("hint enemy");
    let real_enemy = context.game.entities.get(&enemy).and_then(|target| {
        if target.classname == "player_noise" {
            target.owner.clone()
        } else {
            Some(enemy.clone())
        }
    });
    let Some(real_enemy) = real_enemy else {
        context.entity_mut().enemy = None;
        stop_hints(context);
        return true;
    };
    if visible(context, Some(&real_enemy)) {
        stop_hints(context);
    } else if context.game.options.mode == Q2Mode::Coop {
        context.find_target();
    }
    true
}

/// Hint touch (`touch`).
pub fn touch_hints(hint: &ActorId, context: &mut MonsterContext) {
    if context.state().move_target.as_ref() != Some(hint) {
        return;
    }
    let actor = context.actor().clone();
    let goal = pursuer(&mut *context.game, &actor).goal.clone();
    if goal.as_ref() == Some(hint) {
        stop_hints(context);
        return;
    }
    let node = context.game.mission_monsters.hints.nodes.get(hint).cloned();
    let start = node.and_then(|node| {
        context
            .game
            .mission_monsters
            .hints
            .starts
            .get(node.chain as usize)
            .cloned()
    });
    let mut next: Option<ActorId> = None;
    let mut goal_found = false;
    if let Some(start) = start {
        for entry in chain(&mut *context.game, &start) {
            let following = context
                .game
                .mission_monsters
                .hints
                .nodes
                .get(&entry)
                .and_then(|node| node.next.clone());
            if &entry == hint {
                next = following.filter(|following| context.game.entities.contains_key(following));
                break;
            }
            if Some(&entry) == goal.as_ref() {
                goal_found = true;
            }
            if following.as_ref() == Some(hint) && goal_found {
                next = Some(entry);
                break;
            }
        }
    }
    let Some(next) = next else {
        stop_hints(context);
        return;
    };
    go_hints(context, &next);
    // Resume the existing named monster think, preserving source
    // animation ownership.
    let wait = context.game.require_entity(hint).wait;
    let think = context.entity().think;
    if wait != 0.0 {
        if let Some(think) = think {
            context.game.schedule(actor, wait, think);
        }
    }
}

/// Capture hints (`capture`).
pub fn capture_rogue_hints(game: &mut Q2GameServices) -> RogueHintsCheckpoint {
    let mut nodes: Vec<(ActorId, HintNodeState)> = game
        .mission_monsters
        .hints
        .nodes
        .iter()
        .filter(|(actor, _)| game.host.actors().is_live(actor))
        .map(|(actor, node)| (actor.clone(), node.clone()))
        .collect();
    nodes.sort_by(|a, b| (a.0.slot(), a.0.generation()).cmp(&(b.0.slot(), b.0.generation())));
    let mut monsters: Vec<(ActorId, HintPursuerState)> = game
        .mission_monsters
        .hints
        .pursuers
        .iter()
        .filter(|(actor, _)| game.host.actors().is_live(actor))
        .map(|(actor, state)| (actor.clone(), state.clone()))
        .collect();
    monsters.sort_by(|a, b| (a.0.slot(), a.0.generation()).cmp(&(b.0.slot(), b.0.generation())));
    let starts: Vec<SavedActorId> = game
        .mission_monsters
        .hints
        .starts
        .iter()
        .filter_map(|actor| save_q2_actor(Some(actor)))
        .collect();
    RogueHintsCheckpoint {
        version: 1,
        present: game.mission_monsters.hints.present,
        starts,
        nodes: nodes
            .into_iter()
            .filter_map(|(actor, node)| {
                save_q2_actor(Some(&actor)).map(|saved| SavedHintNode {
                    actor: saved,
                    chain: node.chain,
                    next: save_q2_actor(node.next.as_ref()),
                })
            })
            .collect(),
        monsters: monsters
            .into_iter()
            .filter_map(|(actor, state)| {
                save_q2_actor(Some(&actor)).map(|saved| SavedHintPursuer {
                    actor: saved,
                    goal: save_q2_actor(state.goal.as_ref()),
                    last_time: state.last_time,
                })
            })
            .collect(),
    }
}

/// Restore hints (`restore`).
pub fn restore_rogue_hints(game: &mut Q2GameServices, checkpoint: &RogueHintsCheckpoint) {
    game.mission_monsters.hints.present = checkpoint.present;
    game.mission_monsters.hints.starts.clear();
    game.mission_monsters.hints.nodes.clear();
    game.mission_monsters.hints.pursuers.clear();
    for start in &checkpoint.starts {
        let actor = reference(game, start);
        game.mission_monsters.hints.starts.push(actor);
    }
    for node in &checkpoint.nodes {
        let actor = reference(game, &node.actor);
        let next = node.next.as_ref().map(|next| reference(game, next));
        game.mission_monsters.hints.nodes.insert(
            actor,
            HintNodeState {
                chain: node.chain,
                next,
            },
        );
    }
    for monster in &checkpoint.monsters {
        let actor = reference(game, &monster.actor);
        let goal = monster.goal.as_ref().map(|goal| reference(game, goal));
        game.mission_monsters.hints.pursuers.insert(
            actor,
            HintPursuerState {
                goal,
                last_time: monster.last_time,
            },
        );
    }
}

/// Resolve a saved actor, referencing it when no live handle exists.
fn reference(game: &mut Q2GameServices, saved: &SavedActorId) -> ActorId {
    game.host
        .actors()
        .resolve_saved(saved.clone())
        .map(|owned| owned.id().clone())
        .unwrap_or_else(|| game.host.actors().reference_saved(saved.clone()))
}
