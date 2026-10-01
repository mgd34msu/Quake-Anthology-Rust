//! Monster target selection, attacks and pursuit (`src/content/q2/foundation/monsters/perception.ts`).
//!
//! Quake II `g_ai.c` target selection, attacks and player-trail
//! pursuit (id Software, GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, length3, scale3, sub3, vec3, Vec3};

use super::ai::{
    angles_vectors, attack_trace_mask, change_yaw, chase_direction, enemy_body, enemy_eye, health, in_front,
    monster_solid_mask, step_direction, target_distance, vector_angles, visible, FL_NOTARGET, MASK_OPAQUE,
};
use super::checkpoint::{save_actor, Q2MonsterPerceptionCheckpoint, SavedNoise, SavedSighting};
use super::types::{
    MonsterAttackState, MonsterCheckAttack, MonsterContext, MonsterLocomotion, MonsterSoundTarget, SourceCombatMode,
};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2Mode, Q2TraceRequest};
use crate::q2::support::contracts::TraceHit;

/// Sighting record (`Sighting`).
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    /// Sighted actor.
    pub actor: ActorId,
    /// Sight time.
    pub time: f64,
}

/// Noise record (`Noise`).
#[derive(Debug, Clone, PartialEq)]
pub struct Noise {
    /// Noise actor.
    pub actor: ActorId,
    /// Noise time.
    pub time: f64,
    /// Noise owner.
    pub owner: ActorId,
    /// Noise origin.
    pub origin: Vec3,
}

/// Trail point (`TrailPoint`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoint {
    /// Point origin.
    pub origin: Vec3,
    /// Point time.
    pub time: f64,
    /// Point yaw.
    pub yaw: f64,
}

/// Noise pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NoisePair {
    /// Primary noise actor.
    pub primary: ActorId,
    /// Secondary noise actor.
    pub secondary: ActorId,
}

/// Monster perception state (`MonsterPerception` data).
///
/// The donor's class becomes arena state plus free functions; rules and
/// hooks resolve through the context so saves stay structural.
#[derive(Debug, Clone, Default)]
pub struct PerceptionRuntime {
    /// Current sight client.
    pub sight_client: Option<ActorId>,
    /// Last sight.
    pub sight: Option<Sighting>,
    /// Alerted actors.
    pub alerted: HashMap<ActorId, Sighting>,
    /// Primary noise.
    pub primary: Option<Noise>,
    /// Secondary noise.
    pub secondary: Option<Noise>,
    /// Noise pairs by owner.
    pub noises: HashMap<ActorId, NoisePair>,
    /// Player trails.
    pub trails: HashMap<ActorId, Vec<TrailPoint>>,
    /// Player origins.
    pub player_origins: HashMap<ActorId, Vec3>,
    /// Hostile timestamps.
    pub hostile: HashMap<ActorId, f64>,
    /// Last frame time.
    pub last_frame: f64,
}

impl PerceptionRuntime {
    /// Empty perception state.
    pub fn new() -> Self {
        Self {
            last_frame: f64::NEG_INFINITY,
            ..Self::default()
        }
    }

    /// Drop perception state after an actor release (`release`).
    pub fn release(&mut self, actor: &ActorId) {
        self.noises.remove(actor);
        self.alerted.remove(actor);
        self.trails.remove(actor);
        self.player_origins.remove(actor);
        self.hostile.remove(actor);
    }
}

/// Whether the monster faces its ideal yaw (`facingIdeal`).
pub fn facing_ideal(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let yaw = f64::from(context.game.body_of(actor).angles.y);
    let delta = ((yaw - context.state().ideal_yaw) % 360.0 + 360.0) % 360.0;
    delta <= 45.0 || delta >= 315.0
}

/// Attack chance profile (`Q2AttackChanceProfile`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2AttackChanceProfile {
    /// Stand-ground chance.
    pub stand_ground: f64,
    /// Melee chance.
    pub melee: f64,
    /// Near chance.
    pub near: f64,
    /// Mid chance.
    pub mid: f64,
    /// Far chance.
    pub far: f64,
    /// Strafe scalar.
    pub strafe_scalar: f64,
}

/// Normal attack chances (`normalAttackChances`).
pub const NORMAL_ATTACK_CHANCES: Q2AttackChanceProfile = Q2AttackChanceProfile {
    stand_ground: 0.7,
    melee: 0.4,
    near: 0.25,
    mid: 0.06,
    far: 0.0,
    strafe_scalar: 1.0,
};

/// Species attack check with the normal profile (`defaultCheckAttack`).
pub fn default_check_attack(context: &mut MonsterContext) -> bool {
    check_attack_with_profile(context, &NORMAL_ATTACK_CHANCES)
}

/// Species attack check (`defaultCheckAttack` with a profile).
///
/// The species-specific `M_CheckAttack` slot; the controller handles
/// turning and dispatch separately.
pub fn check_attack_with_profile(context: &mut MonsterContext, profile: &Q2AttackChanceProfile) -> bool {
    let Some(target) = enemy_eye(context) else { return false };
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let rogue = context.source_combat_rules() == SourceCombatMode::Rogue;
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let start = vec3(
        body.origin.x,
        body.origin.y,
        body.origin.z + context.entity().view_height as f32,
    );
    let enemy_actor = context.entity().enemy.clone();
    if health(context.game, enemy_actor.as_ref()) > 0.0 {
        let mask = attack_trace_mask(context.game);
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: target,
            bounds: None,
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        let hit_enemy = match &trace.hit {
            TraceHit::Actor { actor: hit } => {
                Some(hit) == enemy_actor.as_ref() || rerelease && context.game.host.is_player(hit)
            }
            _ => false,
        };
        if !hit_enemy {
            let solid = enemy_actor
                .as_ref()
                .and_then(|enemy| context.game.entity(enemy))
                .map(|target| target.solid);
            if !rerelease && !rogue
                || solid.is_some_and(|solid| solid != crate::q2::foundation::host::Q2Solid::None)
                || trace.fraction < 1.0
            {
                let blind_fire = context.state().blind_fire;
                let had_visibility = context.state().had_visibility;
                let blind_delay = context.state().blind_fire_delay;
                let attack_finished = context.state().attack_finished;
                let trail_time = context.state().trail_time;
                let now = context.game.host.now();
                let hit_monster = match &trace.hit {
                    TraceHit::Actor { actor: hit } => context.game.host.is_monster(hit),
                    _ => false,
                };
                if (rerelease || rogue)
                    && blind_fire
                    && (!rerelease || had_visibility)
                    && blind_delay <= 20.0
                    && !visible(context, None)
                    && !hit_monster
                    && now >= attack_finished
                    && now >= trail_time + blind_delay
                {
                    let blind_target = context.state().blind_fire_target;
                    let blind = context.game.host.trace(&Q2TraceRequest {
                        start,
                        end: blind_target,
                        bounds: None,
                        ignore: Some(actor.clone()),
                        mask: 0x2000000,
                        exclude: Vec::new(),
                    });
                    let blind_hit = blind.fraction == 1.0
                        || matches!(&blind.hit, TraceHit::Actor { actor: hit } if Some(hit) == enemy_actor.as_ref());
                    if !blind.all_solid && !blind.start_solid && blind_hit {
                        context.state_mut().attack_state = MonsterAttackState::Blind;
                        return true;
                    }
                }
                return false;
            }
        }
    }
    let distance = target_distance(context);
    if rerelease && distance <= 20.0 || !rerelease && distance < 80.0 {
        if !rerelease && context.game.options.skill == 0 && (context.game.random() * 4.0).floor() as i32 != 0 {
            if rogue {
                context.state_mut().attack_state = MonsterAttackState::Straight;
            }
            return false;
        }
        let melee = context.state().has_melee && (!rerelease || context.state().melee_time <= context.game.host.now());
        context.state_mut().attack_state = if melee {
            MonsterAttackState::Melee
        } else {
            MonsterAttackState::Missile
        };
        return true;
    }
    if rerelease
        && context.state().attack_state == MonsterAttackState::Melee
        && context.state().melee_time > context.game.host.now()
    {
        context.state_mut().attack_state = MonsterAttackState::Missile;
    }
    if !context.state().has_ranged_attack {
        if rerelease || rogue {
            context.state_mut().attack_state = MonsterAttackState::Straight;
        }
        return false;
    }
    if context.game.host.now() < context.state().attack_finished || !rerelease && distance >= 1000.0 {
        return false;
    }
    let mut chance = if rerelease {
        if context.state().stand_ground {
            profile.stand_ground
        } else if distance <= 20.0 {
            profile.melee
        } else if distance <= 440.0 {
            profile.near
        } else if distance <= 940.0 {
            profile.mid
        } else {
            profile.far
        }
    } else if context.state().stand_ground {
        0.4
    } else if distance < 500.0 {
        0.1
    } else if distance < 1000.0 {
        0.02
    } else {
        0.0
    };
    if !rerelease {
        chance *= if context.game.options.skill == 0 {
            0.5
        } else if context.game.options.skill >= 2 {
            2.0
        } else {
            1.0
        };
    }
    let enemy_actor = context.entity().enemy.clone();
    let non_solid_enemy = enemy_actor.as_ref().is_some_and(|enemy| {
        context
            .game
            .entity(enemy)
            .is_some_and(|target| target.solid == crate::q2::foundation::host::Q2Solid::None)
    });
    let hostile_enemy = enemy_actor
        .as_ref()
        .is_some_and(|enemy| !context.game.host.is_player(enemy));
    if rerelease && (hostile_enemy && non_solid_enemy || context.game.random() < chance)
        || !rerelease && (context.game.random() < chance || rogue && non_solid_enemy)
    {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        let finished = if rerelease {
            context.game.host.now()
        } else {
            context.game.host.now() + 2.0 * context.game.random()
        };
        context.state_mut().attack_finished = finished;
        return true;
    }
    if context.state().locomotion == MonsterLocomotion::Fly
        && (!rerelease || context.state().strafe_time <= context.game.host.now())
    {
        let enemy_class = enemy_actor
            .as_ref()
            .and_then(|enemy| context.game.entity(enemy))
            .map(|target| target.classname.clone());
        let mut strafe_chance = if rerelease || rogue {
            if context.entity().classname == "monster_daedalus" {
                0.8
            } else {
                0.6
            }
        } else {
            0.3
        };
        if (rerelease || rogue)
            && (enemy_class.as_deref() == Some("tesla") || rerelease && enemy_class.as_deref() == Some("tesla_mine"))
        {
            strafe_chance = 0.0;
        } else if rerelease {
            strafe_chance *= profile.strafe_scalar;
        }
        if rerelease && strafe_chance == 0.0 {
            return false;
        }
        let next = if context.game.random() < strafe_chance {
            MonsterAttackState::Sliding
        } else {
            MonsterAttackState::Straight
        };
        if rerelease && next != context.state().attack_state {
            let delay = context
                .game
                .host
                .rerelease_random()
                .expect("Rerelease monster strafe timing requires the shared source RNG")
                .time_milliseconds(1000, 3000) as f64
                / 1000.0;
            let strafe = context.game.host.now() + delay;
            context.state_mut().strafe_time = strafe;
        }
        context.state_mut().attack_state = next;
    } else if rerelease && context.state().locomotion != MonsterLocomotion::Fly && context.state().pathing.is_none() {
        context.state_mut().attack_state = MonsterAttackState::Straight;
    }
    false
}

/// Save an actor id.
fn saved(actor: &ActorId) -> SavedActorId {
    save_actor(Some(actor)).expect("live actor id")
}

/// Capture perception state (`capture`).
pub fn capture(game: &Q2GameServices) -> Q2MonsterPerceptionCheckpoint {
    let perception = &game.monsters.perception;
    let sight = perception.sight.as_ref().map(|value| SavedSighting {
        actor: saved(&value.actor),
        time: value.time,
    });
    let noise = |value: Option<&Noise>| {
        value.map(|value| SavedNoise {
            actor: saved(&value.actor),
            time: value.time,
            owner: saved(&value.owner),
            origin: value.origin,
        })
    };
    let mut alerted: Vec<(SavedActorId, SavedSighting)> = perception
        .alerted
        .iter()
        .map(|(id, value)| {
            (
                saved(id),
                SavedSighting {
                    actor: saved(&value.actor),
                    time: value.time,
                },
            )
        })
        .collect();
    alerted.sort_by_key(|(actor, _)| (actor.slot, actor.generation));
    let mut noises: Vec<(SavedActorId, SavedActorId, SavedActorId)> = perception
        .noises
        .iter()
        .map(|(id, pair)| (saved(id), saved(&pair.primary), saved(&pair.secondary)))
        .collect();
    noises.sort_by_key(|(actor, _, _)| (actor.slot, actor.generation));
    let mut trails: Vec<(SavedActorId, Vec<TrailPoint>)> = perception
        .trails
        .iter()
        .map(|(id, points)| (saved(id), points.clone()))
        .collect();
    trails.sort_by_key(|(actor, _)| (actor.slot, actor.generation));
    let mut player_origins: Vec<(SavedActorId, Vec3)> = perception
        .player_origins
        .iter()
        .map(|(id, origin)| (saved(id), *origin))
        .collect();
    player_origins.sort_by_key(|(actor, _)| (actor.slot, actor.generation));
    let mut hostile: Vec<(SavedActorId, f64)> =
        perception.hostile.iter().map(|(id, time)| (saved(id), *time)).collect();
    hostile.sort_by_key(|(actor, _)| (actor.slot, actor.generation));
    Q2MonsterPerceptionCheckpoint {
        sight_client: perception.sight_client.as_ref().map(saved),
        sight,
        alerted,
        primary: noise(perception.primary.as_ref()),
        secondary: noise(perception.secondary.as_ref()),
        noises,
        trails,
        player_origins,
        hostile,
        last_frame: if perception.last_frame.is_finite() {
            Some(perception.last_frame)
        } else {
            None
        },
    }
}

/// Restore perception state (`restore`).
pub fn restore(game: &mut Q2GameServices, checkpoint: &Q2MonsterPerceptionCheckpoint) {
    let sight_client = checkpoint.sight_client.map(|id| game.host.actors().reference_saved(id));
    let sight = checkpoint.sight.as_ref().map(|value| Sighting {
        actor: game.host.actors().reference_saved(value.actor),
        time: value.time,
    });
    let saved_noise = |game: &mut Q2GameServices, value: &SavedNoise| Noise {
        actor: game.host.actors().reference_saved(value.actor),
        time: value.time,
        owner: game.host.actors().reference_saved(value.owner),
        origin: value.origin,
    };
    let primary = checkpoint.primary.as_ref().map(|value| saved_noise(&mut *game, value));
    let secondary = checkpoint
        .secondary
        .as_ref()
        .map(|value| saved_noise(&mut *game, value));
    let perception = &mut game.monsters.perception;
    perception.sight_client = sight_client;
    perception.sight = sight;
    perception.primary = primary;
    perception.secondary = secondary;
    perception.alerted.clear();
    perception.noises.clear();
    perception.trails.clear();
    perception.player_origins.clear();
    perception.hostile.clear();
    // Actor ids resolve through the registry; the borrows end before
    // each insert.
    let alerted: Vec<(ActorId, Sighting)> = checkpoint
        .alerted
        .iter()
        .map(|(id, value)| {
            let game = &mut *game;
            (
                game.host.actors().reference_saved(*id),
                Sighting {
                    actor: game.host.actors().reference_saved(value.actor),
                    time: value.time,
                },
            )
        })
        .collect();
    for (id, value) in alerted {
        game.monsters.perception.alerted.insert(id, value);
    }
    let noises: Vec<(ActorId, NoisePair)> = checkpoint
        .noises
        .iter()
        .map(|(id, primary, secondary)| {
            let game = &mut *game;
            (
                game.host.actors().reference_saved(*id),
                NoisePair {
                    primary: game.host.actors().reference_saved(*primary),
                    secondary: game.host.actors().reference_saved(*secondary),
                },
            )
        })
        .collect();
    for (id, pair) in noises {
        game.monsters.perception.noises.insert(id, pair);
    }
    let trails: Vec<(ActorId, Vec<TrailPoint>)> = checkpoint
        .trails
        .iter()
        .map(|(id, points)| {
            let game = &mut *game;
            (game.host.actors().reference_saved(*id), points.clone())
        })
        .collect();
    for (id, points) in trails {
        game.monsters.perception.trails.insert(id, points);
    }
    let origins: Vec<(ActorId, Vec3)> = checkpoint
        .player_origins
        .iter()
        .map(|(id, origin)| {
            let game = &mut *game;
            (game.host.actors().reference_saved(*id), *origin)
        })
        .collect();
    for (id, origin) in origins {
        game.monsters.perception.player_origins.insert(id, origin);
    }
    let hostile: Vec<(ActorId, f64)> = checkpoint
        .hostile
        .iter()
        .map(|(id, time)| {
            let game = &mut *game;
            (game.host.actors().reference_saved(*id), *time)
        })
        .collect();
    for (id, time) in hostile {
        game.monsters.perception.hostile.insert(id, time);
    }
    game.monsters.perception.last_frame = checkpoint.last_frame.unwrap_or(f64::NEG_INFINITY);
}

/// Run the frame perception update (`beginFrame`).
pub fn begin_frame(game: &mut Q2GameServices) {
    if game.monsters.perception.last_frame == game.host.now() {
        return;
    }
    let now = game.host.now();
    game.monsters.perception.last_frame = now;
    let players = game.host.players();
    let rogue = game.monsters.source_combat == SourceCombatMode::Rogue;
    let sight_client = game.monsters.perception.sight_client.clone();
    let current = match sight_client {
        None => 0,
        Some(id) => players
            .iter()
            .position(|player| *player == id)
            .map(|index| index as i32)
            .unwrap_or(-1),
    };
    game.monsters.perception.sight_client = None;
    for i in 1..=players.len() {
        let candidate = players[((current + i as i32) % players.len() as i32) as usize].clone();
        let visible_flag = game.entity(&candidate).map(|target| target.flags).unwrap_or(0);
        if targetable(game, &candidate) && (rogue && (visible_flag & 0x8000) == 0 || !rogue) {
            game.monsters.perception.sight_client = Some(candidate);
            break;
        }
    }
    for player in players {
        let body = game.host.bodies().read(&player);
        let Some(body) = body else { continue };
        if health(game, Some(&player)) <= 0.0 {
            continue;
        }
        let trail = game
            .monsters
            .perception
            .trails
            .get(&player)
            .cloned()
            .unwrap_or_default();
        let previous = trail.last().copied();
        let observed = game.monster_target(Some(&player));
        let Some(observed) = observed else { continue };
        let eye = vec3(
            body.origin.x,
            body.origin.y,
            body.origin.z + observed.view_height as f32,
        );
        let stale = match previous {
            None => true,
            Some(previous) => {
                game.host
                    .trace(&Q2TraceRequest {
                        start: eye,
                        end: previous.origin,
                        bounds: None,
                        ignore: Some(player.clone()),
                        mask: MASK_OPAQUE,
                        exclude: Vec::new(),
                    })
                    .fraction
                    != 1.0
            }
        };
        if stale {
            let old_origin = game
                .monsters
                .perception
                .player_origins
                .get(&player)
                .copied()
                .unwrap_or(body.origin);
            let yaw = match previous {
                None => f64::from(body.angles.y),
                Some(previous) => f64::from(vector_angles(sub3(old_origin, previous.origin)).y),
            };
            let mut trail = trail;
            trail.push(TrailPoint {
                origin: old_origin,
                time: now,
                yaw,
            });
            if trail.len() > 8 {
                trail.remove(0);
            }
            game.monsters.perception.trails.insert(player.clone(), trail);
        }
        game.monsters.perception.player_origins.insert(player, body.origin);
    }
}

/// Report a noise (`reportNoise`).
pub fn report_noise(game: &mut Q2GameServices, actor: ActorId, origin: Vec3, secondary: bool) {
    if !game.host.actors().is_live(&actor)
        || game.options.mode == Q2Mode::Deathmatch
        || (game.entity(&actor).map(|target| target.flags).unwrap_or(0) & FL_NOTARGET as i64) != 0
    {
        return;
    }
    let pair = match game.monsters.perception.noises.get(&actor).cloned() {
        Some(pair) => pair,
        None => {
            let primary = game.create("player_noise", std::collections::BTreeMap::new());
            let other = game.create("player_noise", std::collections::BTreeMap::new());
            for noise in [primary.clone(), other.clone()] {
                game.require_entity_mut(&noise).owner = Some(actor.clone());
                game.require_entity_mut(&noise).visible = false;
                game.require_entity_mut(&noise).server_flags |= 1;
                let mut moved = game.body_of(noise.clone());
                moved.bounds = qa_core::math::Bounds {
                    min: vec3(-8.0, -8.0, -8.0),
                    max: vec3(8.0, 8.0, 8.0),
                };
                game.write_body(noise, &moved, false);
            }
            let pair = NoisePair {
                primary,
                secondary: other,
            };
            game.monsters.perception.noises.insert(actor.clone(), pair.clone());
            pair
        }
    };
    let noise_actor = if secondary { pair.secondary } else { pair.primary };
    if game.entity(&noise_actor).is_none() {
        return;
    }
    let mut moved = game.body_of(noise_actor.clone());
    moved.origin = origin;
    game.write_body(noise_actor.clone(), &moved, true);
    let record = Noise {
        actor: noise_actor,
        owner: actor,
        origin,
        time: game.host.now(),
    };
    if secondary {
        game.monsters.perception.secondary = Some(record);
    } else {
        game.monsters.perception.primary = Some(record);
    }
}

/// Whether an actor is targetable (`targetable`).
fn targetable(game: &mut Q2GameServices, actor: &ActorId) -> bool {
    let observed = game.monster_target(Some(actor));
    game.host.actors().is_live(actor)
        && health(game, Some(actor)) > 0.0
        && observed.is_some_and(|observed| !observed.notarget)
}

/// Whether an actor is a visual candidate (`visualCandidate`).
fn visual_candidate(context: &mut MonsterContext, actor: &ActorId) -> bool {
    let Some(_target) = context.game.host.bodies().read(actor) else {
        return false;
    };
    if !context.game.host.actors().is_live(actor) {
        return false;
    }
    let saved = context.entity().enemy.clone();
    context.entity_mut().enemy = Some(actor.clone());
    let distance = target_distance(context);
    context.entity_mut().enemy = saved;
    if context.game.options.edition == Q2Edition::Rerelease {
        if distance > 940.0 {
            return false;
        }
        let now = context.game.host.now();
        let hostile = context
            .game
            .monsters
            .perception
            .hostile
            .get(actor)
            .copied()
            .unwrap_or(-1.0);
        return distance <= 440.0 && hostile >= now && (context.entity().spawnflags & 1) == 0
            || visible(context, Some(actor)) && (distance <= 20.0 || in_front(context, actor));
    }
    let observed = context.game.monster_target(Some(actor));
    let Some(observed) = observed else { return false };
    if distance >= 1000.0 || observed.light_level.is_some_and(|light| light <= 5.0) || !visible(context, Some(actor)) {
        return false;
    }
    if distance >= 80.0 && distance < 500.0 {
        let shown_state = context.game.monsters.states.get(actor).map(|state| state.show_hostile);
        let shown = shown_state
            .or_else(|| context.game.monsters.perception.hostile.get(actor).copied())
            .unwrap_or(-1.0);
        if shown < context.game.host.now() && !in_front(context, actor) {
            return false;
        }
    }
    if distance >= 500.0 && !in_front(context, actor) {
        return false;
    }
    !context.state().good_guy
}

/// Find a target (`findTarget`).
pub fn find_target(context: &mut MonsterContext) -> bool {
    if context.state().good_guy || context.state().combat_point || context.state().dead {
        return false;
    }
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let now = context.game.host.now();
    let frame = if context.game.options.edition == Q2Edition::Classic {
        0.1
    } else {
        context.game.host.frame_seconds()
    };
    let recent_sight = |sight: Option<Sighting>| sight.is_some_and(|sight| sight.time >= now - frame);
    let mut candidate: Option<ActorId> = None;
    let mut noise: Option<Noise> = None;
    if rerelease {
        let players = context.game.host.players();
        let mut candidates = Vec::new();
        for actor in players {
            if !targetable(context.game, &actor) {
                continue;
            }
            let Some(target) = context.game.host.bodies().read(&actor) else {
                continue;
            };
            if close_enough(context, target.origin, 0.0, Some(&actor))
                || in_front(context, &actor) && visible(context, Some(&actor))
            {
                candidates.push(actor);
            }
        }
        if !candidates.is_empty() {
            let index = (context.game.random() * candidates.len() as f64).floor() as usize;
            candidate = candidates.get(index).cloned();
        }
        if candidate == context.entity().enemy && context.state().sound_target.is_none() {
            return false;
        }
        if candidate.is_none() && (context.entity().spawnflags & 1) == 0 {
            let alerted: Vec<Sighting> = context.game.monsters.perception.alerted.values().cloned().collect();
            // Donor map order is insertion order; slot order keeps the
            // scan deterministic.
            let mut alerted = alerted;
            alerted.sort_by_key(|sight| (sight.actor.slot(), sight.actor.generation()));
            for sight in alerted {
                if recent_sight(Some(sight.clone())) && visual_candidate(context, &sight.actor) {
                    candidate = Some(sight.actor);
                    break;
                }
            }
        }
    } else if recent_sight(context.game.monsters.perception.sight.clone()) && (context.entity().spawnflags & 1) == 0 {
        candidate = context.game.monsters.perception.sight.clone().map(|sight| sight.actor);
        let candidate_enemy = candidate
            .as_ref()
            .and_then(|actor| context.game.entity(actor))
            .and_then(|target| target.enemy.clone());
        if candidate_enemy == context.entity().enemy {
            return false;
        }
    }
    if candidate.is_none() {
        let primary = context.game.monsters.perception.primary.clone();
        let secondary = context.game.monsters.perception.secondary.clone();
        if recent_sight(primary.as_ref().map(|noise| Sighting {
            actor: noise.actor.clone(),
            time: noise.time,
        })) {
            noise = primary;
        } else if context.entity().enemy.is_none()
            && (context.entity().spawnflags & 1) == 0
            && recent_sight(secondary.as_ref().map(|noise| Sighting {
                actor: noise.actor.clone(),
                time: noise.time,
            }))
        {
            noise = secondary;
        }
        if let Some(noise) = noise.as_ref() {
            candidate = Some(noise.actor.clone());
        } else if context.game.options.edition == Q2Edition::Classic {
            candidate = context.game.monsters.perception.sight_client.clone();
        }
    }
    let Some(candidate) = candidate else { return false };
    if !context.game.host.actors().is_live(&candidate) {
        return false;
    }
    if context.state().hint_path && context.game.options.mode == Q2Mode::Coop {
        noise = None;
    }
    if Some(&candidate) == context.entity().enemy.as_ref()
        && !(rerelease && noise.is_some() && context.state().sound_target.is_some())
    {
        return true;
    }
    if let Some(noise) = noise {
        if (context
            .game
            .entity(&noise.owner)
            .map(|target| target.flags)
            .unwrap_or(0)
            & FL_NOTARGET as i64)
            != 0
        {
            return false;
        }
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        if (context.entity().spawnflags & 1) != 0 {
            if !visible(context, Some(&candidate)) {
                return false;
            }
        } else if !context.game.host.in_phs(origin, noise.origin) {
            return false;
        }
        if f64::from(length3(sub3(noise.origin, origin))) > 1000.0
            || !context.game.host.areas_connected(origin, noise.origin)
        {
            return false;
        }
        context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(noise.origin, origin)).y);
        if !context.state().manual_steering {
            change_yaw(context);
        }
        if context.state().temporary_stand_ground {
            context.state_mut().stand_ground = false;
            context.state_mut().temporary_stand_ground = false;
        }
        context.state_mut().sound_target = Some(MonsterSoundTarget {
            actor: noise.actor,
            owner: noise.owner,
            origin: noise.origin,
            time: noise.time,
        });
        context.entity_mut().enemy = Some(candidate);
    } else {
        if context.game.host.is_player(&candidate) {
            if !targetable(context.game, &candidate) {
                return false;
            }
        } else if context.game.host.is_monster(&candidate) {
            let candidate_enemy = context.game.entity(&candidate).and_then(|target| target.enemy.clone());
            let Some(candidate_enemy) = candidate_enemy else {
                return false;
            };
            if !targetable(context.game, &candidate_enemy) {
                return false;
            }
        } else {
            return false;
        }
        if !visual_candidate(context, &candidate) {
            return false;
        }
        context.state_mut().sound_target = None;
        if context.game.host.is_player(&candidate) {
            context.entity_mut().enemy = Some(candidate);
        } else {
            let candidate_enemy = context.game.entity(&candidate).and_then(|target| target.enemy.clone());
            context.entity_mut().enemy = candidate_enemy;
        }
        let enemy = context.entity().enemy.clone();
        let valid = enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy));
        if !valid {
            context.entity_mut().enemy = None;
            return false;
        }
    }
    if context.state().hint_path && context.game.monsters.hint_hooks.is_some() {
        let mut hooks = context.game.monsters.hint_hooks.take();
        if let Some(hooks) = hooks.as_mut() {
            hooks.stop(context);
        }
        context.game.monsters.hint_hooks = hooks;
        return true;
    }
    found_target(context);
    if context.state().sound_target.is_none() {
        if context.game.options.edition == Q2Edition::Classic || !context.state().close_sight_tripped {
            context.dispatch("$sight");
        }
        context.state_mut().close_sight_tripped = true;
    }
    true
}

/// Hunt the enemy (`huntTarget`).
pub fn hunt_target(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else { return };
    let enemy_actor = context.entity().enemy.clone();
    context.entity_mut().goal = enemy_actor;
    if context.state().stand_ground {
        context.stand();
    } else {
        context.run();
    }
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor).origin;
    context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(enemy.origin, origin)).y);
    if context.game.options.edition == Q2Edition::Classic && !context.state().stand_ground {
        let finished = context.game.host.now() + 1.0;
        context.state_mut().attack_finished = finished;
    }
}

/// Record a found target (`foundTarget`).
pub fn found_target(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else { return };
    let enemy_actor = context.entity().enemy.clone();
    if enemy_actor
        .as_ref()
        .is_some_and(|enemy| context.game.host.is_player(enemy))
    {
        let enemy_actor = enemy_actor.clone().expect("player enemy");
        if context.source_combat_rules() == SourceCombatMode::Rogue {
            context.game.require_entity_mut(&enemy_actor).flags &= !0x8000;
        }
        let actor = context.actor().clone();
        let record = Sighting {
            actor,
            time: context.game.host.now(),
        };
        context.game.monsters.perception.sight = Some(record.clone());
        context
            .game
            .monsters
            .perception
            .alerted
            .insert(enemy_actor.clone(), record);
        let hostile = context.game.host.now() + 1.0;
        context.game.monsters.perception.hostile.insert(enemy_actor, hostile);
    }
    let hostile = context.game.host.now() + 1.0;
    context.state_mut().show_hostile = hostile;
    if context.game.options.edition == Q2Edition::Rerelease {
        if context.state().trail_time == 0.0 {
            let finished = context.game.host.now() + 0.6;
            context.state_mut().attack_finished = finished;
        }
        let skill_bonus = if context.game.options.skill == 0 {
            0.4
        } else if context.game.options.skill == 1 {
            0.2
        } else {
            0.0
        };
        context.state_mut().attack_finished += skill_bonus;
        context.state_mut().saved_goal = Some(enemy.origin);
        context.state_mut().blind_fire_target = add3(enemy.origin, scale3(enemy.velocity, -0.1));
        context.state_mut().blind_fire_delay = 0.0;
    }
    context.state_mut().last_sighting = enemy.origin;
    let now = context.game.host.now();
    context.state_mut().trail_time = now;
    if context.source_combat_rules() == SourceCombatMode::Rogue && context.game.options.edition == Q2Edition::Classic {
        context.state_mut().blind_fire_target = enemy.origin;
    }
    let actor = context.actor().clone();
    let mut mission = context.mission(&actor);
    if let Some(mission) = mission.as_mut() {
        mission.found_target();
        let route = mission.combat_route();
        if let Some(goal) = route.goal {
            context.state_mut().combat_point = true;
            context.state_mut().move_target = Some(goal.clone());
            context.entity_mut().goal = Some(goal);
            context.state_mut().pause_time = 0.0;
            context.run();
            return;
        }
    }
    if context.state().combat_point {
        return;
    }
    if context.state().combat_target.is_empty() {
        hunt_target(context);
        return;
    }
    let combat_target = context.state().combat_target.clone();
    let target = context.game.pick_target(&combat_target);
    let Some(target) = target else {
        let classname = context.entity().classname.clone();
        context
            .game
            .host
            .diagnostic(&format!("{classname}: combattarget {combat_target} not found"));
        hunt_target(context);
        return;
    };
    context.state_mut().combat_target = String::new();
    context.entity_mut().combat_target = String::new();
    context.state_mut().combat_point = true;
    context.state_mut().move_target = Some(target.clone());
    context.entity_mut().goal = Some(target.clone());
    if context.game.options.edition == Q2Edition::Classic {
        context.game.require_entity_mut(&target).targetname = String::new();
    }
    context.state_mut().pause_time = 0.0;
    context.run();
}

/// React to damage (`reactToDamage`).
pub fn react_to_damage(context: &mut MonsterContext, attacker: Option<&ActorId>) {
    let Some(attacker) = attacker.cloned() else { return };
    if !context.game.host.is_player(&attacker) && !context.game.host.is_monster(&attacker) {
        return;
    }
    let mut hooks = context.game.monsters.source_combat_hooks.take();
    let handled = hooks
        .as_mut()
        .map(|hooks| hooks.before_react(context, &attacker))
        .unwrap_or(false);
    context.game.monsters.source_combat_hooks = hooks;
    if handled {
        return;
    }
    let actor = context.actor().clone();
    if attacker == actor || Some(&attacker) == context.entity().enemy.as_ref() {
        return;
    }
    let other = context.game.entity(&attacker).cloned();
    let rogue = context.source_combat_rules() == SourceCombatMode::Rogue;
    if context.state().good_guy
        && (context.game.host.is_player(&attacker)
            || context
                .game
                .monsters
                .states
                .get(&attacker)
                .is_some_and(|state| state.good_guy)
            || other.as_ref().is_some_and(|_| {
                let mut hooks = context.game.monsters.source_combat_hooks.take();
                let good = other.as_ref().is_some_and(|entity| {
                    hooks
                        .as_mut()
                        .map(|hooks| hooks.is_good_guy(&mut *context.game, entity))
                        .unwrap_or(false)
                });
                context.game.monsters.source_combat_hooks = hooks;
                good
            }))
    {
        return;
    }
    if context.game.host.is_player(&attacker) {
        context.state_mut().sound_target = None;
        let current_enemy = context.entity().enemy.clone();
        let enemy_is_player = current_enemy
            .as_ref()
            .is_some_and(|enemy| context.game.host.is_player(enemy));
        if enemy_is_player {
            if visible(context, None) {
                context.state_mut().old_enemy = Some(attacker.clone());
                return;
            }
            let old = context.entity().enemy.clone();
            context.state_mut().old_enemy = old;
        }
        context.entity_mut().enemy = Some(attacker);
    } else if let Some(other) = other {
        let ignore_shots = if rogue || context.game.options.edition == Q2Edition::Rerelease {
            context.state().ignore_shots
                || context
                    .game
                    .monsters
                    .states
                    .get(&attacker)
                    .is_some_and(|state| state.ignore_shots)
        } else {
            ["monster_tank", "monster_supertank", "monster_makron", "monster_jorg"].contains(&other.classname.as_str())
        };
        let retaliate = (context.entity().flags & 3) == (other.flags & 3)
            && context.entity().classname != other.classname
            && !ignore_shots
            || other.enemy == Some(actor.clone());
        let current_enemy = context.entity().enemy.clone();
        if current_enemy
            .as_ref()
            .is_some_and(|enemy| context.game.host.is_player(enemy))
        {
            context.state_mut().old_enemy = current_enemy;
        }
        if retaliate {
            context.entity_mut().enemy = Some(attacker);
        } else if other.enemy.is_some() && other.enemy != Some(actor) {
            context.entity_mut().enemy = other.enemy;
        } else {
            return;
        }
    } else {
        context.entity_mut().enemy = Some(attacker);
    }
    if !context.state().ducked {
        found_target(context);
    }
}

/// Check for an attack (`checkAttack`).
pub fn check_attack(context: &mut MonsterContext, check: MonsterCheckAttack) -> bool {
    if context.state().combat_point {
        return false;
    }
    if context.state().sound_target.is_some() {
        let target = context.state().sound_target.clone().expect("sound target");
        let primary = context.game.monsters.perception.primary.clone();
        let secondary = context.game.monsters.perception.secondary.clone();
        let fresh = if primary.as_ref().is_some_and(|noise| noise.actor == target.actor) {
            primary.map(|noise| MonsterSoundTarget {
                actor: noise.actor,
                owner: noise.owner,
                origin: noise.origin,
                time: noise.time,
            })
        } else if secondary.as_ref().is_some_and(|noise| noise.actor == target.actor) {
            secondary.map(|noise| MonsterSoundTarget {
                actor: noise.actor,
                owner: noise.owner,
                origin: noise.origin,
                time: noise.time,
            })
        } else {
            Some(target)
        };
        let fresh = fresh.expect("sound target");
        context.state_mut().sound_target = Some(fresh.clone());
        if context.game.host.now() - fresh.time <= 5.0 {
            let hostile = context.game.host.now() + 1.0;
            context.state_mut().show_hostile = hostile;
            return false;
        }
        if context.entity().goal == context.entity().enemy {
            let move_target = context.state().move_target.clone();
            context.entity_mut().goal = move_target;
        }
        context.state_mut().sound_target = None;
        if context.state().temporary_stand_ground {
            context.state_mut().stand_ground = false;
            context.state_mut().temporary_stand_ground = false;
        }
    }
    let mut enemy = enemy_body(context);
    let enemy_actor = context.entity().enemy.clone();
    let enemy_health = health(context.game, enemy_actor.as_ref());
    let lost = if context.state().medic {
        enemy_health > 0.0
    } else if context.state().brutal {
        context.game.options.edition == Q2Edition::Classic && enemy_health <= -80.0
    } else {
        enemy_health <= 0.0
    };
    if enemy.is_none() || lost {
        if context.source_combat_rules() == SourceCombatMode::Rogue {
            context.state_mut().medic = false;
        }
        context.entity_mut().enemy = None;
        context.state_mut().close_sight_tripped = false;
        if context.game.options.edition == Q2Edition::Rerelease {
            context.entity_mut().goal = None;
        }
        let old_enemy = context.state().old_enemy.clone();
        if old_enemy
            .as_ref()
            .is_some_and(|old| health(context.game, Some(old)) > 0.0)
        {
            context.entity_mut().enemy = old_enemy;
            context.state_mut().old_enemy = None;
            hunt_target(context);
            enemy = enemy_body(context);
        } else if context.source_combat_rules() == SourceCombatMode::Rogue
            && context.game.monsters.source_combat_hooks.is_some()
        {
            let mut hooks = context.game.monsters.source_combat_hooks.take();
            let recovered = hooks.as_mut().map(|hooks| hooks.recover_enemy(context)).unwrap_or(None);
            context.game.monsters.source_combat_hooks = hooks;
            context.entity_mut().enemy = recovered;
            if context.entity().enemy.is_some() {
                context.state_mut().old_enemy = None;
                hunt_target(context);
                enemy = enemy_body(context);
            } else {
                if context.state().move_target.is_some() {
                    let move_target = context.state().move_target.clone();
                    context.entity_mut().goal = move_target;
                    context.walk();
                } else {
                    let pause = context.game.host.now() + 100000000.0;
                    context.state_mut().pause_time = pause;
                    context.stand();
                }
                return true;
            }
        } else {
            if context.state().move_target.is_some()
                && (context.game.options.edition == Q2Edition::Classic || !context.state().stand_ground)
            {
                let move_target = context.state().move_target.clone();
                context.entity_mut().goal = move_target;
                context.walk();
            } else {
                let pause = context.game.host.now() + 100000000.0;
                context.state_mut().pause_time = pause;
                context.stand();
            }
            return true;
        }
    }
    let Some(enemy) = enemy else { return false };
    let enemy_visible = visible(context, None);
    if enemy_visible {
        let search = context.game.host.now() + 5.0;
        context.state_mut().search_time = search;
        context.state_mut().last_sighting = enemy.origin;
        if context.source_combat_rules() == SourceCombatMode::Rogue
            && context.game.options.edition == Q2Edition::Classic
        {
            context.state_mut().lost_sight = false;
            let now = context.game.host.now();
            context.state_mut().trail_time = now;
            context.state_mut().blind_fire_target = enemy.origin;
            context.state_mut().blind_fire_delay = 0.0;
        }
        if context.game.options.edition == Q2Edition::Rerelease {
            context.state_mut().had_visibility = true;
            context.state_mut().lost_sight = false;
            context.state_mut().saved_goal = Some(enemy.origin);
            let now = context.game.host.now();
            context.state_mut().trail_time = now;
            context.state_mut().blind_fire_target = add3(enemy.origin, scale3(enemy.velocity, -0.1));
            context.state_mut().blind_fire_delay = 0.0;
            if let Some(enemy_actor) = context.entity().enemy.clone() {
                let hostile = context.game.host.now() + 1.0;
                context.game.monsters.perception.hostile.insert(enemy_actor, hostile);
            }
        }
    }
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    if !rerelease && context.source_combat_rules() == SourceCombatMode::Rogue {
        if !check(context) {
            return false;
        }
        if matches!(
            context.state().attack_state,
            MonsterAttackState::Missile | MonsterAttackState::Melee | MonsterAttackState::Blind
        ) {
            let actor = context.actor().clone();
            let origin = context.game.body_of(actor).origin;
            context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(enemy.origin, origin)).y);
            if !context.state().manual_steering {
                change_yaw(context);
            }
            if facing_ideal(context) {
                if context.state().attack_state == MonsterAttackState::Melee {
                    context.melee();
                    context.state_mut().attack_state = MonsterAttackState::Straight;
                } else {
                    context.attack();
                    if matches!(
                        context.state().attack_state,
                        MonsterAttackState::Missile | MonsterAttackState::Blind
                    ) {
                        context.state_mut().attack_state = MonsterAttackState::Straight;
                    }
                }
            }
            return true;
        }
        return enemy_visible;
    }
    let mut selected = false;
    if rerelease && context.state().check_attack_time <= context.game.host.now() {
        let throttle = context.game.host.now() + 0.1;
        context.state_mut().check_attack_time = throttle;
        selected = check(context);
    }
    if matches!(
        context.state().attack_state,
        MonsterAttackState::Missile | MonsterAttackState::Melee
    ) || rerelease && context.state().attack_state == MonsterAttackState::Blind
    {
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(enemy.origin, origin)).y);
        if !context.state().manual_steering {
            change_yaw(context);
        }
        if facing_ideal(context) {
            if context.state().attack_state == MonsterAttackState::Melee {
                context.melee();
            } else {
                context.attack();
                if rerelease {
                    let finished = context.game.host.now() + 1.0 + context.game.random();
                    context.state_mut().attack_finished = finished;
                }
            }
            if !rerelease
                || matches!(
                    context.state().attack_state,
                    MonsterAttackState::Missile | MonsterAttackState::Blind | MonsterAttackState::Melee
                )
            {
                context.state_mut().attack_state = MonsterAttackState::Straight;
            }
        }
        return true;
    }
    if rerelease {
        return selected;
    }
    enemy_visible && check(context)
}

/// Move toward the goal (`moveToGoal`).
pub fn move_to_goal(context: &mut MonsterContext, distance: f64) -> bool {
    let actor = context.actor().clone();
    if context.state().locomotion == MonsterLocomotion::Stationary
        || context.game.body_of(actor).ground.is_none() && context.state().locomotion == MonsterLocomotion::Walk
    {
        return false;
    }
    let enemy = enemy_body(context);
    let goal_actor = context.entity().goal.clone();
    let mut goal = goal_actor
        .as_ref()
        .and_then(|goal| context.game.host.bodies().read(goal))
        .map(|body| body.origin);
    let tracking = !context.state().hint_path
        && !context.state().combat_point
        && context.state().sound_target.is_none()
        && enemy.is_some();
    if tracking && !visible(context, None) {
        goal = Some(pursuit_goal(context, distance));
    } else if tracking {
        context.state_mut().lost_sight = false;
        context.state_mut().last_sighting = enemy.as_ref().map(|body| body.origin).unwrap_or(vec3(0.0, 0.0, 0.0));
        let now = context.game.host.now();
        context.state_mut().trail_time = now;
    }
    let Some(goal) = goal else { return false };
    if tracking {
        let enemy_origin = enemy.map(|body| body.origin);
        let enemy_actor = context.entity().enemy.clone();
        if enemy_origin.is_some_and(|origin| close_enough(context, origin, distance, enemy_actor.as_ref())) {
            return true;
        }
    }
    if ((context.game.random() * 4.0).floor() as i32 != 1
        || context.source_combat_rules() == SourceCombatMode::Rogue && context.state().charging)
        && step_direction(context, context.state().ideal_yaw, distance)
    {
        return true;
    }
    if context.consume_source_blocked() {
        return false;
    }
    let actor = context.actor().clone();
    if !context.game.host.actors().is_live(&actor) {
        return false;
    }
    if context.game.options.edition == Q2Edition::Rerelease
        && context.source_combat_rules() != SourceCombatMode::Rogue
        && context.blocked(distance)
    {
        return true;
    }
    chase_direction(context, goal, distance)
}

/// Whether an origin is within reach (`closeEnough`).
fn close_enough(context: &mut MonsterContext, origin: Vec3, distance: f64, actor: Option<&ActorId>) -> bool {
    let self_actor = context.actor().clone();
    let body = context.game.body_of(self_actor);
    let target = actor.and_then(|actor| context.game.host.bodies().read(actor));
    let bounds = target.map(|body| body.bounds).unwrap_or(qa_core::math::Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    });
    let distance = distance as f32;
    origin.x + bounds.min.x <= body.origin.x + body.bounds.max.x + distance
        && origin.x + bounds.max.x >= body.origin.x + body.bounds.min.x - distance
        && origin.y + bounds.min.y <= body.origin.y + body.bounds.max.y + distance
        && origin.y + bounds.max.y >= body.origin.y + body.bounds.min.y - distance
        && origin.z + bounds.min.z <= body.origin.z + body.bounds.max.z + distance
        && origin.z + bounds.max.z >= body.origin.z + body.bounds.min.z - distance
}

/// Compute the pursuit goal (`pursuitGoal`).
fn pursuit_goal(context: &mut MonsterContext, distance: f64) -> Vec3 {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let mut new_goal = false;
    if !context.state().lost_sight {
        context.state_mut().lost_sight = true;
        context.state_mut().pursuit_last_seen = true;
        context.state_mut().pursue_next = false;
        context.state_mut().pursue_temporary = false;
        new_goal = true;
    }
    if context.state().pursue_next {
        context.state_mut().pursue_next = false;
        let search = context.game.host.now() + 5.0;
        context.state_mut().search_time = search;
        if context.state().pursue_temporary && context.state().saved_goal.is_some() {
            context.state_mut().pursue_temporary = false;
            let saved = context.state().saved_goal.unwrap_or(vec3(0.0, 0.0, 0.0));
            context.state_mut().last_sighting = saved;
            new_goal = true;
        } else {
            let enemy_actor = context.entity().enemy.clone();
            let trail = enemy_actor
                .as_ref()
                .and_then(|enemy| context.game.monsters.perception.trails.get(enemy))
                .cloned()
                .unwrap_or_default();
            let mut marker = trail
                .iter()
                .find(|point| point.time > context.state().trail_time)
                .copied();
            if context.state().pursuit_last_seen && marker.is_some() {
                let index = marker.and_then(|marker| trail.iter().position(|point| *point == marker));
                let prior = index
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| trail.get(index))
                    .copied();
                if let Some(marker_point) = marker {
                    let trace = context.game.host.trace(&Q2TraceRequest {
                        start: body.origin,
                        end: marker_point.origin,
                        bounds: None,
                        ignore: Some(actor.clone()),
                        mask: MASK_OPAQUE,
                        exclude: Vec::new(),
                    });
                    if trace.fraction != 1.0 {
                        if let Some(prior) = prior {
                            let clear = context
                                .game
                                .host
                                .trace(&Q2TraceRequest {
                                    start: body.origin,
                                    end: prior.origin,
                                    bounds: None,
                                    ignore: Some(actor.clone()),
                                    mask: MASK_OPAQUE,
                                    exclude: Vec::new(),
                                })
                                .fraction
                                == 1.0;
                            if clear {
                                marker = Some(prior);
                            }
                        }
                    }
                }
            }
            context.state_mut().pursuit_last_seen = false;
            if let Some(marker) = marker {
                context.state_mut().last_sighting = marker.origin;
                context.state_mut().trail_time = marker.time;
                context.state_mut().ideal_yaw = marker.yaw;
                let mut moved = context.game.body_of(actor.clone());
                moved.angles.y = marker.yaw as f32;
                context.game.write_body(actor.clone(), &moved, false);
                new_goal = true;
            }
        }
    }
    let d1 = f64::from(length3(sub3(context.state().last_sighting, body.origin)));
    if d1 <= distance {
        context.state_mut().pursue_next = true;
    }
    if new_goal && d1 > 0.0 {
        let last_sighting = context.state().last_sighting;
        let mask = monster_solid_mask(context.game);
        let center = context.game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end: last_sighting,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        if center.fraction < 1.0 {
            let d2 = d1 * (center.fraction + 1.0) * 0.5;
            context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(last_sighting, body.origin)).y);
            let ideal_yaw = context.state().ideal_yaw;
            let basis = angles_vectors(vec3(body.angles.x, ideal_yaw as f32, body.angles.z));
            let make = |forward: f64, right: f64| {
                add3(
                    body.origin,
                    add3(scale3(basis.forward, forward as f32), scale3(basis.right, right as f32)),
                )
            };
            let mask = monster_solid_mask(context.game);
            let left = context.game.host.trace(&Q2TraceRequest {
                start: body.origin,
                end: make(d2, -16.0),
                bounds: Some(body.bounds),
                ignore: Some(actor.clone()),
                mask,
                exclude: Vec::new(),
            });
            let mask = monster_solid_mask(context.game);
            let right = context.game.host.trace(&Q2TraceRequest {
                start: body.origin,
                end: make(d2, 16.0),
                bounds: Some(body.bounds),
                ignore: Some(actor.clone()),
                mask,
                exclude: Vec::new(),
            });
            let center_fraction = d1 * center.fraction / d2;
            let side = if left.fraction >= center_fraction && left.fraction > right.fraction {
                -16.0
            } else if right.fraction >= center_fraction && right.fraction > left.fraction {
                16.0
            } else {
                0.0
            };
            if side != 0.0 {
                let fraction = if side < 0.0 { left.fraction } else { right.fraction };
                context.state_mut().saved_goal = Some(last_sighting);
                context.state_mut().pursue_temporary = true;
                context.state_mut().last_sighting = make(if fraction < 1.0 { d2 * fraction * 0.5 } else { d2 }, side);
                let last_sighting = context.state().last_sighting;
                context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(last_sighting, body.origin)).y);
            }
            let ideal_yaw = context.state().ideal_yaw;
            let mut moved = context.game.body_of(actor.clone());
            moved.angles.y = ideal_yaw as f32;
            context.game.write_body(actor, &moved, false);
        }
    }
    context.state().last_sighting
}
