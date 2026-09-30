//! Deathball mode (`src/content/q2/missionpacks/modes/deathball.ts`).
//!
//! Original Rogue dm_ball.c (GPL-2.0-or-later).

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Vec3, add3, dot3, length3, normalize3, scale3, sub3, vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::movedir;
use crate::q2::foundation::host::{
    Q2Die, Q2EffectEvent, Q2GameServices, Q2Mode, Q2MotionKind, Q2Pain, Q2PresentationEvent,
    Q2PrintLevel, Q2Solid, Q2SpawnFn, Q2Think, Q2Touch, SpawnModule,
};
use crate::q2::foundation::monsters::ai::monster_solid_mask;
use crate::q2::foundation::scenery::kill_q2_box;
use crate::q2::support::contracts::{CombatState, DeathReaction, PainReaction, TouchContact};

/// Deathball settings (`Q2DeathBallHooks::settings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2DeathBallSettings {
    /// Team 1 skin.
    pub team1_skin: String,
    /// Team 2 skin.
    pub team2_skin: String,
    /// Goal limit.
    pub goal_limit: i32,
}

/// Deathball rules (`q2DeathBallRules`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2DeathBallRules {
    /// Stop speed.
    pub stop_speed: f64,
    /// Deathmatch flags.
    pub deathmatch_flags: i32,
}

/// Deathball rules for flags (`q2DeathBallRules`).
pub fn q2_deathball_rules(deathmatch_flags: i32) -> Q2DeathBallRules {
    Q2DeathBallRules {
        stop_speed: 0.0,
        deathmatch_flags: deathmatch_flags | 0x20000 | 0x80000 | 0x40000 | 256 | 64,
    }
}

/// Deathball hooks (`Q2DeathBallHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2DeathBallHooks {
    /// Read settings.
    pub settings: fn() -> Q2DeathBallSettings,
    /// Read a player skin.
    pub skin: fn(ActorId) -> String,
    /// Set a player skin.
    pub set_skin: fn(ActorId, String),
    /// Add score.
    pub add_score: fn(ActorId, f64),
    /// End the level.
    pub end_level: fn(),
    /// Select a spawn placement.
    pub select_spawn: fn(ActorId, &mut Q2GameServices) -> (Vec3, Vec3),
    /// Spawn distance for a spot.
    pub spawn_distance: fn(ActorId, &mut Q2GameServices) -> f64,
}

/// Deathball checkpoint (`Q2DeathBallCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2DeathBallCheckpoint {
    /// Ball actor.
    pub ball: Option<SavedActorId>,
    /// Start count.
    pub starts: i32,
    /// Team 1 score.
    pub team1_score: f64,
    /// Team 2 score.
    pub team2_score: f64,
}

/// Arena runtime state for deathball.
#[derive(Debug, Clone)]
pub struct DeathballRuntime {
    /// Session hooks.
    pub hooks: Option<Q2DeathBallHooks>,
    /// Ball actor.
    pub ball: Option<ActorId>,
    /// Start count.
    pub starts: i32,
    /// Team 1 score.
    pub team1_score: f64,
    /// Team 2 score.
    pub team2_score: f64,
}

impl Default for DeathballRuntime {
    fn default() -> Self {
        Self {
            hooks: None,
            ball: None,
            starts: 0,
            team1_score: 0.0,
            team2_score: 0.0,
        }
    }
}

/// Deathball callbacks (`Q2DeathBall::callbacks`).
pub fn deathball_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("DBall_BallRespawn", dball_respawn as Q2Think);
    callbacks.touch.insert("DBall_BallTouch", dball_ball_touch as Q2Touch);
    callbacks.touch.insert("DBall_GoalTouch", dball_goal_touch as Q2Touch);
    callbacks.touch.insert("DBall_SpeedTouch", dball_speed_touch as Q2Touch);
    callbacks.pain.insert("DBall_BallPain", dball_ball_pain as Q2Pain);
    callbacks.die.insert("DBall_BallDie", dball_ball_die as Q2Die);
    callbacks
}

/// Deathball mode (`Q2DeathBall`).
#[derive(Debug, Clone, Copy)]
pub struct Q2DeathBall {
    /// Session hooks.
    pub hooks: Q2DeathBallHooks,
}

/// Session deathball hooks.
pub fn deathball_hooks(game: &Q2GameServices) -> Q2DeathBallHooks {
    game.deathball.hooks.expect("Q2 deathball mode is not registered")
}

impl Q2DeathBall {
    /// Register deathball and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices) -> SpawnModule {
        game.deathball.hooks = Some(self.hooks);
        SpawnModule {
            spawn: deathball_spawn as Q2SpawnFn,
            item_name: |_| None,
            callbacks: deathball_callbacks(),
        }
    }

    /// Spawn a deathball entity (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        if ![
            "dm_dball_ball",
            "dm_dball_goal",
            "dm_dball_speed_change",
            "dm_dball_ball_start",
            "dm_dball_team1_start",
            "dm_dball_team2_start",
        ]
        .contains(&classname.as_str())
        {
            return false;
        }
        if game.options.mode != Q2Mode::Deathmatch {
            game.remove_actor(entity);
            return true;
        }
        game.source_callbacks.register(&deathball_callbacks());
        if classname == "dm_dball_ball" {
            game.deathball.ball = Some(entity.clone());
            let mask = monster_solid_mask(game);
            {
                let record = game.require_entity_mut(&entity);
                record.model = "models/objects/dball/tris.md2".to_string();
                record.max_health = 50000.0;
                record.clip_mask = mask;
                record.pain = Some(dball_ball_pain as Q2Pain);
                record.die = Some(dball_ball_die as Q2Die);
                record.touch = Some(dball_ball_touch as Q2Touch);
            }
            let owned = game.owned_of(entity.clone());
            game.host.combat().create(
                &owned,
                &CombatState {
                    health: 50000.0,
                    armor: ArmorState {
                        regular: RegularArmorState::None,
                        powered: PoweredProtectionState::None,
                    },
                    mass: 50.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: false,
                    team: None,
                },
            );
            let mut moved = game.body_of(entity.clone());
            moved.bounds.min = vec3(-32.0, -32.0, -32.0);
            moved.bounds.max = vec3(32.0, 32.0, 32.0);
            game.write_body(entity.clone(), &moved, true);
            game.set_solid(entity.clone(), Q2Solid::Box);
            game.set_motion_kind(entity.clone(), Q2MotionKind::NewToss);
            game.show(entity);
        } else if classname == "dm_dball_goal" || classname == "dm_dball_speed_change" {
            let speed = classname == "dm_dball_speed_change";
            game.require_entity_mut(&entity).touch = Some(
                if speed {
                    dball_speed_touch as Q2Touch
                } else {
                    dball_goal_touch as Q2Touch
                },
            );
            game.require_entity_mut(&entity).visible = false;
            if speed {
                if game.require_entity(&entity).speed == 0.0 {
                    game.require_entity_mut(&entity).speed = 2.0;
                }
                if game.require_entity(&entity).delay == 0.0 {
                    game.require_entity_mut(&entity).delay = 0.2;
                }
            } else if game.require_entity(&entity).wait == 0.0 {
                game.require_entity_mut(&entity).wait = 10.0;
            }
            let angles = game.body_of(entity.clone()).angles;
            game.require_entity_mut(&entity).movedir = movedir(angles);
            let mut moved = game.body_of(entity.clone());
            moved.angles = Vec3::default();
            game.write_body(entity.clone(), &moved, true);
            game.set_solid(entity.clone(), Q2Solid::Trigger);
            game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
            game.show(entity);
        }
        true
    }

    /// Capture deathball state (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2DeathBallCheckpoint {
        let _ = self;
        Q2DeathBallCheckpoint {
            ball: game.deathball.ball.as_ref().map(SavedActorId::from),
            starts: game.deathball.starts,
            team1_score: game.deathball.team1_score,
            team2_score: game.deathball.team2_score,
        }
    }

    /// Restore deathball state (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, saved: Q2DeathBallCheckpoint) {
        let _ = self;
        game.deathball.ball = saved.ball.map(|saved| {
            game.host.actors().resolve_saved(saved).map(|owned| owned.id().clone()).unwrap_or_else(|| game.host.actors().reference_saved(saved))
        });
        game.deathball.starts = saved.starts;
        game.deathball.team1_score = saved.team1_score;
        game.deathball.team2_score = saved.team2_score;
    }

    /// Ball actor (`ballActor`).
    pub fn ball_actor(&self, game: &Q2GameServices) -> Option<ActorId> {
        let _ = self;
        game.deathball.ball.clone()
    }

    /// Team scores (`scores`).
    pub fn scores(&self, game: &Q2GameServices) -> (f64, f64) {
        let _ = self;
        (game.deathball.team1_score, game.deathball.team2_score)
    }

    /// Check the goal limit (`checkRules`).
    pub fn check_rules(&self, game: &mut Q2GameServices) -> bool {
        let limit = (self.hooks.settings)().goal_limit;
        if limit == 0 {
            return false;
        }
        let winner = if game.deathball.team1_score >= f64::from(limit) {
            Some(1)
        } else if game.deathball.team2_score >= f64::from(limit) {
            Some(2)
        } else {
            None
        };
        let Some(winner) = winner else {
            return false;
        };
        game.host.emit(Q2PresentationEvent::Print {
            actor: None,
            level: Q2PrintLevel::High,
            text: format!("Team {winner} Wins.\n"),
        });
        (self.hooks.end_level)();
        true
    }

    /// Assign a team on client begin (`clientBegin`).
    pub fn client_begin(&self, entity: &ActorId, game: &mut Q2GameServices) {
        let settings = (self.hooks.settings)();
        let mut one = 0;
        let mut two = 0;
        let mut unassigned = 0;
        for actor in game.host.players() {
            if &actor == entity {
                continue;
            }
            let skin = (self.hooks.skin)(actor);
            if skin.contains('/') && skin == settings.team1_skin {
                one += 1;
            } else if skin.contains('/') && skin == settings.team2_skin {
                two += 1;
            } else {
                unassigned += 1;
            }
        }
        (self.hooks.set_skin)(
            entity.clone(),
            if one > two {
                settings.team2_skin
            } else {
                settings.team1_skin
            },
        );
        if unassigned != 0 {
            game.host.diagnostic(&format!("{unassigned} unassigned players present!"));
        }
    }

    /// Select a team spawn (`selectSpawn`).
    pub fn select_spawn(&self, entity: &ActorId, game: &mut Q2GameServices) -> (Vec3, Vec3) {
        let skin = (self.hooks.skin)(entity.clone());
        let settings = (self.hooks.settings)();
        let classname = if skin == settings.team1_skin {
            "dm_dball_team1_start"
        } else if skin == settings.team2_skin {
            "dm_dball_team2_start"
        } else {
            "info_player_deathmatch"
        };
        let mut best: Option<ActorId> = None;
        let mut distance = 0.0;
        let spots: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        for spot in spots {
            if game.require_entity(&spot).classname != classname {
                continue;
            }
            let candidate = (self.hooks.spawn_distance)(spot.clone(), game);
            if candidate > distance {
                best = Some(spot);
                distance = candidate;
            }
        }
        let Some(best) = best else {
            return (self.hooks.select_spawn)(entity.clone(), game);
        };
        let body = game.body_of(best);
        (add3(body.origin, vec3(0.0, 0.0, 9.0)), body.angles)
    }

    /// Count ball starts after spawning (`postSpawn`).
    pub fn post_spawn(&self, game: &mut Q2GameServices) {
        let _ = self;
        game.deathball.starts = 0;
        let entities: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        for entity in entities {
            let classname = game.require_entity(&entity).classname.clone();
            if classname == "misc_teleporter_dest" {
                game.set_solid(entity, Q2Solid::None);
            } else if classname == "dm_dball_ball_start" {
                game.deathball.starts += 1;
            }
        }
        if game.deathball.starts == 0 {
            game.host.diagnostic("No Deathball start points!");
        }
    }

    /// Scale ball damage (`changeDamage`).
    pub fn change_damage(
        &self,
        target: &ActorId,
        attacker: Option<&ActorId>,
        damage: f64,
        game: &Q2GameServices,
    ) -> f64 {
        let _ = self;
        if Some(target) == game.deathball.ball.as_ref() {
            1.0
        } else if attacker != game.deathball.ball.as_ref() {
            (damage / 2.0).trunc()
        } else {
            damage
        }
    }

    /// Scale ball knockback (`changeKnockback`).
    pub fn change_knockback(
        &self,
        target: &ActorId,
        knockback: f64,
        means_of_death: i32,
        game: &mut Q2GameServices,
    ) -> f64 {
        let _ = self;
        if Some(target) != game.deathball.ball.as_ref() {
            return knockback;
        }
        if knockback < 1.0 {
            if means_of_death == 8 {
                return 70.0;
            }
            if means_of_death == 14 {
                return 90.0;
            }
            game.host.diagnostic(&format!("zero knockback, mod {means_of_death}"));
            return knockback;
        }
        match means_of_death {
            1 => knockback * 3.0,
            2 => (knockback * 3.0 / 8.0).trunc(),
            3 => (knockback / 3.0).trunc(),
            4 | 9 => (knockback * 3.0 / 2.0).trunc(),
            10 => knockback * 4.0,
            6 | 15 | 46 | 7 | 16 | 24 | 51 | 41 => (knockback / 2.0).trunc(),
            11 | 44 => (knockback / 3.0).trunc(),
            _ => knockback,
        }
    }
}

/// Deathball spawn entry.
fn deathball_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    Q2DeathBall { hooks: deathball_hooks(game) }.spawn(entity, game)
}

/// Goal touch (`goalTouch`).
fn dball_goal_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) != game.deathball.ball.as_ref() {
        return;
    }
    let ball_id = game.deathball.ball.clone();
    let Some(ball) = ball_id.as_ref().and_then(|ball| game.entity(ball).map(|entity| entity.actor.id().clone())) else {
        return;
    };
    let team = if game.require_entity(&entity).spawnflags & 1 != 0 { 1 } else { 2 };
    if team == 1 {
        game.deathball.team1_score = (game.deathball.team1_score + game.require_entity(&entity).wait).trunc();
    } else {
        game.deathball.team2_score = (game.deathball.team2_score + game.require_entity(&entity).wait).trunc();
    }
    let enemy = game.require_entity(&ball).enemy.clone();
    for actor in game.host.players() {
        let skin = (deathball_hooks(game).skin)(actor.clone());
        let wait = game.require_entity(&entity).wait;
        let score = (wait + if actor == enemy.clone().unwrap_or(actor.clone()) { 5.0 } else { 0.0 }).trunc();
        if !skin.contains('/') {
            continue;
        }
        let settings = (deathball_hooks(game).settings)();
        let player_team = if skin == settings.team1_skin {
            Some(1)
        } else if skin == settings.team2_skin {
            Some(2)
        } else {
            None
        };
        match player_team {
            None => game.host.diagnostic("unassigned player!!!!"),
            Some(player_team) if player_team == team => {
                (deathball_hooks(game).add_score)(actor, score);
            }
            _ => {
                if Some(&actor) == enemy.as_ref() {
                    (deathball_hooks(game).add_score)(actor, -score);
                }
            }
        }
    }
    dball_reset_ball(&ball, game);
    let authored = game.require_entity(&entity).authored_target();
    let id = ball.clone();
    game.use_targets(&authored, Some(&id), false);
}

/// Ball touch (`ballTouch`).
fn dball_ball_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.host.is_player(&contact.other)
        || !game.host.combat().read(&contact.other).is_some_and(|combat| combat.can_take_damage)
    {
        return;
    }
    let body = game.body_of(entity.clone());
    let other = game.host.bodies().read(&contact.other);
    let speed = f64::from(length3(body.velocity));
    if let Some(other) = other {
        if speed != 0.0 && dot3(sub3(body.origin, other.origin), body.velocity) > 0.7 {
            let damage = (speed / 10.0).trunc();
            game.damage(
                contact.other,
                entity.clone(),
                Some(entity.clone()),
                damage,
                damage,
                Vec3::default(),
                body.origin,
                Vec3::default(),
                52,
                0,
                None,
            );
        }
    }
}

/// Ball pain (`pain`).
fn dball_ball_pain(entity: ActorId, game: &mut Q2GameServices, reaction: PainReaction) {
    game.require_entity_mut(&entity).enemy = reaction.attacker;
    let max_health = game.require_entity(&entity).max_health;
    let owned = game.owned_of(entity);
    game.host.combat().set_health(&owned, max_health);
}

/// Goal effect (`goalEffect`).
fn dball_goal_effect(entity: &ActorId, game: &mut Q2GameServices) {
    let origin = game.body_of(entity.clone()).origin;
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:dball_goal".to_string(),
        origin,
        direction: Vec3::default(),
        count: 0,
        color: 0,
    }));
}

/// Ball die (`die`).
fn dball_ball_die(entity: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    dball_reset_ball(&entity, game);
}

/// Reset the ball (`resetBall`).
fn dball_reset_ball(entity: &ActorId, game: &mut Q2GameServices) {
    dball_goal_effect(entity, game);
    game.require_entity_mut(entity).angular_velocity = Vec3::default();
    let mut moved = game.body_of(entity.clone());
    moved.angles = Vec3::default();
    moved.velocity = Vec3::default();
    game.write_body(entity.clone(), &moved, true);
    let motion = game.require_entity(entity).motion;
    game.set_motion_kind(entity.clone(), motion);
    game.set_solid(entity.clone(), Q2Solid::None);
    game.schedule(entity.clone(), 2.0, dball_respawn as Q2Think);
}

/// Ball respawn (`respawn`).
fn dball_respawn(entity: ActorId, game: &mut Q2GameServices) {
    dball_goal_effect(&entity, game);
    let starts: Vec<ActorId> = game
        .entities
        .values()
        .filter(|entity| entity.classname == "dm_dball_ball_start")
        .map(|entity| entity.actor.id().clone())
        .collect();
    let which = (game.host.random() * f64::from(game.deathball.starts)).ceil() as usize;
    let spot = starts.get(which.wrapping_sub(1)).or_else(|| starts.first());
    if spot.is_none() {
        game.host.diagnostic("No ball start points found!");
    }
    game.require_entity_mut(&entity).angular_velocity = Vec3::default();
    game.require_entity_mut(&entity).model = "models/objects/dball/tris.md2".to_string();
    let origin = spot.map(|spot| game.body_of(spot.clone()).origin).unwrap_or_else(|| game.body_of(entity.clone()).origin);
    let mut moved = game.body_of(entity.clone());
    moved.origin = origin;
    moved.angles = Vec3::default();
    moved.velocity = Vec3::default();
    moved.ground = None;
    game.write_body(entity.clone(), &moved, true);
    game.set_solid(entity.clone(), Q2Solid::Box);
    let motion = game.require_entity(&entity).motion;
    game.set_motion_kind(entity.clone(), motion);
    game.show(entity.clone());
    game.host.emit(Q2PresentationEvent::EntityEvent {
        actor: entity.clone(),
        event: 6,
    });
    kill_q2_box(game, entity.clone());
    game.link_actor(entity);
}

/// Speed touch (`speedTouch`).
fn dball_speed_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if Some(&contact.other) != game.deathball.ball.as_ref()
        || game.require_entity(&entity).timestamp >= game.host.now()
    {
        return;
    }
    let ball_id = game.deathball.ball.clone();
    let Some(ball) = ball_id.as_ref().and_then(|ball| game.entity(ball).map(|entity| entity.actor.id().clone())) else {
        return;
    };
    let velocity = game.body_of(ball.clone()).velocity;
    let (spawnflags, movedir, delay, speed) = {
        let record = game.require_entity(&entity);
        (record.spawnflags, record.movedir, record.delay, record.speed)
    };
    if f64::from(length3(velocity)) < 1.0
        || spawnflags & 1 != 0 && dot3(normalize3(velocity), movedir) < 0.8
    {
        return;
    }
    game.require_entity_mut(&entity).timestamp = game.host.now() + delay;
    let mut moved = game.body_of(ball.clone());
    moved.velocity = scale3(velocity, speed as f32);
    game.write_body(ball.clone(), &moved, true);
    let motion = game.require_entity(&ball).motion;
    game.set_motion_kind(ball, motion);
}
