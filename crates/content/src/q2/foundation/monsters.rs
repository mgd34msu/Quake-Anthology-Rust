//! Q2 monster runner (`src/content/q2/foundation/monsters/index.ts`).
//!
//! Animation data and callbacks extend without copying the frame or
//! perception runner. The donor's `Q2Monsters` class becomes arena state
//! ([`MonsterRuntime`]) plus free functions; contexts are transient
//! [`MonsterContext`](types::MonsterContext) facades built per dispatch.
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Vec3, add3, dot3, length3, scale3, sub3, vec3};

use self::types::{
    MonsterAction, MonsterAi, MonsterAttackState, MonsterContext, MonsterHandler, NextFrame, PlatformPhase, Q2MonsterDefinition, Q2MonsterHintHooks, Q2MonsterHooks,
    Q2MonsterSourceCombatHooks, SourceCombatMode,
};
use super::host::Q2GameServices;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

pub mod actions;
pub mod ai;
pub mod alternate_fly;
pub mod alternate_fly_state;
pub mod checkpoint;
pub mod frames;
pub mod gibs;
pub mod infantry;
pub mod moves;
pub mod muzzle;
pub mod soldier;
pub mod perception;
pub mod types;

/// External path follower (`Q2PathFollower`).
///
/// The application wires cross-product followers here; the game only
/// consults them for actors without a monster context.
pub trait Q2PathFollower {
    /// Follower actor.
    fn actor(&self) -> &OwnedActor;
    /// Move target.
    fn move_target(&self) -> Option<&ActorId>;
    /// Enemy.
    fn enemy(&self) -> Option<&ActorId>;
    /// Advance toward a goal.
    fn advance(&mut self, name: &str, goal: Option<&ActorId>, pause_until: f64);
}

/// External combat follower (`Q2CombatFollower`).
pub trait Q2CombatFollower {
    /// Move target.
    fn move_target(&self) -> Option<&ActorId>;
    /// Enemy.
    fn enemy(&self) -> Option<&ActorId>;
    /// Old enemy.
    fn old_enemy(&self) -> Option<&ActorId>;
    /// Activator.
    fn activator(&self) -> Option<&ActorId>;
    /// Whether walking.
    fn walking(&self) -> bool;
    /// Advance toward a target.
    fn advance(&mut self, target: &str, goal: Option<&ActorId>, move_target: Option<&ActorId>);
    /// Hold position.
    fn hold(&mut self);
    /// Finish the follow.
    fn finish(&mut self);
}

/// Pending rerelease monster damage.
#[derive(Debug, Clone)]
pub struct PendingMonsterDamage {
    /// Death reaction.
    pub reaction: crate::q2::support::contracts::DeathReaction,
    /// Attack provenance.
    pub attack: Option<crate::q2::support::contracts::AttackProvenance>,
}

/// Monster arena runtime.
pub struct MonsterRuntime {
    /// Definitions by classname.
    pub definitions: HashMap<String, Rc<Q2MonsterDefinition>>,
    /// Edition definitions by edition and classname.
    pub edition_definitions: HashMap<crate::q2::foundation::host::Q2Edition, HashMap<String, Rc<Q2MonsterDefinition>>>,
    /// Monster states by actor.
    pub states: HashMap<ActorId, self::types::MonsterState>,
    /// Resolved definitions by actor.
    pub actor_definitions: HashMap<ActorId, Rc<Q2MonsterDefinition>>,
    /// Perception state.
    pub perception: perception::PerceptionRuntime,
    /// Pending rerelease damage by actor.
    pub pending_damage: HashMap<ActorId, PendingMonsterDamage>,
    /// Whether release cleanup is connected.
    pub connected: bool,
    /// Source combat rules.
    pub source_combat: SourceCombatMode,
    /// Source combat hooks.
    pub source_combat_hooks: Option<Box<dyn Q2MonsterSourceCombatHooks>>,
    /// Hint-path hooks.
    pub hint_hooks: Option<Box<dyn Q2MonsterHintHooks>>,
    /// Constructor hooks.
    pub hooks: Q2MonsterHooks,
    /// Ballistics bundle.
    pub weapons: self::types::MonsterWeapons,
    /// External path follower factory.
    pub external_path_follower: Option<Box<dyn FnMut(&ActorId) -> Option<Box<dyn Q2PathFollower>>>>,
    /// External combat follower factory.
    pub external_combat_follower: Option<Box<dyn FnMut(&ActorId) -> Option<Box<dyn Q2CombatFollower>>>>,
}

impl Default for MonsterRuntime {
    fn default() -> Self {
        Self {
            definitions: HashMap::new(),
            edition_definitions: HashMap::new(),
            states: HashMap::new(),
            actor_definitions: HashMap::new(),
            perception: perception::PerceptionRuntime::new(),
            pending_damage: HashMap::new(),
            connected: false,
            source_combat: SourceCombatMode::default(),
            source_combat_hooks: None,
            hint_hooks: None,
            hooks: Q2MonsterHooks::default(),
            weapons: self::types::MonsterWeapons {
                fire_bullet: crate::q2::foundation::weapons::ballistics::fire_bullet,
                fire_shotgun: crate::q2::foundation::weapons::ballistics::fire_shotgun,
                fire_blaster: crate::q2::foundation::weapons::ballistics::fire_blaster,
                fire_hit: crate::q2::foundation::weapons::ballistics::fire_hit,
                fire_rocket: crate::q2::foundation::weapons::ballistics::fire_rocket,
                fire_grenade: crate::q2::foundation::weapons::ballistics::fire_grenade,
                fire_rail: crate::q2::foundation::weapons::ballistics::fire_rail,
                fire_bfg: crate::q2::foundation::weapons::ballistics::fire_bfg,
            },
            external_path_follower: None,
            external_combat_follower: None,
        }
    }
}

impl MonsterRuntime {
    /// Require a monster state (callback invariant).
    pub fn require_state(&self, actor: &ActorId) -> &self::types::MonsterState {
        self.states
            .get(actor)
            .unwrap_or_else(|| panic!("Missing Q2 source monster context {}", actor.slot()))
    }

    /// Require a monster state, mutably (callback invariant).
    pub fn require_state_mut(&mut self, actor: &ActorId) -> &mut self::types::MonsterState {
        self.states
            .get_mut(actor)
            .unwrap_or_else(|| panic!("Missing Q2 source monster context {}", actor.slot()))
    }

    /// Require a resolved monster definition (callback invariant).
    pub fn require_definition(&self, actor: &ActorId) -> Rc<Q2MonsterDefinition> {
        self.actor_definitions
            .get(actor)
            .cloned()
            .unwrap_or_else(|| panic!("Missing Q2 source monster definition {}", actor.slot()))
    }

    /// Drop monster state after an actor release.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.states.remove(actor);
        self.actor_definitions.remove(actor);
        self.pending_damage.remove(actor);
        self.perception.release(actor);
    }
}

/// Shared monster callbacks (`sharedCallbacks`).
fn shared_duck_down(context: &mut MonsterContext) {
    let duck = context.game.host.now() + 5.0;
    context.state_mut().next_duck_time = duck;
    ai::set_duck(context, true);
}

/// Shared duck hold.
fn shared_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().duck_wait;
    context.state_mut().hold_frame = hold;
}

/// Shared duck up.
fn shared_duck_up(context: &mut MonsterContext) {
    if !context.state().ducked {
        return;
    }
    ai::set_duck(context, false);
    let now = context.game.host.now();
    if context.state().next_duck_time > now {
        let compressed = now + (context.state().next_duck_time - now) * 0.5;
        context.state_mut().next_duck_time = compressed;
    }
}

/// Shared footstep.
fn shared_footstep(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_some() {
        context.game.host_emit(
            crate::q2::foundation::host::Q2PresentationEvent::EntityEvent { actor, event: 8 },
        );
    }
}

/// Look up a shared callback (`sharedCallbacks`).
pub fn shared_callback(name: &str) -> Option<MonsterHandler> {
    match name {
        "monster_done_dodge" => Some(MonsterHandler::Callback(ai::finish_dodge)),
        "monster_duck_down" => Some(MonsterHandler::Callback(shared_duck_down)),
        "monster_duck_hold" => Some(MonsterHandler::Callback(shared_duck_hold)),
        "monster_duck_up" => Some(MonsterHandler::Callback(shared_duck_up)),
        "monster_footstep" => Some(MonsterHandler::Callback(shared_footstep)),
        _ => None,
    }
}

/// Dodge an incoming attack (`dodge`).
pub fn monster_dodge(
    context: &mut MonsterContext,
    attacker: ActorId,
    eta_seconds: f64,
    trace: Option<&crate::q2::support::contracts::TraceResult>,
    gravity: bool,
) {
    let definition = context.definition();
    if let Some(dodge) = definition.dodge {
        dodge(context, &attacker, eta_seconds, trace, gravity);
        return;
    }
    let random = context.game.random();
    if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Classic {
        if random > 0.25 {
            return;
        }
        if context.entity().enemy.is_none() {
            context.entity_mut().enemy = Some(attacker);
        }
        if context.state().kind == "infantry" {
            context.set_move("infantry_move_duck", true);
            return;
        }
        if context.state().kind != "soldier" {
            return;
        }
        if context.game.options.skill == 0 {
            context.set_move("soldier_move_duck", true);
            return;
        }
        let pause = context.game.host.now() + eta_seconds + 0.3;
        context.state_mut().pause_time = pause;
        let threshold = if context.game.options.skill == 1 { 0.33 } else { 0.66 };
        let duck = context.game.random() > threshold;
        context.set_move(if duck { "soldier_move_duck" } else { "soldier_move_attack3" }, true);
        return;
    }
    let actor = context.actor().clone();
    if ai::health(context.game, Some(&actor)) < 1.0 {
        return;
    }
    let definition = context.definition();
    let ducker = definition.duck.is_some() && !gravity;
    let dodger = definition.sidestep.is_some() && !context.state().stand_ground;
    if !ducker && !dodger {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker);
        perception::found_target(context);
    }
    if eta_seconds < context.game.host.frame_seconds()
        || eta_seconds > 2.5
        || random > 0.5
    {
        return;
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let height = f64::from(body.origin.z + body.bounds.max.z)
        - if ducker && trace.is_some() { 32.0 } else { 0.0 };
    if ducker && trace.is_some() && !dodger
        && (trace.is_some_and(|trace| f64::from(trace.end.z) <= height) || context.state().ducked)
    {
        return;
    }
    if dodger {
        if context.state().dodging {
            return;
        }
        if !ducker
            || trace.is_none()
            || trace.is_some_and(|trace| f64::from(trace.end.z) <= height)
            || context.state().ducked
        {
            if context.game.options.skill < 2
                && context.game.random()
                    >= if context.game.options.skill == 0 { 0.25 } else { 0.5 }
            {
                let dodge = context.game.host.now() + 0.8 + context.game.random() * 0.6;
                context.state_mut().dodge_time = dodge;
                return;
            }
            let lefty = match trace {
                None => context.game.random() < 0.5,
                Some(trace) => {
                    f64::from(dot3(
                        ai::angles_vectors(body.angles).right,
                        sub3(trace.end, body.origin),
                    )) >= 0.0
                }
            };
            context.state_mut().lefty = lefty;
            let sidestep = context.definition().sidestep;
            if sidestep.is_some_and(|sidestep| sidestep(context)) {
                if ducker && context.state().ducked {
                    shared_duck_up(context);
                }
                context.state_mut().dodging = true;
                context.state_mut().attack_state = MonsterAttackState::Sliding;
                let dodge = context.game.host.now() + 0.4 + context.game.random() * 1.6;
                context.state_mut().dodge_time = dodge;
            }
            return;
        }
    }
    if ducker && trace.is_some() && eta_seconds < 0.5 {
        if context.state().next_duck_time > context.game.host.now() {
            return;
        }
        ai::finish_dodge(context);
        let duck = context.definition().duck;
        if duck.is_some_and(|duck| duck(context, eta_seconds)) {
            if context.state().duck_wait < context.game.host.now() {
                let wait = context.game.host.now() + eta_seconds;
                context.state_mut().duck_wait = wait;
            }
            shared_duck_down(context);
            if context.game.options.skill == 0 {
                let wait = context.state().duck_wait + 0.5 + context.game.random() * 0.5;
                context.state_mut().duck_wait = wait;
            } else if context.game.options.skill == 1 {
                let wait = context.state().duck_wait + 0.1 + context.game.random() * 0.25;
                context.state_mut().duck_wait = wait;
            }
        }
        let dodge = context.game.host.now() + 0.2 + context.game.random() * 0.5;
        context.state_mut().dodge_time = dodge;
    }
}

/// Default blocked-move handling (`blocked`).
pub fn monster_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let enemy = ai::enemy_body(context);
    let Some(enemy) = enemy else { return false };
    if context.state().kind == "soldier" && (context.state().dodging || context.state().ducked) {
        return false;
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let forward = ai::angles_vectors(body.angles).forward;
    if context.state().kind == "infantry"
        && (context.entity().spawnflags & 8) == 0
        && context.state().jump_time <= context.game.host.now()
    {
        let self_min = f64::from(body.origin.z + body.bounds.min.z);
        let enemy_min = f64::from(enemy.origin.z + enemy.bounds.min.z);
        let ahead = add3(body.origin, scale3(forward, 48.0));
        let down = enemy_min < self_min - 18.0;
        let up = enemy_min > self_min + 18.0;
        if down || up {
            let start = if up {
                vec3(ahead.x, ahead.y, body.origin.z + body.bounds.max.z + 40.0)
            } else {
                ahead
            };
            let end = if down { vec3(ahead.x, ahead.y, self_min as f32 - 193.0) } else { ahead };
            let mask = ai::monster_solid_mask(context.game);
            let clear = !down
                || context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
                    start: body.origin,
                    end: ahead,
                    bounds: Some(body.bounds),
                    ignore: Some(actor.clone()),
                    mask,
                    exclude: Vec::new(),
                })
                .fraction
                    == 1.0;
            if clear {
                let mask = ai::monster_solid_mask(context.game) | ai::MASK_WATER;
                let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
                    start,
                    end,
                    bounds: None,
                    ignore: Some(actor.clone()),
                    mask,
                    exclude: Vec::new(),
                });
                let solid = context
                    .game
                    .host
                    .point_contents(vec3(trace.end.x, trace.end.y, trace.end.z - 1.0));
                let landing = if up {
                    f64::from(trace.end.z) - self_min <= 40.0
                } else {
                    self_min - f64::from(trace.end.z) >= 24.0
                        && enemy_min - f64::from(trace.end.z) <= 32.0
                        && matches!(
                            &trace.contact,
                            crate::q2::support::contracts::TraceContact::Plane { plane }
                                if plane.normal.z >= 0.9
                        )
                };
                if trace.fraction < 1.0
                    && !trace.all_solid
                    && !trace.start_solid
                    && (solid & (3 | 32)) != 0
                    && landing
                {
                    if (solid & 32) != 0 {
                        let mask = ai::monster_solid_mask(context.game);
                        let deep = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
                            start: trace.end,
                            end,
                            bounds: None,
                            ignore: Some(actor.clone()),
                            mask,
                            exclude: Vec::new(),
                        });
                        if (context.game.host.point_contents(vec3(
                            deep.end.x,
                            deep.end.y,
                            deep.end.z + 48.0,
                        )) & ai::MASK_WATER)
                            != 0
                        {
                            return false;
                        }
                    }
                    ai::finish_dodge(context);
                    let jump = context.game.host.now() + 3.0;
                    context.state_mut().jump_time = jump;
                    context.set_move(if up { "infantry_move_jump2" } else { "infantry_move_jump" }, true);
                    return true;
                }
            }
        }
    }
    let above = f64::from(enemy.origin.z + enemy.bounds.min.z)
        >= f64::from(body.origin.z + body.bounds.max.z);
    let below = f64::from(enemy.origin.z + enemy.bounds.max.z)
        <= f64::from(body.origin.z + body.bounds.min.z);
    if !above && !below {
        return false;
    }
    let mut platform = body.ground.as_ref().and_then(|ground| context.game.entity(ground).cloned());
    if platform.as_ref().is_none_or(|platform| !platform.classname.starts_with("func_plat")) {
        let start = add3(body.origin, scale3(forward, distance as f32));
        let mask = ai::monster_solid_mask(context.game);
        let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start,
            end: vec3(start.x, start.y, start.z - 384.0),
            bounds: None,
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        platform = match &trace.hit {
            crate::q2::support::contracts::TraceHit::Actor { actor } => {
                context.game.entity(actor).cloned()
            }
            _ => None,
        };
    }
    let Some(platform) = platform else { return false };
    if !platform.classname.starts_with("func_plat") || platform.use_.is_none() {
        return false;
    }
    let endpoint = context.platform_state(platform.actor.id());
    let on_platform = body.ground == Some(platform.actor.id().clone());
    let call = if above {
        on_platform && endpoint == Some(PlatformPhase::Bottom)
            || !on_platform && endpoint == Some(PlatformPhase::Top)
    } else {
        on_platform && endpoint == Some(PlatformPhase::Top)
            || !on_platform && endpoint == Some(PlatformPhase::Bottom)
    };
    if call {
        let platform_actor = platform.actor.id().clone();
        context.game.dispatch_use(platform_actor, Some(actor.clone()), Some(actor));
        return true;
    }
    false
}

/// Monster think (`sourceThink`).
pub fn source_think(actor: ActorId, game: &mut Q2GameServices) {
    let mut context = MonsterContext::new(actor, game);
    monster_think(&mut context);
}

/// Monster dead think (`sourceDeadThink`).
pub fn source_dead_think(actor: ActorId, game: &mut Q2GameServices) {
    let mut context = MonsterContext::new(actor, game);
    ai::monster_dead_think(&mut context);
}

/// Monster flies on (`sourceFliesOn`).
pub fn source_flies_on(actor: ActorId, game: &mut Q2GameServices) {
    let mut context = MonsterContext::new(actor, game);
    ai::flies_on(&mut context);
}

/// Monster flies off (`sourceFliesOff`).
pub fn source_flies_off(actor: ActorId, game: &mut Q2GameServices) {
    let mut context = MonsterContext::new(actor, game);
    ai::flies_off(&mut context);
}

/// Monster start (`sourceStart`).
pub fn source_start(actor: ActorId, game: &mut Q2GameServices) {
    let mut context = MonsterContext::new(actor, game);
    start_monster(&mut context);
}

/// Monster triggered spawn (`sourceTriggerSpawn`).
pub fn source_trigger_spawn(actor: ActorId, game: &mut Q2GameServices) {
    let mut context = MonsterContext::new(actor, game);
    trigger_spawn(&mut context);
}

/// Monster use (`sourceUse`).
pub fn source_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    {
        let context = MonsterContext::new(actor.clone(), &mut *game);
        let mut mission = context.mission(&actor);
        if mission.as_mut().is_some_and(|mission| mission.r#use(activator.as_ref())) {
            return;
        }
    }
    let mut context = MonsterContext::new(actor.clone(), game);
    if context.entity().enemy.is_some() || context.state().dead || activator.is_none() {
        return;
    }
    let activator = activator.expect("use activator");
    if context.game.monster_target(Some(&activator)).is_some_and(|observed| observed.notarget) {
        return;
    }
    if !context.game.host.is_player(&activator)
        && !context
            .game
            .monsters
            .states
            .get(&activator)
            .is_some_and(|state| state.good_guy)
    {
        return;
    }
    context.entity_mut().enemy = Some(activator);
    perception::found_target(&mut context);
}

/// Monster trigger use (`sourceTriggerUse`).
pub fn source_trigger_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    let delay = if game.options.edition == crate::q2::foundation::host::Q2Edition::Classic {
        0.1
    } else {
        game.host.frame_seconds()
    };
    let think = game.source_callbacks.resolve_think(Some("monster_triggered_spawn"));
    let think = think.expect("monster triggered spawn thinker");
    game.schedule(actor.clone(), delay, think);
    if activator.as_ref().is_some_and(|activator| game.host.is_player(activator)) {
        game.require_entity_mut(&actor).enemy = activator;
    }
    game.require_entity_mut(&actor).use_ = Some(source_use as crate::q2::foundation::host::Q2Use);
}

/// Monster pain (`sourcePain`).
pub fn source_pain(actor: ActorId, game: &mut Q2GameServices, reaction: PainReaction) {
    let mut context = MonsterContext::new(actor.clone(), game);
    perception::react_to_damage(&mut context, reaction.attacker.as_ref());
    if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease {
        let inflictor = reaction.attack.as_ref().and_then(|attack| attack.inflictor.clone());
        let point = context.game.body_of(actor).origin;
        let hit = PainHit { inflictor, point };
        queue_pain(&mut context, reaction, &hit);
    } else if let Some(definition) = context.try_definition() {
        if let Some(pain) = definition.pain {
            pain(&mut context, &reaction);
        }
    }
    set_skin(&mut context);
    let actor = context.actor().clone();
    context.game.show(actor);
}

/// Monster death (`sourceDie`).
pub fn source_die(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    if game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease {
        if ai::health(game, Some(&actor)) < -999.0 {
            let owned = game.owned_of(actor.clone());
            game.host.combat().set_health(&owned, -999.0);
        }
        {
            let entity = game.require_entity_mut(&actor);
            entity.enemy = reaction.pain.attacker.clone();
            entity.flags |= 1 << 19;
        }
        let mut context = MonsterContext::new(actor, game);
        let hit =
            PainHit { inflictor: reaction.inflictor.clone(), point: reaction.point };
        queue_pain(&mut context, reaction.pain.clone(), &hit);
        return;
    }
    let mut context = MonsterContext::new(actor, game);
    let definition = context.definition();
    monster_die(&mut context, definition, reaction);
}

/// Queued pain hit (`queuePain` hit).
#[derive(Debug, Clone)]
struct PainHit {
    /// Inflictor.
    inflictor: Option<ActorId>,
    /// Impact point.
    point: Vec3,
}

/// Run monster think (`think`).
///
/// Think identity compares function addresses within one build; the
/// donor compares callback identity the same way.
#[allow(unpredictable_function_pointer_comparisons)]
fn monster_think(context: &mut MonsterContext) {
    if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease {
        context.entity_mut().render_flags &= !((1 << 22) | (1 << 26));
        context.entity_mut().old_frame = -1;
        let prior_think = context.entity().think;
        let actor = context.actor().clone();
        process_pain(&actor, &mut *context.game);
        if !context.game.host.actors().is_live(&actor) || context.entity().think != prior_think {
            return;
        }
        check_dodge(&mut *context);
    }
    let delay = if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Classic {
        0.1
    } else {
        context.game.host.frame_seconds()
    };
    let think = context.game.source_callbacks.resolve_think(Some("monster_think"));
    let think = think.expect("monster thinker");
    let actor = context.actor().clone();
    context.game.schedule(actor.clone(), delay, think);
    let mut mission = context.mission(&actor);
    if let Some(mission) = mission.as_mut() {
        if context.entity().enemy.is_some() {
            let route = mission.combat_route();
            if let Some(goal) = route.goal {
                context.state_mut().combat_point = true;
                context.state_mut().move_target = Some(goal.clone());
                context.entity_mut().goal = Some(goal);
            } else if context.state().combat_point {
                context.state_mut().combat_point = false;
                context.state_mut().move_target = None;
                let enemy = context.entity().enemy.clone();
                context.entity_mut().goal = enemy;
            }
            if route.stand_ground {
                context.state_mut().stand_ground = true;
            }
        }
    }
    move_frame(context);
    let actor = context.actor().clone();
    if !context.game.host.actors().is_live(&actor) || context.state().gibbed {
        return;
    }
    let link_count =
        context.game.host.bodies().linked(&actor).map(|body| body.link_count).unwrap_or(0);
    if link_count as i32 != context.state().last_link_count {
        context.state_mut().last_link_count = link_count as i32;
        check_ground(context);
    }
    categorize_position(context);
    world_effects(context);
    if !context.game.host.actors().is_live(&actor) {
        return;
    }
    context.entity_mut().effects &= !256;
    context.entity_mut().render_flags &= !(1024 | 2048 | 4096);
    if context.state().resurrecting {
        context.entity_mut().effects |= 256;
        context.entity_mut().render_flags |= 1024;
    }
    context.game.show(actor);
}

/// Advance the animation frame (`moveFrame`).
///
/// `M_MoveFrame` captures a frame before AI, preserving its callback
/// when AI changes the move.
fn move_frame(context: &mut MonsterContext) {
    let rerelease = context.game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease;
    let mut movement = context.state().current_move.clone();
    let mut run_frame = !rerelease || context.state().next_move_time <= context.game.host.now();
    let next_is_current = context
        .state()
        .next_move
        .as_ref()
        .is_some_and(|next| *next == context.state().current_move);
    if run_frame && context.state().next_move.is_some() && !next_is_current {
        let next = context.state().next_move.clone().expect("queued move");
        context.state_mut().current_move = next.clone();
        context.state_mut().next_move = None;
        movement = next;
    }
    if !run_frame {
        let frame = context.entity().frame;
        run_frame = frame < movement.first_frame || frame > movement.last_frame;
    }
    if run_frame {
        let mut explicit = false;
        let next_frame = context.state().next_frame;
        if next_frame != 0 && next_frame >= movement.first_frame && next_frame <= movement.last_frame {
            context.entity_mut().frame = next_frame;
            context.state_mut().next_frame = 0;
        } else {
            if context.entity().frame == movement.last_frame && movement.end.is_some() {
                let end = movement.end.clone().expect("move end");
                context.dispatch(&end);
                if rerelease && context.state().next_move.is_some() {
                    let next = context.state().next_move.clone().expect("queued move");
                    context.state_mut().current_move = next;
                    context.state_mut().next_move = None;
                    if context.state().next_frame != 0 {
                        let next_frame = context.state().next_frame;
                        context.entity_mut().frame = next_frame;
                        context.state_mut().next_frame = 0;
                        explicit = true;
                    }
                }
                movement = context.state().current_move.clone();
                let corpse = context.state().corpse
                    || (context.entity().server_flags & 2) != 0;
                let actor = context.actor().clone();
                if corpse || !context.game.host.actors().is_live(&actor) {
                    return;
                }
            }
            if context.entity().frame < movement.first_frame
                || context.entity().frame > movement.last_frame
            {
                context.state_mut().hold_frame = false;
                context.entity_mut().frame = movement.first_frame;
            } else if !explicit && !context.state().hold_frame {
                let frame = context.entity().frame + 1;
                context.entity_mut().frame = frame;
                if context.entity().frame > movement.last_frame {
                    context.entity_mut().frame = movement.first_frame;
                }
            }
        }
        let next_move_time = crate::q2::foundation::entity_services::js_round(
            (context.game.host.now() + 0.1) * 1000.0,
        ) / 1000.0;
        context.state_mut().next_move_time = next_move_time;
        let next_frame = context.state().next_frame;
        if rerelease
            && next_frame != 0
            && (next_frame < movement.first_frame || next_frame > movement.last_frame)
        {
            context.state_mut().next_frame = 0;
        }
    }
    let frame_index = (context.entity().frame - movement.first_frame) as usize;
    let frame = types::record_at(&movement.frames, frame_index).clone();
    let distance = if context.state().hold_frame {
        0.0
    } else {
        frame.distance
            * context.state().scale
            * if rerelease { context.game.host.frame_seconds() * 10.0 } else { 1.0 }
    };
    match &frame.ai {
        MonsterAi::Source(name) => {
            let handler = context.definition().ai.get(name).cloned();
            let Some(handler) = handler else {
                panic!("Missing Q2 source AI {name}");
            };
            handler(context, distance);
        }
        ai => ai::run_ai(context, ai, distance),
    }
    let actor = context.actor().clone();
    if !context.game.host.actors().is_live(&actor) {
        return;
    }
    if run_frame {
        for action in frame.actions.clone() {
            match action {
                MonsterAction::Name(name) => context.dispatch(&name),
                MonsterAction::NextFrame(next) => {
                    let next_frame = match next {
                        NextFrame::Next => context.entity().frame + 1,
                        NextFrame::Frame(number) => number,
                    };
                    context.state_mut().next_frame = next_frame;
                }
            }
            if !context.game.host.actors().is_live(&actor) {
                return;
            }
        }
    }
    if rerelease && frame.lerp_frame != -1 {
        context.entity_mut().render_flags |= 1 << 22;
        context.entity_mut().old_frame = frame.lerp_frame;
    }
}

/// Process a death (`die`).
fn monster_die(
    context: &mut MonsterContext,
    definition: std::rc::Rc<Q2MonsterDefinition>,
    reaction: DeathReaction,
) {
    let mut hooks = context.game.monsters.source_combat_hooks.take();
    if let Some(hooks) = hooks.as_mut() {
        hooks.before_killed(context);
    }
    context.game.monsters.source_combat_hooks = hooks;
    if !context.state().dead {
        let commander = context.state().commander.clone();
        let commander_entity =
            commander.as_ref().and_then(|commander| context.game.entity(commander).cloned());
        if let Some(commander_entity) = commander_entity {
            let commander_id = commander_entity.actor.id().clone();
            let spawned_by = context.state().spawned_by;
            let monster_slots = context.state().monster_slots;
            if let Some(commander_state) = context.game.monsters.states.get_mut(&commander_id) {
                if spawned_by == types::MonsterSpawner::Carrier
                    && commander_entity.classname == "monster_carrier"
                {
                    commander_state.monster_slots += 1;
                } else if spawned_by == types::MonsterSpawner::Medic
                    && commander_entity.classname == "monster_medic_commander"
                {
                    if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease
                    {
                        commander_state.monster_used -= monster_slots;
                    } else {
                        commander_state.monster_slots += 1;
                    }
                } else if spawned_by == types::MonsterSpawner::Widow
                    && commander_entity.classname.starts_with("monster_widow")
                    && commander_state.monster_used > 0
                {
                    commander_state.monster_used -= 1;
                }
            }
        }
        let actor = context.actor().clone();
        let mut mission = context.mission(&actor);
        let mission_present = mission.is_some();
        if !mission_present
            && !context.state().good_guy
            && !context.state().do_not_count
            && (context.game.options.edition == crate::q2::foundation::host::Q2Edition::Classic
                || (context.entity().spawnflags & 65536) == 0)
        {
            context.game.counters.killed_monsters += 1;
        }
        context.entity_mut().enemy = reaction.pain.attacker.clone();
        let motion = context.entity().motion;
        if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Classic
            && matches!(
                motion,
                crate::q2::foundation::host::Q2MotionKind::Push
                    | crate::q2::foundation::host::Q2MotionKind::Stop
                    | crate::q2::foundation::host::Q2MotionKind::Stationary
            )
        {
            (definition.die)(context, &reaction);
            let actor = context.actor().clone();
            context.game.show(actor);
            return;
        }
        context.entity_mut().touch = None;
        context.entity_mut().flags &= !3;
        let item = context.entity().spawn.values.get("item").cloned();
        if item.as_ref().is_some_and(|item| !item.is_empty()) {
            let drop = context.game.monsters.hooks.drop_item;
            let Some(drop) = drop else {
                panic!("Monster authored item drop requires source item services");
            };
            let actor = context.actor().clone();
            let owned = context.game.owned_of(actor);
            let game = &mut *context.game;
            drop(owned, game, item.as_deref().unwrap_or(""));
        }
        if !mission_present {
            let death_target = context.entity().death_target.clone();
            if !death_target.is_empty() {
                context.entity_mut().target = death_target;
            }
            if !context.entity().target.is_empty() {
                let attacker = reaction.pain.attacker.clone();
                let authored = context.game.require_entity(context.actor()).authored_target();
                context.game.use_targets(&authored, attacker.as_ref(), false);
            }
        } else if let Some(mission) = mission.as_mut() {
            mission.killed(reaction.pain.attacker.as_ref());
        }
    }
    (definition.die)(context, &reaction);
    let actor = context.actor().clone();
    context.game.show(actor);
}

/// Queue damage for end-of-frame processing (`queuePain`).
fn queue_pain(context: &mut MonsterContext, reaction: PainReaction, hit: &PainHit) {
    let previous = context.game.monsters.pending_damage.get(context.actor()).cloned();
    let pending = PendingMonsterDamage {
        reaction: DeathReaction {
            pain: PainReaction {
                damage: reaction.damage + previous.as_ref().map(|pending| pending.reaction.pain.damage).unwrap_or(0.0),
                kick: reaction.kick + previous.as_ref().map(|pending| pending.reaction.pain.kick).unwrap_or(0.0),
                ..reaction.clone()
            },
            inflictor: hit.inflictor.clone(),
            point: hit.point,
        },
        attack: reaction.attack.clone(),
    };
    context.game.monsters.pending_damage.insert(context.actor().clone(), pending);
}

/// Update the skin from health (`setSkin`).
fn set_skin(context: &mut MonsterContext) {
    if context.state().gibbed
        || context.game.options.edition != crate::q2::foundation::host::Q2Edition::Rerelease
    {
        return;
    }
    let actor = context.actor().clone();
    let wounded =
        ai::health(context.game, Some(&actor)) < context.entity().max_health / 2.0;
    if context.state().kind == "soldier" {
        let skin = (context.entity().skin & !1) | if wounded { 1 } else { 0 };
        context.entity_mut().skin = skin;
    } else if context.state().kind == "infantry" {
        context.entity_mut().skin = if wounded { 1 } else { 0 };
    }
}

/// Admit a monster (`admit`).
fn admit_monster(
    game: &mut Q2GameServices,
    actor: ActorId,
    definition: Rc<Q2MonsterDefinition>,
    reviving: bool,
    summoned: bool,
) -> bool {
    use crate::q2::foundation::host::{Q2Edition, Q2MotionKind, Q2Solid};
    if game.options.mode == crate::q2::foundation::host::Q2Mode::Deathmatch {
        game.remove_actor(actor);
        return true;
    }
    connect_monsters(game);
    let initial = definition
        .moves
        .iter()
        .find(|movement| movement.name == definition.initial_move)
        .cloned()
        .unwrap_or_else(|| panic!("Missing Q2 monster initial move {}", definition.initial_move));
    let previous = if reviving { game.monsters.states.get(&actor).cloned() } else { None };
    let classname = game.require_entity(&actor).classname.clone();
    let locomotion = definition.locomotion.unwrap_or(types::MonsterLocomotion::Walk);
    let yaw_speed = definition.yaw_speed.unwrap_or(if locomotion == types::MonsterLocomotion::Walk {
        20.0
    } else {
        10.0
    });
    let ideal_yaw = f64::from(game.body_of(actor.clone()).angles.y);
    let combat_target = game.require_entity(&actor).combat_target.clone();
    let state = types::MonsterState {
        kind: definition.kind.clone(),
        weapon: types::MonsterWeapon::from_classname(&classname),
        locomotion,
        has_melee: definition.melee.is_some(),
        has_ranged_attack: definition.has_ranged_attack,
        has_idle: definition.idle.is_some(),
        has_search: definition.search.is_some(),
        blind_fire: definition.blind_fire,
        ignore_shots: previous.as_ref().is_some_and(|previous| previous.ignore_shots),
        do_not_count: summoned || previous.as_ref().is_some_and(|previous| previous.do_not_count),
        spawned_by: previous.as_ref().map(|previous| previous.spawned_by).unwrap_or_default(),
        commander: previous.as_ref().and_then(|previous| previous.commander.clone()),
        monster_slots: previous.as_ref().map(|previous| previous.monster_slots).unwrap_or(0),
        monster_used: previous.as_ref().map(|previous| previous.monster_used).unwrap_or(0),
        current_move: initial,
        next_frame: 0,
        next_move_time: 0.0,
        scale: definition.scale,
        gib_health: definition.gib_health,
        can_take_damage: true,
        ideal_yaw,
        yaw_speed,
        combat_target,
        cocked: definition.kind == "soldier",
        normal_height: f64::from(definition.bounds.max.z),
        air_finished: game.host.now() + 12.0,
        ..types::MonsterState::default()
    };
    game.monsters.states.insert(actor.clone(), state);
    game.monsters.actor_definitions.insert(actor.clone(), definition.clone());
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let multiplier = if rerelease {
        crate::q2::foundation::fields::number_field(
            &game.require_entity(&actor).spawn,
            "health_multiplier",
            1.0,
        )
    } else {
        1.0
    };
    let solid_mask = ai::monster_solid_mask(game);
    {
        let entity = game.require_entity_mut(&actor);
        entity.model = definition.model.clone();
        entity.max_health = definition.health * multiplier;
        entity.view_height = definition.view_height.unwrap_or(if rerelease {
            0
        } else if locomotion == types::MonsterLocomotion::Swim {
            10
        } else {
            25
        });
        entity.server_flags = (entity.server_flags | 4) & !2;
        entity.flags |= if locomotion == types::MonsterLocomotion::Fly {
            1
        } else if locomotion == types::MonsterLocomotion::Swim {
            2
        } else {
            0
        };
        entity.clip_mask = solid_mask;
        entity.skin = if definition.kind == "soldier" {
            match types::MonsterWeapon::from_classname(&classname) {
                types::MonsterWeapon::Blaster => 0,
                types::MonsterWeapon::Shotgun => 2,
                types::MonsterWeapon::Machinegun => 4,
            }
        } else {
            0
        };
        entity.count = entity.skin;
        entity.scale = if rerelease {
            crate::q2::foundation::fields::number_field(&entity.spawn, "scale", 1.0)
        } else {
            1.0
        };
        if rerelease {
            entity.render_flags |= 32768;
        } else {
            entity.render_flags |= 64;
        }
    }
    let entity_scale = game.require_entity(&actor).scale;
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.scale *= entity_scale;
    }
    let mut moved = game.body_of(actor.clone());
    moved.bounds = qa_core::math::Bounds {
        min: scale3(definition.bounds.min, entity_scale as f32),
        max: scale3(definition.bounds.max, entity_scale as f32),
    };
    game.write_body(actor.clone(), &moved, false);
    let normal_height = f64::from(game.body_of(actor.clone()).bounds.max.z);
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.normal_height = normal_height;
    }
    if rerelease && game.require_entity(&actor).view_height == 0 {
        let height = (normal_height - 8.0).trunc() as i32;
        game.require_entity_mut(&actor).view_height = height;
    }
    if game.host.combat().read(&actor).is_none() {
        let owned = game.owned_of(actor.clone());
        game.create_combat(&owned, game.require_entity(&actor).max_health, definition.mass * entity_scale, true);
    } else {
        let owned = game.owned_of(actor.clone());
        let max_health = game.require_entity(&actor).max_health;
        game.host.combat().set_health(&owned, max_health);
        game.host.combat().set_armor(&owned, &crate::contract::ArmorState {
            regular: crate::contract::RegularArmorState::None,
            powered: crate::contract::PoweredProtectionState::None,
        });
        game.set_combat_traits(&owned, &crate::q2::support::contracts::CombatTraitChanges {
            mass: Some(definition.mass * entity_scale),
            can_take_damage: Some(true),
            invulnerable: Some(false),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        });
    }
    {
        let entity = game.require_entity_mut(&actor);
        entity.pain = Some(source_pain as crate::q2::foundation::host::Q2Pain);
        entity.die = Some(source_die as crate::q2::foundation::host::Q2Die);
        entity.use_ = Some(source_use as crate::q2::foundation::host::Q2Use);
    }
    if let Some(initialize) = definition.initialize.clone() {
        let mut context = MonsterContext::new(actor.clone(), &mut *game);
        initialize.dispatch(&mut context);
    }
    if !game.host.actors().is_live(&actor) {
        return true;
    }
    let initial_combat =
        game.host.combat().read(&actor).expect("initialized monster combat state");
    match &initial_combat.armor.powered {
        crate::contract::PoweredProtectionState::None => {
            let power = game.host.inventory().count(&actor, &"q2:monster-power".to_string());
            if let Some(state) = game.monsters.states.get_mut(&actor) {
                state.max_power_armor_power = power;
            }
        }
        crate::contract::PoweredProtectionState::Screen { cells } => {
            if let Some(state) = game.monsters.states.get_mut(&actor) {
                state.initial_power_armor = types::MonsterPowerArmor::Screen;
                state.max_power_armor_power = *cells;
            }
        }
        crate::contract::PoweredProtectionState::Shield { cells } => {
            if let Some(state) = game.monsters.states.get_mut(&actor) {
                state.initial_power_armor = types::MonsterPowerArmor::Shield;
                state.max_power_armor_power = *cells;
            }
        }
    }
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.base_health = initial_combat.health;
    }
    if rerelease && (game.require_entity(&actor).spawnflags & 524288) != 0 {
        if let Some(state) = game.monsters.states.get_mut(&actor) {
            state.good_guy = true;
        }
    }
    let good_guy = game.monsters.states.get(&actor).is_some_and(|state| state.good_guy);
    if !good_guy && (game.require_entity(&actor).spawnflags & 4) != 0 {
        let entity = game.require_entity_mut(&actor);
        entity.spawnflags = (entity.spawnflags & !4) | 1;
    }
    let count = (!reviving || game.options.edition == Q2Edition::Classic)
        && !game.monsters.states.get(&actor).is_some_and(|state| state.good_guy)
        && !game.monsters.states.get(&actor).is_some_and(|state| state.do_not_count)
        && (game.options.edition == Q2Edition::Classic
            || (game.require_entity(&actor).spawnflags & 65536) == 0);
    if count {
        let mut mission = game.monsters.hooks.mission.as_ref().and_then(|hook| hook(&actor));
        if let Some(mission) = mission.as_mut() {
            mission.spawned();
        } else {
            game.counters.total_monsters += 1;
        }
    }
    let locomotion =
        game.monsters.states.get(&actor).map(|state| state.locomotion).unwrap_or(types::MonsterLocomotion::Walk);
    game.set_solid(actor.clone(), Q2Solid::Box);
    game.set_motion_kind(
        actor.clone(),
        if locomotion == types::MonsterLocomotion::Stationary {
            Q2MotionKind::Stationary
        } else {
            Q2MotionKind::Step
        },
    );
    let mut context = MonsterContext::new(actor.clone(), game);
    let automatic =
        definition.start_mode.map(|start_mode| start_mode(&mut context)) != Some(types::StartMode::Manual);
    if automatic {
        let (first, last) = {
            let movement = &context.state().current_move;
            (movement.first_frame, movement.last_frame)
        };
        let frame =
            first + (context.game.random() * (last - first + 1) as f64).floor() as i32;
        context.entity_mut().frame = frame;
    }
    let actor = context.actor().clone();
    context.game.show(actor.clone());
    if automatic {
        let delay = if context.game.options.edition == Q2Edition::Classic {
            0.1
        } else {
            context.game.host.frame_seconds()
        };
        let think = context.game.source_callbacks.resolve_think(Some("monster_start_go"));
        let think = think.expect("monster start thinker");
        context.game.schedule(actor, delay, think);
    }
    if let Some(after_spawn) = definition.after_spawn.clone() {
        after_spawn.dispatch(&mut context);
    }
    true
}

/// Start a monster (`start`).
fn start_monster(context: &mut MonsterContext) {
    use crate::q2::foundation::host::{Q2Edition, Q2MotionKind, Q2Solid};
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let spawn_dead = rerelease && (context.entity().spawnflags & 65536) != 0;
    if (context.entity().spawnflags & 2) == 0
        && context.state().locomotion == types::MonsterLocomotion::Walk
        && context.game.host.now() < 1.0
        && (context.game.options.edition == Q2Edition::Classic
            || (context.entity().spawnflags & 262144) == 0)
    {
        drop_to_floor(context);
    }
    let actor = context.actor().clone();
    if ai::health(context.game, Some(&actor)) <= 0.0 {
        return;
    }
    let actor = context.actor().clone();
    let mut mission = context.mission(&actor);
    if let Some(mission) = mission.as_mut() {
        let goal = mission.route();
        context.state_mut().move_target = goal.clone();
        context.entity_mut().goal = goal.clone();
        let target = goal.as_ref().and_then(|goal| context.game.host.bodies().read(goal));
        if target.is_none() {
            let pause = context.game.host.now() + 100000000.0;
            context.state_mut().pause_time = pause;
            if !spawn_dead {
                context.stand();
            }
        } else {
            let target = target.expect("mission goal body");
            let actor = context.actor().clone();
            let origin = context.game.body_of(actor.clone()).origin;
            let yaw = f64::from(ai::vector_angles(sub3(target.origin, origin)).y);
            context.state_mut().ideal_yaw = yaw;
            let mut moved = context.game.body_of(actor.clone());
            moved.angles.y = yaw as f32;
            context.game.write_body(actor, &moved, false);
            if !spawn_dead {
                context.walk();
            }
        }
    } else {
        if !context.entity().target.is_empty() {
            let target_name = context.entity().target.clone();
            let targets = context.game.targets(&target_name);
            let combat = targets.iter().any(|target| {
                context.game.entity(target).is_some_and(|target| target.classname == "point_combat")
            });
            if combat {
                let target_name = context.entity().target.clone();
                context.state_mut().combat_target = target_name.clone();
                context.entity_mut().combat_target = target_name;
                context.entity_mut().target = String::new();
            }
        }
        if !context.entity().target.is_empty() {
            let target_name = context.entity().target.clone();
            let target = context.game.pick_target(&target_name);
            let goal = target.as_ref().and_then(|target| {
                context.game.entity(target).map(|entity| entity.actor.id().clone())
            });
            context.entity_mut().goal = goal.clone();
            context.state_mut().move_target = goal;
            match target {
                None => {
                    let classname = context.entity().classname.clone();
                    let target_name = context.entity().target.clone();
                    context.game.host.diagnostic(&format!(
                        "{classname}: target {target_name} not found"
                    ));
                    context.entity_mut().target = String::new();
                    let pause = context.game.host.now() + 100000000.0;
                    context.state_mut().pause_time = pause;
                    if !spawn_dead {
                        context.stand();
                    }
                }
                Some(target) => {
                    let classname =
                        context.game.entity(&target).map(|entity| entity.classname.clone());
                    if classname.as_deref() == Some("path_corner") {
                        let goal_origin = context.game.body_of(target).origin;
                        let actor = context.actor().clone();
                        let origin = context.game.body_of(actor.clone()).origin;
                        let yaw = f64::from(ai::vector_angles(sub3(goal_origin, origin)).y);
                        context.state_mut().ideal_yaw = yaw;
                        let mut moved = context.game.body_of(actor.clone());
                        moved.angles.y = yaw as f32;
                        context.game.write_body(actor, &moved, false);
                        if !spawn_dead {
                            context.walk();
                        }
                        context.entity_mut().target = String::new();
                    } else {
                        context.entity_mut().goal = None;
                        context.state_mut().move_target = None;
                        let pause = context.game.host.now() + 100000000.0;
                        context.state_mut().pause_time = pause;
                        if !spawn_dead {
                            context.stand();
                        }
                    }
                }
            }
        } else {
            let pause = context.game.host.now() + 100000000.0;
            context.state_mut().pause_time = pause;
            if !spawn_dead {
                context.stand();
            }
        }
    }
    if spawn_dead {
        let definition = context.definition();
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor.clone()).origin;
        let owned = context.game.owned_of(actor.clone());
        context.game.host.combat().set_health(&owned, 0.0);
        let reaction = DeathReaction {
            pain: PainReaction {
                attack: None,
                this: owned,
                attacker: Some(actor.clone()),
                kick: 0.0,
                damage: 0.0,
            },
            inflictor: Some(actor.clone()),
            point: qa_core::math::vec3(0.0, 0.0, 0.0),
        };
        (definition.die)(context, &reaction);
        if !context.game.host.actors().is_live(&actor) || context.state().gibbed {
            return;
        }
        let movement = context.state().current_move.clone();
        let mut frame_number = movement.first_frame;
        while frame_number < movement.last_frame {
            context.entity_mut().frame = frame_number;
            let actions = types::record_at(
                &movement.frames,
                (frame_number - movement.first_frame) as usize,
            )
            .actions
            .clone();
            for action in actions {
                match action {
                    MonsterAction::Name(name) => context.dispatch(&name),
                    MonsterAction::NextFrame(next) => {
                        let next_frame = match next {
                            NextFrame::Next => context.entity().frame + 1,
                            NextFrame::Frame(number) => number,
                        };
                        context.state_mut().next_frame = next_frame;
                    }
                }
                if !context.game.host.actors().is_live(&actor) {
                    return;
                }
            }
            frame_number += 1;
        }
        if let Some(end) = movement.end.clone() {
            context.dispatch(&end);
        }
        if !context.game.host.actors().is_live(&actor) {
            return;
        }
        context.entity_mut().frame = movement.last_frame;
        let mut moved = context.game.body_of(actor.clone());
        moved.origin = origin;
        context.game.write_body(actor.clone(), &moved, true);
        context.game.show(actor);
        return;
    }
    if (context.entity().spawnflags & 2) != 0 {
        let actor = context.actor().clone();
        context.game.set_solid(actor.clone(), Q2Solid::None);
        context.game.set_motion_kind(actor.clone(), Q2MotionKind::Stationary);
        context.entity_mut().visible = false;
        context.entity_mut().server_flags |= 1;
        let owned = context.game.owned_of(actor.clone());
        context.game.set_combat_traits(&owned, &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(false),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        });
        context.game.show(actor.clone());
        context.entity_mut().use_ =
            Some(source_trigger_use as crate::q2::foundation::host::Q2Use);
        context.game.cancel_actor(actor);
        return;
    }
    let actor = context.actor().clone();
    let mut mission = context.mission(&actor);
    if let Some(mission) = mission.as_mut() {
        mission.started();
    }
    let delay = if context.game.options.edition == Q2Edition::Classic {
        0.1
    } else {
        context.game.host.frame_seconds()
    };
    let think = context.game.source_callbacks.resolve_think(Some("monster_think"));
    let think = think.expect("monster thinker");
    context.game.schedule(actor, delay, think);
}

/// Trigger a monster spawn (`triggerSpawn`).
fn trigger_spawn(context: &mut MonsterContext) {
    use crate::q2::foundation::host::Q2MotionKind;
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor.clone());
    place_triggered_monster(&mut *context.game, owned);
    context.entity_mut().spawnflags &= !2;
    context.entity_mut().server_flags &= !1;
    context.entity_mut().visible = true;
    context.game.set_solid(actor.clone(), crate::q2::foundation::host::Q2Solid::Box);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Step);
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(&owned, &crate::q2::support::contracts::CombatTraitChanges {
        can_take_damage: Some(true),
        ..crate::q2::support::contracts::CombatTraitChanges::default()
    });
    let air = context.game.host.now() + 12.0;
    context.state_mut().air_finished = air;
    start_monster(context);
    let enemy = context.entity().enemy.clone();
    if enemy.is_some()
        && (context.entity().spawnflags & 1) == 0
        && (context
            .game
            .entity(enemy.as_ref().expect("spawn enemy"))
            .map(|target| target.flags)
            .unwrap_or(0)
            & 32)
            == 0
    {
        perception::found_target(context);
    } else {
        context.entity_mut().enemy = None;
    }
    context.entity_mut().use_ = Some(source_use as crate::q2::foundation::host::Q2Use);
    let actor = context.actor().clone();
    context.game.show(actor);
}

/// Drop a monster to the floor (`dropToFloor`).
fn drop_to_floor(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let direction = if context.entity().gravity_vector.z > 0.0 { 1.0 } else { -1.0 };
    let mask = ai::monster_solid_mask(context.game);
    let offset = context.game.options.edition == crate::q2::foundation::host::Q2Edition::Classic
        || context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: body.origin,
            end: body.origin,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        })
        .start_solid;
    let start =
        if offset { vec3(body.origin.x, body.origin.y, body.origin.z - direction) } else { body.origin };
    let mut moved = context.game.body_of(actor.clone());
    moved.origin = start;
    context.game.write_body(actor.clone(), &moved, false);
    let mask = ai::monster_solid_mask(context.game);
    let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
        start,
        end: vec3(start.x, start.y, start.z + direction * 256.0),
        bounds: Some(body.bounds),
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    if trace.fraction != 1.0 && !trace.all_solid {
        let mut moved = context.game.body_of(actor.clone());
        moved.origin = trace.end;
        context.game.write_body(actor.clone(), &moved, true);
        check_ground(context);
        categorize_position(context);
    }
}

/// Process queued pain (`processPain`).
fn process_pain(actor: &ActorId, game: &mut Q2GameServices) {
    let pending = game.monsters.pending_damage.get(actor).cloned();
    let has_context = game.monsters.states.contains_key(actor);
    let definition = game.monsters.actor_definitions.get(actor).cloned();
    let (Some(pending), Some(definition)) = (pending, definition) else { return };
    if !has_context || pending.reaction.pain.damage == 0.0 {
        return;
    }
    game.require_entity_mut(actor).last_attack = pending.attack.clone();
    if ai::health(game, Some(actor)) <= 0.0 {
        {
            let mut context = MonsterContext::new(actor.clone(), &mut *game);
            monster_die(&mut context, definition, pending.reaction.clone());
        }
        let last_frame =
            game.monsters.states.get(actor).map(|state| state.current_move.last_frame);
        if game.host.actors().is_live(actor)
            && ai::health(game, Some(actor))
                > game.monsters.states.get(actor).map(|state| state.gib_health).unwrap_or(0.0)
            && Some(game.require_entity(actor).frame) == last_frame
        {
            let frame = game.require_entity(actor).frame - 1
                - (game.random() * 2.0).floor() as i32;
            game.require_entity_mut(actor).frame = frame;
            let body = game.body_of(actor.clone());
            let locomotion =
                game.monsters.states.get(actor).map(|state| state.locomotion);
            if body.ground.is_some()
                && game.require_entity(actor).motion == crate::q2::foundation::host::Q2MotionKind::Toss
                && locomotion != Some(types::MonsterLocomotion::Stationary)
            {
                let delta = if game.random() < 0.5 { 4.5 } else { -4.5 };
                let mut moved = body.clone();
                moved.angles.y += delta as f32;
                game.write_body(actor.clone(), &moved, true);
            }
        }
    } else if let Some(pain) = definition.pain {
        let mut context = MonsterContext::new(actor.clone(), &mut *game);
        pain(&mut context, &pending.reaction.pain);
    }
    game.monsters.pending_damage.remove(actor);
    if !game.host.actors().is_live(actor) {
        return;
    }
    {
        let mut context = MonsterContext::new(actor.clone(), &mut *game);
        set_skin(&mut context);
        let health_target = context.entity().health_target.clone();
        if !health_target.is_empty() {
            let saved = context.entity().target.clone();
            context.entity_mut().target = health_target;
            let enemy = context.entity().enemy.clone();
            let authored = context.game.require_entity(context.actor()).authored_target();
            context.game.use_targets(&authored, enemy.as_ref(), false);
            context.entity_mut().target = saved;
        }
    }
    if game.host.actors().is_live(actor) {
        game.show(actor.clone());
    }
}

/// Check for dodges (`checkDodge`).
fn check_dodge(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let definition = context.try_definition();
    let Some(definition) = definition else { return };
    if context.state().dead
        || context.state().dodge_time > context.game.host.now()
        || definition.dodge.is_none() && definition.duck.is_none() && definition.sidestep.is_none()
    {
        return;
    }
    let body = context.game.body_of(actor.clone());
    let forward = ai::angles_vectors(body.angles).forward;
    let minimum = vec3(
        body.origin.x + body.bounds.min.x - 512.0,
        body.origin.y + body.bounds.min.y - 512.0,
        body.origin.z + body.bounds.min.z - 512.0,
    );
    let maximum = vec3(
        body.origin.x + body.bounds.max.x + 512.0,
        body.origin.y + body.bounds.max.y + 512.0,
        body.origin.z + body.bounds.max.z + 512.0,
    );
    let mut projectiles: Vec<ActorId> = context
        .game
        .entities
        .values()
        .filter(|projectile| {
            projectile.projectile
                && projectile.dodgeable
                && projectile.solid != crate::q2::foundation::host::Q2Solid::None
                && projectile.owner.is_some()
        })
        .map(|projectile| projectile.actor.id().clone())
        .collect();
    projectiles.sort_by_key(|actor| (actor.slot(), actor.generation()));
    for projectile in projectiles {
        let shot = context.game.body_of(projectile.clone());
        let speed = f64::from(length3(shot.velocity));
        if speed < 4.0 {
            continue;
        }
        let shot_min = add3(shot.origin, shot.bounds.min);
        let shot_max = add3(shot.origin, shot.bounds.max);
        if shot_min.x > maximum.x
            || shot_min.y > maximum.y
            || shot_min.z > maximum.z
            || shot_max.x < minimum.x
            || shot_max.y < minimum.y
            || shot_max.z < minimum.z
        {
            continue;
        }
        let facing = f64::from(dot3(
            qa_core::math::normalize3(sub3(shot.origin, body.origin)),
            forward,
        ));
        if facing <= 0.35 {
            continue;
        }
        let clip_mask = context.game.require_entity(&projectile).clip_mask;
        let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: shot.origin,
            end: add3(shot.origin, shot.velocity),
            bounds: Some(shot.bounds),
            ignore: Some(projectile.clone()),
            mask: clip_mask,
            exclude: Vec::new(),
        });
        let hit_self = matches!(&trace.hit, crate::q2::support::contracts::TraceHit::Actor { actor: hit } if *hit == actor);
        if !hit_self {
            continue;
        }
        let owner = context.game.require_entity(&projectile).owner.clone().expect("projectile owner");
        let motion = context.game.require_entity(&projectile).motion;
        let eta = f64::from(length3(sub3(trace.end, shot.origin))) / speed;
        let gravity = motion == crate::q2::foundation::host::Q2MotionKind::Bounce
            || motion == crate::q2::foundation::host::Q2MotionKind::Toss;
        monster_dodge(context, owner, eta, Some(&trace), gravity);
        break;
    }
}

/// Check ground support (`checkGround`).
fn check_ground(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    if (context.entity().flags & 3) != 0 {
        return;
    }
    if f64::from(body.velocity.z) * f64::from(context.entity().gravity_vector.z) < -100.0 {
        let mut moved = context.game.body_of(actor.clone());
        moved.ground = None;
        context.game.write_body(actor, &moved, false);
        return;
    }
    let gravity = context.entity().gravity_vector.z;
    let mask = ai::monster_solid_mask(context.game);
    let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
        start: body.origin,
        end: vec3(body.origin.x, body.origin.y, body.origin.z + 0.25 * gravity),
        bounds: Some(body.bounds),
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    let steep = match &trace.contact {
        crate::q2::support::contracts::TraceContact::Plane { plane } => {
            if gravity < 0.0 {
                plane.normal.z < 0.7
            } else {
                plane.normal.z > -0.7
            }
        }
        crate::q2::support::contracts::TraceContact::None => true,
    };
    if !trace.start_solid && steep {
        let mut moved = context.game.body_of(actor.clone());
        moved.ground = None;
        context.game.write_body(actor, &moved, false);
        return;
    }
    if !trace.start_solid && !trace.all_solid {
        let end = trace.end;
        let game = &mut *context.game;
        let ground = ai::trace_ground_actor(&trace, game);
        let mut moved = game.body_of(actor.clone());
        moved.origin = end;
        moved.ground = ground;
        moved.velocity.z = 0.0;
        game.write_body(actor, &moved, false);
    }
}

/// Categorize water position (`categorizePosition`).
fn categorize_position(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let feet = vec3(
        body.origin.x,
        body.origin.y,
        body.origin.z
            + if context.entity().gravity_vector.z > 0.0 {
                body.bounds.max.z - 1.0
            } else {
                body.bounds.min.z + 1.0
            },
    );
    let contents = context.game.host.point_contents(feet);
    let level = if contents & ai::MASK_WATER == 0 {
        0
    } else if context.game.host.point_contents(vec3(feet.x, feet.y, feet.z + 26.0)) & ai::MASK_WATER == 0 {
        1
    } else if context.game.host.point_contents(vec3(feet.x, feet.y, feet.z + 48.0)) & ai::MASK_WATER == 0 {
        2
    } else {
        3
    };
    context.state_mut().water_level = level;
    context.state_mut().water_type = if level == 0 { 0 } else { contents };
}

/// Apply world effects (`worldEffects`).
fn world_effects(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let water = context.state().water_level;
    let contents = context.state().water_type;
    let world = context.game.host.world_actor();
    if ai::health(context.game, Some(&actor)) > 0.0 {
        let swim = context.state().locomotion == types::MonsterLocomotion::Swim;
        if if swim { water > 0 } else { water < 3 } {
            let air = context.game.host.now() + if swim { 9.0 } else { 12.0 };
            context.state_mut().air_finished = air;
        } else if context.state().air_finished < context.game.host.now()
            && context.state().pain_time < context.game.host.now()
        {
            let damage =
                (15.0f64).min(2.0 + 2.0 * (context.game.host.now() - context.state().air_finished).floor());
            let origin = body.origin;
            context.game.damage(
                actor.clone(),
                world.clone(),
                Some(world.clone()),
                damage,
                0.0,
                vec3(0.0, 0.0, 0.0),
                origin,
                vec3(0.0, 0.0, 0.0),
                17,
                2,
                None,
            );
            let pain = context.game.host.now() + 1.0;
            context.state_mut().pain_time = pain;
        }
    }
    if water == 0 {
        if (context.entity().flags & 8) != 0 {
            context.game.sound(&actor, "player/watr_out.wav", 4, 1.0, 1.0);
        }
        context.entity_mut().flags &= !8;
        return;
    }
    if (contents & 8) != 0
        && (context.entity().flags & 128) == 0
        && context.state().environmental_damage_time < context.game.host.now()
    {
        let damage = context.game.host.now() + 0.2;
        context.state_mut().environmental_damage_time = damage;
        let origin = body.origin;
        context.game.damage(
            actor.clone(),
            world.clone(),
            Some(world.clone()),
            10.0 * water as f64,
            0.0,
            vec3(0.0, 0.0, 0.0),
            origin,
            vec3(0.0, 0.0, 0.0),
            19,
            0,
            None,
        );
    }
    if (contents & 16) != 0
        && (context.entity().flags & 64) == 0
        && context.state().environmental_damage_time < context.game.host.now()
    {
        let damage = context.game.host.now() + 1.0;
        context.state_mut().environmental_damage_time = damage;
        let origin = body.origin;
        context.game.damage(
            actor.clone(),
            world.clone(),
            Some(world.clone()),
            4.0 * water as f64,
            0.0,
            vec3(0.0, 0.0, 0.0),
            origin,
            vec3(0.0, 0.0, 0.0),
            18,
            0,
            None,
        );
    }
    if (context.entity().flags & 8) == 0 {
        if !context.state().dead {
            let path = if (contents & 8) != 0 {
                if context.game.random() <= 0.5 {
                    "player/lava1.wav"
                } else {
                    "player/lava2.wav"
                }
            } else {
                "player/watr_in.wav"
            };
            context.game.sound(&actor, path, 4, 1.0, 1.0);
        }
        context.entity_mut().flags |= 8;
        context.state_mut().environmental_damage_time = 0.0;
    }
}

/// Touch a path corner (`touchPathCorner`).
pub fn touch_path_corner(corner: ActorId, game: &mut Q2GameServices, actor: ActorId) {
    let internal = game.monsters.states.contains_key(&actor);
    if !internal {
        let mut factory = game.monsters.external_path_follower.take();
        let follower = factory.as_mut().and_then(|factory| factory(&actor));
        game.monsters.external_path_follower = factory;
        let Some(mut follower) = follower else { return };
        touch_path_corner_external(&corner, game, &actor, &mut *follower);
        return;
    }
    let move_target = game.monsters.states.get(&actor).and_then(|state| state.move_target.clone());
    let enemy = game.require_entity(&actor).enemy.clone();
    if move_target != Some(corner.clone()) || enemy.is_some() {
        return;
    }
    let path_target = game.require_entity(&corner).spawn.values.get("pathtarget").cloned();
    if let Some(path_target) = path_target {
        let mut authored = game.require_entity(&corner).authored_target();
        authored.target = path_target;
        game.use_targets(&authored, Some(&actor), false);
    }
    let target_name = game.require_entity(&corner).target.clone();
    let mut next = if target_name.is_empty() { None } else { game.pick_target(&target_name) };
    if let Some(next_actor) = next.clone() {
        if (game.require_entity(&next_actor).spawnflags & 1) != 0 {
            let destination = game.body_of(next_actor.clone());
            let Some(body) = game.host.bodies().read(&actor) else { return };
            let origin = vec3(
                destination.origin.x,
                destination.origin.y,
                destination.origin.z + destination.bounds.min.z - body.bounds.min.z,
            );
            let mut moved = game.body_of(actor.clone());
            moved.origin = origin;
            game.write_body(actor.clone(), &moved, true);
            game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::EntityEvent {
                actor: actor.clone(),
                event: 7,
            });
            let next_target = game.require_entity(&next_actor).target.clone();
            next = if next_target.is_empty() { None } else { game.pick_target(&next_target) };
        }
    }
    let (name, goal) = match next.clone() {
        None => (String::new(), None),
        Some(next_actor) => {
            let targetname = game.require_entity(&next_actor).targetname.clone();
            let id = game.require_entity(&next_actor).actor.id().clone();
            (targetname, Some(id))
        }
    };
    let wait = game.require_entity(&corner).wait;
    let pause_until = if wait != 0.0 {
        game.host.now() + wait
    } else if next.is_none() {
        game.host.now() + 100000000.0
    } else {
        0.0
    };
    let mut context = MonsterContext::new(actor.clone(), game);
    context.entity_mut().goal = goal.clone();
    context.state_mut().move_target = goal.clone();
    if pause_until != 0.0 {
        context.state_mut().pause_time = pause_until;
        context.stand();
    } else {
        let target = goal.as_ref().and_then(|goal| context.game.host.bodies().read(goal));
        if let Some(target) = target {
            let actor = context.actor().clone();
            let origin = context.game.body_of(actor).origin;
            let yaw = f64::from(ai::vector_angles(sub3(target.origin, origin)).y);
            context.state_mut().ideal_yaw = yaw;
        }
    }
    let _ = name;
}

/// Touch a path corner for an external follower.
fn touch_path_corner_external(
    corner: &ActorId,
    game: &mut Q2GameServices,
    actor: &ActorId,
    follower: &mut dyn Q2PathFollower,
) {
    if follower.move_target() != Some(corner) || follower.enemy().is_some() {
        return;
    }
    let path_target = game.require_entity(corner).spawn.values.get("pathtarget").cloned();
    if let Some(path_target) = path_target {
        let mut authored = game.require_entity(corner).authored_target();
        authored.target = path_target;
        game.use_targets(&authored, Some(actor), false);
    }
    let target_name = game.require_entity(corner).target.clone();
    let mut next = if target_name.is_empty() { None } else { game.pick_target(&target_name) };
    if let Some(next_actor) = next.clone() {
        if (game.require_entity(&next_actor).spawnflags & 1) != 0 {
            let destination = game.body_of(next_actor.clone());
            let Some(body) = game.host.bodies().read(actor) else { return };
            let origin = vec3(
                destination.origin.x,
                destination.origin.y,
                destination.origin.z + destination.bounds.min.z - body.bounds.min.z,
            );
            let owned = game.owned_of(actor.clone());
            let mut moved = body.clone();
            moved.origin = origin;
            game.host.bodies().write(&owned, &moved);
            game.host.bodies().link(&owned, None);
            game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::EntityEvent {
                actor: actor.clone(),
                event: 7,
            });
            let next_target = game.require_entity(&next_actor).target.clone();
            next = if next_target.is_empty() { None } else { game.pick_target(&next_target) };
        }
    }
    let (name, goal) = match next.clone() {
        None => (String::new(), None),
        Some(next_actor) => {
            let targetname = game.require_entity(&next_actor).targetname.clone();
            let id = game.require_entity(&next_actor).actor.id().clone();
            (targetname, Some(id))
        }
    };
    let wait = game.require_entity(corner).wait;
    let pause_until = if wait != 0.0 {
        game.host.now() + wait
    } else if next.is_none() {
        game.host.now() + 100000000.0
    } else {
        0.0
    };
    follower.advance(&name, goal.as_ref(), pause_until);
}

/// Touch a combat point (`touchCombatPoint`).
pub fn touch_combat_point(corner: ActorId, game: &mut Q2GameServices, actor: ActorId) {
    let internal = game.monsters.states.contains_key(&actor);
    if !internal {
        let mut factory = game.monsters.external_combat_follower.take();
        let follower = factory.as_mut().and_then(|factory| factory(&actor));
        game.monsters.external_combat_follower = factory;
        let Some(mut follower) = follower else { return };
        touch_combat_point_external(&corner, game, &actor, &mut *follower);
        return;
    }
    let move_target = game.monsters.states.get(&actor).and_then(|state| state.move_target.clone());
    if move_target != Some(corner.clone()) {
        return;
    }
    if !game.require_entity(&corner).target.is_empty() {
        let target_name = game.require_entity(&corner).target.clone();
        let target = game.pick_target(&target_name);
        let goal = target.as_ref().and_then(|target| {
            game.entity(target).map(|entity| entity.actor.id().clone())
        });
        let mut context = MonsterContext::new(actor.clone(), &mut *game);
        context.entity_mut().target = target_name.clone();
        context.entity_mut().goal = goal.clone();
        context.state_mut().move_target = goal.or(Some(corner.clone()));
        let game = &mut *context.game;
        if target.is_none() {
            game.host.diagnostic(&format!("point_combat target {target_name} does not exist"));
        }
        game.require_entity_mut(&corner).target = String::new();
    } else if (game.require_entity(&corner).spawnflags & 1) != 0
        && game.monsters.states.get(&actor).is_some_and(|state| state.locomotion == types::MonsterLocomotion::Walk)
    {
        let mut context = MonsterContext::new(actor.clone(), &mut *game);
        let pause = context.game.host.now() + 100000000.0;
        context.state_mut().pause_time = pause;
        context.state_mut().stand_ground = true;
        context.stand();
    }
    let move_target = game.monsters.states.get(&actor).and_then(|state| state.move_target.clone());
    if move_target == Some(corner.clone()) {
        let mut context = MonsterContext::new(actor.clone(), &mut *game);
        context.entity_mut().target = String::new();
        context.state_mut().move_target = None;
        let enemy = context.entity().enemy.clone();
        context.entity_mut().goal = enemy;
        context.state_mut().combat_point = false;
    }
    let path_target = game.require_entity(&corner).spawn.values.get("pathtarget").cloned();
    if let Some(path_target) = path_target {
        let enemy = game.require_entity(&actor).enemy.clone();
        let old_enemy = game.monsters.states.get(&actor).and_then(|state| state.old_enemy.clone());
        let activator_entity = game.require_entity(&actor).activator.clone();
        let mut chosen = actor.clone();
        for candidate in [enemy, old_enemy, activator_entity].into_iter().flatten() {
            if game.host.is_player(&candidate) {
                chosen = candidate;
                break;
            }
        }
        let mut authored = game.require_entity(&corner).authored_target();
        authored.target = path_target;
        game.use_targets(&authored, Some(&chosen), false);
    }
}

/// Touch a combat point for an external follower.
fn touch_combat_point_external(
    corner: &ActorId,
    game: &mut Q2GameServices,
    actor: &ActorId,
    follower: &mut dyn Q2CombatFollower,
) {
    if follower.move_target() != Some(corner) {
        return;
    }
    if !game.require_entity(corner).target.is_empty() {
        let target_name = game.require_entity(corner).target.clone();
        let target = game.pick_target(&target_name);
        let goal = target.as_ref().and_then(|target| {
            game.entity(target).map(|entity| entity.actor.id().clone())
        });
        let move_target = goal.clone().or(Some(corner.clone()));
        follower.advance(&target_name, goal.as_ref(), move_target.as_ref());
        if target.is_none() {
            game.host.diagnostic(&format!("point_combat target {target_name} does not exist"));
        }
        game.require_entity_mut(corner).target = String::new();
    } else if (game.require_entity(corner).spawnflags & 1) != 0 && follower.walking() {
        follower.hold();
    }
    if follower.move_target() == Some(corner) {
        follower.finish();
    }
    let path_target = game.require_entity(corner).spawn.values.get("pathtarget").cloned();
    if let Some(path_target) = path_target {
        let mut chosen = actor.clone();
        for candidate in [follower.enemy(), follower.old_enemy(), follower.activator()]
            .into_iter()
            .flatten()
        {
            if game.host.is_player(candidate) {
                chosen = candidate.clone();
                break;
            }
        }
        let mut authored = game.require_entity(corner).authored_target();
        authored.target = path_target;
        game.use_targets(&authored, Some(&chosen), false);
    }
}

/// Spawn a monster (`spawn`, `Q2SpawnModule`).
pub fn monster_spawn(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    let Some(definition) = monster_definition(&classname, game) else { return false };
    admit_monster(game, actor, definition, false, false)
}

/// Spawn a summoned monster (`spawnSummoned`).
pub fn spawn_summoned_monster(game: &mut Q2GameServices, actor: ActorId) {
    let classname = game.require_entity(&actor).classname.clone();
    let Some(definition) = monster_definition(&classname, game) else {
        panic!("Cannot summon unknown Q2 monster {classname}");
    };
    admit_monster(game, actor.clone(), definition, false, true);
    if !game.monsters.states.contains_key(&actor) {
        panic!("Q2 monster {classname} was inhibited during summoning");
    }
}

/// Respawn a monster (`respawn`).
pub fn respawn_monster(game: &mut Q2GameServices, actor: ActorId) {
    let classname = game.require_entity(&actor).classname.clone();
    let definition = game
        .monsters
        .actor_definitions
        .get(&actor)
        .cloned()
        .or_else(|| monster_definition(&classname, game));
    let Some(definition) = definition else {
        panic!("Cannot revive unknown Q2 monster {classname}");
    };
    game.cancel_actor(actor.clone());
    admit_monster(game, actor.clone(), definition, true, false);
    if !game.monsters.states.contains_key(&actor) {
        panic!("Q2 monster {classname} was inhibited during revival");
    }
}

/// Resume a monster (`resumeMonster`).
pub fn resume_monster(game: &mut Q2GameServices, actor: ActorId) {
    if !game.monsters.states.contains_key(&actor) {
        let classname = game.require_entity(&actor).classname.clone();
        panic!("No monster callbacks for {classname}");
    }
    let delay = if game.options.edition == crate::q2::foundation::host::Q2Edition::Classic {
        0.1
    } else {
        game.host.frame_seconds()
    };
    let think = game.source_callbacks.resolve_think(Some("monster_think"));
    let think = think.expect("monster thinker");
    game.schedule(actor, delay, think);
}

/// Spawn an infantry turret driver (`spawnInfantryDriver`).
pub fn spawn_infantry_driver(game: &mut Q2GameServices, actor: ActorId) {
    let Some(infantry) = built_in_monster("monster_infantry", game) else {
        panic!("Missing infantry definition");
    };
    let definition = Rc::new(Q2MonsterDefinition {
        classname: "turret_driver".to_string(),
        gib_health: 0.0,
        view_height: Some(24),
        ..(*infantry).clone()
    });
    admit_monster(game, actor.clone(), definition, false, false);
    if !game.monsters.states.contains_key(&actor) {
        panic!("Turret infantry driver was inhibited");
    }
}

/// Run the frame perception update (`beginFrame`).
pub fn begin_monster_frame(game: &mut Q2GameServices) {
    perception::begin_frame(game);
}

/// Run end-of-frame pain (`endFrame`).
pub fn end_monster_frame(game: &mut Q2GameServices) {
    if game.options.edition != crate::q2::foundation::host::Q2Edition::Rerelease {
        return;
    }
    let mut actors: Vec<ActorId> = game.monsters.states.keys().cloned().collect();
    actors.sort_by_key(|actor| {
        game.host.actors().source_of(actor).map(|(_, slot)| slot).unwrap_or(actor.slot())
    });
    for actor in actors {
        if game.host.actors().is_live(&actor)
            && (game.require_entity(&actor).server_flags & 4) != 0
        {
            process_pain(&actor, game);
        }
    }
}

/// Report a noise (`reportNoise`).
pub fn report_monster_noise(
    game: &mut Q2GameServices,
    actor: ActorId,
    origin: Vec3,
    secondary: bool,
) {
    perception::report_noise(game, actor, origin, secondary);
}

/// Look up a monster definition (`definition`).
pub fn monster_definition(
    classname: &str,
    game: &Q2GameServices,
) -> Option<Rc<Q2MonsterDefinition>> {
    if let Some(registered) = game
        .monsters
        .edition_definitions
        .get(&game.options.edition)
        .and_then(|definitions| definitions.get(classname))
        .or_else(|| game.monsters.definitions.get(classname))
    {
        return Some(registered.clone());
    }
    if classname == "turret_driver" {
        let infantry = built_in_monster("monster_infantry", game)?;
        return Some(Rc::new(Q2MonsterDefinition {
            classname: classname.to_string(),
            gib_health: 0.0,
            view_height: Some(24),
            ..(*infantry).clone()
        }));
    }
    built_in_monster(classname, game)
}

/// Register a monster definition (`register`).
pub fn register_monster(
    game: &mut Q2GameServices,
    definition: Q2MonsterDefinition,
    edition: Option<crate::q2::foundation::host::Q2Edition>,
) {
    if let Some(edition) = edition {
        if game.monsters.edition_definitions.get(&edition).is_some_and(|definitions| {
            definitions.contains_key(&definition.classname)
        }) {
            panic!("Duplicate Q2 monster definition {}", definition.classname);
        }
    } else if game.monsters.definitions.contains_key(&definition.classname) {
        panic!("Duplicate Q2 monster definition {}", definition.classname);
    }
    for movement in &definition.moves {
        for frame in &movement.frames {
            if let MonsterAi::Source(name) = &frame.ai {
                if definition.ai.get(name).is_none() {
                    panic!("Missing Q2 source AI {name}");
                }
            }
        }
        if movement.frames.len() < (movement.last_frame - movement.first_frame + 1) as usize {
            panic!("Q2 move {} has an incomplete frame table", movement.name);
        }
        let mut callbacks: Vec<Option<String>> = vec![movement.end.clone()];
        for frame in &movement.frames {
            for action in &frame.actions {
                if let MonsterAction::Name(name) = action {
                    callbacks.push(Some(name.clone()));
                }
            }
        }
        for callback in callbacks.into_iter().flatten() {
            if definition.callbacks.get(&callback).is_none()
                && shared_callback(&callback).is_none()
            {
                panic!("Q2 move {} references missing callback {callback}", movement.name);
            }
        }
    }
    if let Some(edition) = edition {
        game.monsters
            .edition_definitions
            .entry(edition)
            .or_default()
            .insert(definition.classname.clone(), Rc::new(definition));
    } else {
        game.monsters.definitions.insert(definition.classname.clone(), Rc::new(definition));
    }
}

/// Infantry idle (`idle` in `builtIn`).
fn infantry_idle(context: &mut MonsterContext) {
    if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Rerelease
        && context.entity().enemy.is_some()
    {
        return;
    }
    context.set_move("infantry_move_fidget", true);
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infidle1.wav", 2, 1.0, 2.0);
}

/// Soldier initialize (`initialize` in `builtIn`).
fn soldier_initialize(context: &mut MonsterContext) {
    if context.game.options.edition == crate::q2::foundation::host::Q2Edition::Classic
        || (context.entity().spawnflags & 8) != 0
    {
        soldier::soldier_stand(context);
        return;
    }
    // The donor draws and discards one RNG sample here.
    context.game.random();
    context.set_move("soldier_move_stand1", true);
}

/// Merged built-in callbacks (`builtInCallbacks`).
fn built_in_callbacks() -> HashMap<String, MonsterHandler> {
    let mut callbacks = soldier::soldier_callbacks();
    callbacks.extend(infantry::infantry_callbacks());
    for name in actions::MONSTER_CALLBACKS {
        if let Some(handler) = shared_callback(name) {
            callbacks.entry(name.to_string()).or_insert(handler);
        }
    }
    callbacks
}

/// Built-in definitions (`builtIn`).
pub fn built_in_monster(
    classname: &str,
    game: &Q2GameServices,
) -> Option<Rc<Q2MonsterDefinition>> {
    use crate::q2::foundation::host::Q2Edition;
    let classic = game.options.edition == Q2Edition::Classic;
    let bounds = qa_core::math::Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, 32.0),
    };
    if classname == "monster_infantry" {
        return Some(Rc::new(Q2MonsterDefinition {
            classname: classname.to_string(),
            kind: "infantry".to_string(),
            model: "models/monsters/infantry/tris.md2".to_string(),
            health: 100.0,
            gib_health: if classic { -40.0 } else { -65.0 },
            mass: 200.0,
            bounds,
            scale: 1.0,
            view_height: None,
            yaw_speed: None,
            locomotion: None,
            initial_move: "infantry_move_stand".to_string(),
            moves: if classic {
                moves::classic_infantry_moves()
            } else {
                moves::rerelease_infantry_moves()
            },
            callbacks: built_in_callbacks(),
            stand: MonsterHandler::Callback(infantry::infantry_stand),
            walk: MonsterHandler::Callback(infantry::infantry_walk),
            run: MonsterHandler::Callback(infantry::infantry_run),
            attack: MonsterHandler::Callback(infantry::infantry_attack),
            sight: Some(MonsterHandler::Callback(infantry::infantry_sight)),
            idle: Some(MonsterHandler::Callback(infantry_idle)),
            search: None,
            melee: None,
            has_ranged_attack: true,
            blind_fire: false,
            pain: Some(infantry::infantry_pain),
            die: infantry::infantry_die,
            ai: HashMap::new(),
            source_callbacks: None,
            initialize: None,
            after_spawn: None,
            start_mode: None,
            restore: None,
            duck: Some(infantry::infantry_duck),
            sidestep: Some(infantry::infantry_sidestep),
            dodge: None,
            blocked: None,
            check_attack: None,
        }));
    }
    if classname == "monster_soldier_light"
        || classname == "monster_soldier"
        || classname == "monster_soldier_ss"
    {
        return Some(Rc::new(Q2MonsterDefinition {
            classname: classname.to_string(),
            kind: "soldier".to_string(),
            model: "models/monsters/soldier/tris.md2".to_string(),
            health: if classname == "monster_soldier_light" {
                20.0
            } else if classname == "monster_soldier" {
                30.0
            } else {
                40.0
            },
            gib_health: -30.0,
            mass: 100.0,
            bounds,
            scale: 1.0,
            view_height: None,
            yaw_speed: None,
            locomotion: None,
            initial_move: "soldier_move_stand1".to_string(),
            moves: if classic {
                moves::classic_soldier_moves()
            } else {
                moves::rerelease_soldier_moves()
            },
            callbacks: built_in_callbacks(),
            stand: MonsterHandler::Callback(soldier::soldier_stand),
            walk: MonsterHandler::Callback(soldier::soldier_walk),
            run: MonsterHandler::Callback(soldier::soldier_run),
            attack: MonsterHandler::Callback(soldier::soldier_attack),
            sight: Some(MonsterHandler::Callback(soldier::soldier_sight)),
            idle: None,
            search: None,
            melee: None,
            has_ranged_attack: true,
            blind_fire: classname != "monster_soldier_ss",
            pain: Some(soldier::soldier_pain),
            die: soldier::soldier_die,
            ai: HashMap::new(),
            source_callbacks: None,
            initialize: Some(MonsterHandler::Callback(soldier_initialize)),
            after_spawn: None,
            start_mode: None,
            restore: None,
            duck: Some(soldier::soldier_duck),
            sidestep: Some(soldier::soldier_sidestep),
            dodge: None,
            blocked: None,
            check_attack: None,
        }));
    }
    None
}

/// Merge callback records (`mergeCallbacks`).
fn merge_callbacks<T: Copy + PartialEq + std::fmt::Debug>(
    sources: Vec<Option<HashMap<&'static str, T>>>,
    kind: &str,
) -> HashMap<&'static str, T> {
    let mut result: HashMap<&'static str, T> = HashMap::new();
    for source in sources.into_iter().flatten() {
        for (name, callback) in source {
            if result.get(name).is_some_and(|existing| *existing != callback) {
                panic!("Duplicate Q2 monster source callback {name} ({kind})");
            }
            result.insert(name, callback);
        }
    }
    result
}

/// Monster callback definitions (`callbacks`).
pub fn monster_callbacks(game: &Q2GameServices) -> super::callbacks::Q2CallbackDefinitions {
    use crate::q2::foundation::host::{Q2Die, Q2Pain, Q2Think, Q2Touch, Q2Use};
    let gib = gibs::q2_gib_callbacks();
    let mut think: HashMap<&'static str, Q2Think> = HashMap::from([
        ("monster_think", source_think as Q2Think),
        ("monster_start_go", source_start as Q2Think),
        ("monster_triggered_spawn", source_trigger_spawn as Q2Think),
        ("monster_dead_think", source_dead_think as Q2Think),
        ("M_FliesOn", source_flies_on as Q2Think),
        ("M_FliesOff", source_flies_off as Q2Think),
    ]);
    think.extend(gib.think);
    let use_: HashMap<&'static str, Q2Use> = HashMap::from([
        ("monster_use", source_use as Q2Use),
        ("monster_triggered_spawn_use", source_trigger_use as Q2Use),
    ]);
    let pain: HashMap<&'static str, Q2Pain> =
        HashMap::from([("monster_pain", source_pain as Q2Pain)]);
    let mut die: HashMap<&'static str, Q2Die> =
        HashMap::from([("monster_die", source_die as Q2Die)]);
    die.extend(gib.die);
    let mut sources_think = vec![Some(think)];
    let mut sources_use = vec![Some(use_)];
    let mut sources_pain = vec![Some(pain)];
    let mut sources_die = vec![Some(die)];
    let mut sources_touch: Vec<Option<HashMap<&'static str, Q2Touch>>> = vec![Some(gib.touch)];
    let mut sources_blocked: Vec<
        Option<HashMap<&'static str, crate::q2::foundation::host::Q2Blocked>>,
    > = vec![Some(HashMap::new())];
    let mut definitions: Vec<Rc<Q2MonsterDefinition>> =
        game.monsters.definitions.values().cloned().collect();
    for entries in game.monsters.edition_definitions.values() {
        definitions.extend(entries.values().cloned());
    }
    for definition in definitions {
        if let Some(callbacks) = definition.source_callbacks.clone() {
            sources_think.push(Some(callbacks.think));
            sources_use.push(Some(callbacks.use_));
            sources_pain.push(Some(callbacks.pain));
            sources_die.push(Some(callbacks.die));
            sources_touch.push(Some(callbacks.touch));
            sources_blocked.push(Some(callbacks.blocked));
        }
    }
    super::callbacks::Q2CallbackDefinitions {
        trajectory: Vec::new(),
        think: merge_callbacks(sources_think, "think"),
        use_: merge_callbacks(sources_use, "use"),
        touch: merge_callbacks(sources_touch, "touch"),
        pain: merge_callbacks(sources_pain, "pain"),
        die: merge_callbacks(sources_die, "die"),
        blocked: merge_callbacks(sources_blocked, "blocked"),
    }
}

/// Connect monster callbacks (`connect`).
fn connect_monsters(game: &mut Q2GameServices) {
    if !game.monsters.connected {
        game.monsters.connected = true;
    }
    let callbacks = monster_callbacks(game);
    game.source_callbacks.register(&callbacks);
}

/// Place a triggered monster (`placeTriggeredMonster`).
pub fn place_triggered_monster(game: &mut Q2GameServices, actor: OwnedActor) {
    use crate::q2::support::contracts::TraceHit;
    let body = game
        .host
        .bodies()
        .read(actor.id())
        .expect("Triggered monster has no shared body");
    let origin = vec3(body.origin.x, body.origin.y, body.origin.z + 1.0);
    let mut moved = body.clone();
    moved.origin = origin;
    game.host.bodies().write(&actor, &moved);
    for _ in 0..1024 {
        let mask = ai::monster_solid_mask(game);
        let trace = game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: origin,
            end: origin,
            bounds: Some(body.bounds),
            ignore: Some(actor.id().clone()),
            mask,
            exclude: Vec::new(),
        });
        let TraceHit::Actor { actor: target } = trace.hit.clone() else { break };
        if game.host.combat().read(&target).is_none() {
            break;
        }
        game.damage(
            target.clone(),
            actor.id().clone(),
            Some(actor.id().clone()),
            100000.0,
            0.0,
            vec3(0.0, 0.0, 0.0),
            origin,
            vec3(0.0, 0.0, 0.0),
            21,
            8,
            None,
        );
        let mask = ai::monster_solid_mask(game);
        let remaining = game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: origin,
            end: origin,
            bounds: Some(body.bounds),
            ignore: Some(actor.id().clone()),
            mask,
            exclude: Vec::new(),
        });
        if matches!(&remaining.hit, TraceHit::Actor { actor: hit } if *hit == target) {
            break;
        }
    }
}

/// Set source combat rules (`setSourceCombatRules`).
pub fn set_source_combat_rules(
    game: &mut Q2GameServices,
    rules: SourceCombatMode,
    hooks: Option<Box<dyn Q2MonsterSourceCombatHooks>>,
) {
    game.monsters.source_combat = rules;
    game.monsters.source_combat_hooks = hooks;
}

/// Set hint paths (`setHintPaths`).
pub fn set_hint_paths(game: &mut Q2GameServices, hooks: Box<dyn Q2MonsterHintHooks>) {
    game.monsters.hint_hooks = Some(hooks);
}

/// Capture monster state (`capture`).
pub fn capture_monsters(game: &Q2GameServices) -> checkpoint::Q2MonstersCheckpoint {
    use super::checkpoint::save_q2_actor;
    let mut actors: Vec<ActorId> = game.monsters.states.keys().cloned().collect();
    actors.sort_by_key(|actor| (actor.slot(), actor.generation()));
    let mut saved = Vec::new();
    for actor in actors {
        let state = game.monsters.states.get(&actor).expect("monster state").clone();
        let entity = game.require_entity(&actor);
        let definition = game.monsters.actor_definitions.get(&actor).unwrap_or_else(|| {
            panic!("Cannot save missing Q2 monster definition {}", entity.classname)
        });
        let pending = game.monsters.pending_damage.get(&actor).map(|pending| {
            checkpoint::Q2MonsterDamageCheckpoint {
                damage: pending.reaction.pain.damage,
                kick: pending.reaction.pain.kick,
                point: pending.reaction.point,
                attacker: save_q2_actor(pending.reaction.pain.attacker.as_ref()),
                inflictor: save_q2_actor(pending.reaction.inflictor.as_ref()),
                attack: pending
                    .attack
                    .as_ref()
                    .map(super::checkpoint::save_q2_attack),
            }
        });
        let sound_target = state.sound_target.as_ref().map(|target| {
            checkpoint::SavedSoundTarget {
                actor: save_q2_actor(Some(&target.actor)).expect("live actor"),
                owner: save_q2_actor(Some(&target.owner)).expect("live actor"),
                origin: target.origin,
                time: target.time,
            }
        });
        saved.push(checkpoint::Q2MonsterActorCheckpoint {
            actor: save_q2_actor(Some(&actor)).expect("live actor"),
            definition: definition.classname.clone(),
            state: checkpoint::Q2MonsterStateCheckpoint {
                movement: state.current_move.name.clone(),
                next_move: state.next_move.as_ref().map(|movement| movement.name.clone()),
                sound_target,
                old_enemy: save_q2_actor(state.old_enemy.as_ref()),
                move_target: save_q2_actor(state.move_target.as_ref()),
                commander: save_q2_actor(state.commander.as_ref()),
                state,
            },
            pending_damage: pending,
        });
    }
    checkpoint::Q2MonstersCheckpoint {
        version: 1,
        actors: saved,
        perception: perception::capture(game),
    }
}

/// Restore monster state (`restore`).
///
/// Shared actor/health/body tables and Q2 entities must be restored
/// first. No source callback executes.
pub fn restore_monsters(game: &mut Q2GameServices, checkpoint: &checkpoint::Q2MonstersCheckpoint) {
    use super::checkpoint::restore_q2_actor;
    connect_monsters(game);
    game.monsters.states.clear();
    game.monsters.actor_definitions.clear();
    game.monsters.pending_damage.clear();
    let mut restored = Vec::new();
    for saved_actor in checkpoint.actors.clone() {
        let owner = restore_q2_actor(game, saved_actor.actor);
        let definition = monster_definition(&saved_actor.definition, game)
            .unwrap_or_else(|| panic!("Missing restored Q2 monster {}", saved_actor.definition));
        if game.entity(owner.id()).is_none() {
            panic!("Missing restored Q2 monster {}", saved_actor.definition);
        }
        let find_move = |name: &str| {
            definition
                .moves
                .iter()
                .find(|movement| movement.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("Unknown saved Q2 monster move {name}"))
        };
        let mut state = saved_actor.state.state.clone();
        state.current_move = find_move(&saved_actor.state.movement);
        state.next_move = saved_actor.state.next_move.as_deref().map(find_move);
        state.sound_target = saved_actor.state.sound_target.as_ref().map(|target| {
            types::MonsterSoundTarget {
                actor: game.host.actors().reference_saved(target.actor),
                owner: game.host.actors().reference_saved(target.owner),
                origin: target.origin,
                time: target.time,
            }
        });
        state.old_enemy = saved_actor
            .state
            .old_enemy
            .map(|id| game.host.actors().reference_saved(id));
        state.move_target = saved_actor
            .state
            .move_target
            .map(|id| game.host.actors().reference_saved(id));
        state.commander =
            saved_actor.state.commander.map(|id| game.host.actors().reference_saved(id));
        game.monsters.states.insert(owner.id().clone(), state);
        game.monsters.actor_definitions.insert(owner.id().clone(), definition);
        if let Some(pending) = saved_actor.pending_damage {
            let mut reference = |id: qa_core::identity::SavedActorId| {
                game.host.actors().reference_saved(id)
            };
            let attack = pending
                .attack
                .as_ref()
                .map(|attack| super::checkpoint::restore_q2_attack(attack, &mut reference));
            let attacker =
                pending.attacker.map(|id| game.host.actors().reference_saved(id));
            let inflictor =
                pending.inflictor.map(|id| game.host.actors().reference_saved(id));
            game.monsters.pending_damage.insert(owner.id().clone(), PendingMonsterDamage {
                reaction: DeathReaction {
                    pain: PainReaction {
                        attack: attack.clone(),
                        this: owner.clone(),
                        attacker,
                        kick: pending.kick,
                        damage: pending.damage,
                    },
                    inflictor,
                    point: pending.point,
                },
                attack,
            });
        }
        restored.push(owner.id().clone());
    }
    perception::restore(game, &checkpoint.perception);
    for actor in restored {
        let restore = game
            .monsters
            .actor_definitions
            .get(&actor)
            .and_then(|definition| definition.restore.clone());
        if let Some(restore) = restore {
            let mut context = MonsterContext::new(actor, &mut *game);
            restore.dispatch(&mut context);
        }
    }
}

/// Set a monster route (`setRoute`).
pub fn set_monster_route(
    game: &mut Q2GameServices,
    actor: ActorId,
    goal: Option<ActorId>,
    pause_until: f64,
) {
    if !game.monsters.states.contains_key(&actor) {
        panic!("Monster route has no source continuation");
    }
    let mut context = MonsterContext::new(actor.clone(), game);
    context.entity_mut().goal = goal.clone();
    context.state_mut().move_target = goal.clone();
    context.state_mut().pause_time = pause_until;
    if goal.is_none() || pause_until > context.game.host.now() {
        context.stand();
    } else {
        context.walk();
    }
    let target = goal.as_ref().and_then(|goal| context.game.host.bodies().read(goal));
    if let Some(target) = target {
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        let yaw = f64::from(ai::vector_angles(sub3(target.origin, origin)).y);
        context.state_mut().ideal_yaw = yaw;
    }
}
