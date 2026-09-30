//! Q1 mg3 monster policy (`src/content/q1/addons/monsters/ai/controller.ts`).
//!
//! `quakec_mg3/ai.qc`: source targeting, liquid damage and Horde
//! movement policy. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.
//!
//! Shares the base source animation/combat owner; MG3 supplies only
//! its differing AI policy. Rust has no virtual dispatch, so mg3
//! prefixes register their own frame driver here
//! ([`register_mg3_monster_callbacks`]): the play loop below mirrors
//! [`BaseMonster::play`] but routes steering steps to [`Mg3Monster::ai`].
//! Lifecycle hooks that vary per family (sight, melee, attacks, pain,
//! death, use, start) live in [`Mg3SourceHooks`]; hooks default to the
//! shared base behavior the donor inherits.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::monsters::monster_target_eligible;
use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation};
use crate::q1::base::frames::monster_frame;
use crate::q1::base::monster_actions::{monster_action, monster_jump_touch};
use crate::q1::base::monsters::{
    register_monster_source, BaseMonster, MonsterActionHandler, MonsterControllerLoad, MonsterControllerStore,
    MonsterSource, MonsterSourceRegistration,
};
use crate::q1::base::projectiles::cast_lightning;
use crate::q1::foundation::callbacks::{
    Q1ActionHandler, Q1CallbackHandlers, Q1DieHandler, Q1PainHandler, Q1TouchHandler, Q1UseHandler,
};
use crate::q1::foundation::entity::{Q1AttackState, Q1MonsterMode, Q1MonsterSpecies};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::types::{
    dot, normalize, vsub, yaw_for, Q1Basis, Q1Edition, Q1Event, Q1Powerup, Q1TraceRequest, POINT,
};
use crate::q1::{q1_error, Q1Error};

use super::path::{walk_mg3_path_to_goal, Mg3PathResult};

fn fround(value: f64) -> f64 {
    f64::from(value as f32)
}

fn same(first: Option<&ActorId>, second: Option<&ActorId>) -> bool {
    match (first, second) {
        (None, None) => true,
        (Some(first), Some(second)) => same_actor(first, second),
        _ => false,
    }
}

/// Mg3 sight-sound hook (`sightSound` override).
pub type Mg3SightHandler = fn(monster: &mut Mg3Monster) -> Result<(), Q1Error>;
/// Mg3 melee-attack hook (`meleeAttack` override).
pub type Mg3MeleeHandler = fn(monster: &mut Mg3Monster) -> Result<(), Q1Error>;
/// Mg3 attack-attempt hook (`tryAttack` override).
pub type Mg3TryAttackHandler = fn(monster: &mut Mg3Monster) -> Result<bool, Q1Error>;
/// Mg3 pain hook (`pain` override).
pub type Mg3PainHandler = fn(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error>;
/// Mg3 death hook (`die` override).
pub type Mg3DieHandler = fn(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error>;
/// Mg3 use hook (`use` override).
pub type Mg3UseHandler = fn(monster: &mut Mg3Monster, activator: Option<&ActorId>) -> Result<(), Q1Error>;
/// Mg3 start hook (`start` override).
pub type Mg3StartHandler = fn(monster: &mut Mg3Monster) -> Result<(), Q1Error>;
/// Mg3 steering hook (`ai` override).
pub type Mg3AiHandler = fn(monster: &mut Mg3Monster, mode: MonsterAi, distance: f64) -> Result<(), Q1Error>;
/// Mg3 combat-steering hook (`run` override).
pub type Mg3RunHandler = fn(monster: &mut Mg3Monster, distance: f64) -> Result<(), Q1Error>;
/// Mg3 target-scan hook (`findTarget` override).
pub type Mg3FindTargetHandler = fn(monster: &mut Mg3Monster) -> Result<bool, Q1Error>;
/// Mg3 target-acquire hook (`found` override).
pub type Mg3FoundHandler = fn(monster: &mut Mg3Monster, target: &ActorId) -> Result<(), Q1Error>;
/// Mg3 frame-play hook (`play` override).
pub type Mg3PlayHandler = fn(monster: &mut Mg3Monster, name: &str) -> Result<(), Q1Error>;

/// Per-family lifecycle hooks for an mg3 monster source. `None`
/// keeps the shared base behavior the donor inherits.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mg3SourceHooks {
    /// Sight-sound override.
    pub sight_sound: Option<Mg3SightHandler>,
    /// Melee-attack override.
    pub melee_attack: Option<Mg3MeleeHandler>,
    /// Attack-attempt override.
    pub try_attack: Option<Mg3TryAttackHandler>,
    /// Pain override.
    pub pain: Option<Mg3PainHandler>,
    /// Death override.
    pub die: Option<Mg3DieHandler>,
    /// Use override.
    pub use_monster: Option<Mg3UseHandler>,
    /// Start override.
    pub start: Option<Mg3StartHandler>,
    /// Steering override.
    pub ai: Option<Mg3AiHandler>,
    /// Combat-steering override.
    pub run: Option<Mg3RunHandler>,
    /// Target-scan override.
    pub find_target: Option<Mg3FindTargetHandler>,
    /// Target-acquire override.
    pub found: Option<Mg3FoundHandler>,
    /// Frame-play override.
    pub play: Option<Mg3PlayHandler>,
}

/// Mg3 monster source registration. Frames and actions mirror the
/// shared source entry so the mg3 play loop can dispatch them.
#[derive(Debug, Clone, Copy)]
pub struct Mg3SourceRegistration {
    /// Alternate frames.
    pub frames: &'static HashMap<String, MonsterFrame>,
    /// Alternate actions.
    pub actions: &'static HashMap<String, MonsterActionHandler>,
    /// Lifecycle hooks.
    pub hooks: Mg3SourceHooks,
}

fn mg3_sources() -> &'static Mutex<HashMap<&'static str, Mg3SourceRegistration>> {
    static SOURCES: OnceLock<Mutex<HashMap<&'static str, Mg3SourceRegistration>>> = OnceLock::new();
    SOURCES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Look up an mg3 monster source by callback prefix.
pub fn mg3_monster_source(prefix: &str) -> Result<Mg3SourceRegistration, Q1Error> {
    mg3_sources()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(prefix)
        .copied()
        .ok_or_else(|| q1_error(format!("Unknown Q1 mg3 monster source: {prefix}")))
}

/// Register an mg3 monster source for the mg3 frame driver.
/// Re-registering a prefix keeps the first entry.
pub fn register_mg3_monster_source(
    prefix: &'static str,
    frames: &'static HashMap<String, MonsterFrame>,
    actions: &'static HashMap<String, MonsterActionHandler>,
    hooks: Mg3SourceHooks,
    load: MonsterControllerLoad,
    store: MonsterControllerStore,
) {
    register_monster_source(MonsterSourceRegistration {
        source: MonsterSource {
            prefix,
            frames: Some(frames),
            actions: Some(actions),
        },
        load,
        store,
    });
    mg3_sources()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(prefix)
        .or_insert(Mg3SourceRegistration { frames, actions, hooks });
}

/// Mg3 monster policy view (`Mg3Monster`). Loaded per callback and
/// finished back, like the shared owner.
pub struct Mg3Monster<'g> {
    /// Shared animation/combat owner.
    pub monster: BaseMonster<'g>,
}

impl<'g> Mg3Monster<'g> {
    /// Load an mg3 monster controller view.
    pub fn load(game: &'g mut Q1EntityServices, id: &ActorId) -> Result<Self, Q1Error> {
        Ok(Self {
            monster: BaseMonster::load(game, id)?,
        })
    }

    /// Write the controller and monster state back.
    pub fn finish(self) -> Result<(), Q1Error> {
        self.monster.finish()
    }

    /// Build a fresh controller for spawning under a prefix
    /// (`BaseMonster.spawn_new` for mg3 sources).
    pub fn spawn_new(
        game: &'g mut Q1EntityServices,
        prefix: &str,
        id: &ActorId,
        spec: &'static crate::q1::base::species::MonsterSpecies,
    ) -> Result<Self, Q1Error> {
        use crate::q1::base::monsters::BaseMonsterState;
        use crate::q1::foundation::entity::{Q1Monster, Q1MonsterMode};

        let id = id.clone();
        let controller = BaseMonsterState::new(spec.species, prefix.to_string(), spec.stand);
        let target = game
            .entity_ref(&id)
            .map(|entity| entity.target.clone())
            .unwrap_or_default();
        let monster = game
            .entity_ref(&id)
            .and_then(|entity| entity.monster.clone())
            .unwrap_or(Q1Monster {
                species: spec.species,
                mode: Q1MonsterMode::Stand,
                frame_index: 0,
                sequence: Vec::new(),
                first_frame: 0,
                enemy: None,
                old_enemy: None,
                path: target,
                pause_until: 0.0,
                attack_finished: 0.0,
                pain_finished: 0.0,
                search_until: 0.0,
                death_drop: false,
                refired: false,
            });
        game.update_entity(&id, |entity| {
            entity
                .fields
                .insert(String::from("source.monsterCallbackPrefix"), prefix.to_string());
            if entity.monster.is_none() {
                entity.monster = Some(monster.clone());
            }
        })?;
        crate::q1::base::creatures::store_monster_controller(game, &id, controller.clone())?;
        Ok(Self {
            monster: BaseMonster {
                game,
                id,
                spec,
                prefix: prefix.to_string(),
                controller,
                monster,
            },
        })
    }

    /// Lifecycle hooks for this monster's source.
    fn hooks(&self) -> Result<Mg3SourceHooks, Q1Error> {
        Ok(mg3_monster_source(&self.monster.prefix.clone())?.hooks)
    }

    /// Save and return the angle basis (`makeVectors`). MG3 passes
    /// body angles directly, unlike the id1 rerelease pitch
    /// adjustment.
    pub fn make_vectors(&mut self) -> Result<Q1Basis, Q1Error> {
        let angles = self
            .monster
            .game
            .body(&self.monster.id.clone())
            .map(|body| body.angles)?;
        Ok(self.monster.game.make_vectors(angles))
    }

    /// Delay the next missile attack (`attackFinished`).
    pub fn attack_finished(&mut self, seconds: f64) {
        self.monster.monster.refired = false;
        self.monster.monster.attack_finished = fround(self.monster.game.time + seconds);
    }

    /// Play the sight sound (`sightSound`).
    pub fn sight_sound(&mut self) -> Result<(), Q1Error> {
        if let Some(sight) = self.hooks()?.sight_sound {
            return sight(self);
        }
        let entity = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let mut sound = self.monster.spec.sight.to_string();
        if entity.classname == "monster_ogre" && entity.number("aflag") != 0.0 {
            sound = String::from("armagon/sight.wav");
        } else if entity.classname == "monster_hell_knight"
            && entity.solid == crate::q1::foundation::types::Q1Solid::None
            && (entity.spawnflags & (65536 | 8388608)) != 0
        {
            return Ok(());
        } else if entity.classname == "monster_enforcer" {
            let choice = (self.monster.game.host.random() * 3.0 + 0.5).floor() as i32;
            let track = if choice == 1 {
                1
            } else if choice == 2 {
                2
            } else if choice == 0 {
                3
            } else {
                4
            };
            sound = format!("enforcer/sight{track}.wav");
        }
        if sound.is_empty() {
            return Ok(());
        }
        self.monster.game.sound_simple(&self.monster.id.clone(), &sound)
    }

    /// Acquire a target (`found`).
    pub fn found(&mut self, target: &ActorId) -> Result<(), Q1Error> {
        if let Some(found) = self.hooks()?.found {
            let target = target.clone();
            return found(self, &target);
        }
        let target = target.clone();
        self.monster.monster.enemy = Some(target.clone());
        if self.monster.game.is_player(&target) {
            self.monster.game.sight_entity = Some(self.monster.id.clone());
            self.monster.game.sight_time = self.monster.game.time;
        }
        let entity = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        if entity.classname == "monster_hell_knight"
            && (entity.spawnflags & (65536 | 8388608)) != 0
            && self.monster.monster.pain_finished > self.monster.game.time
        {
            return Ok(());
        }
        let hostile = fround(self.monster.game.time + 1.0);
        self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
            entity.fields.insert(String::from("show_hostile"), hostile.to_string());
        })?;
        self.sight_sound()?;
        self.hunt_target()
    }

    /// Attempt a ranged or melee attack (`checkAttack`).
    pub fn check_attack(&mut self) -> Result<bool, Q1Error> {
        let enemy = self.monster.monster.enemy.clone();
        let start = self.monster.eye(None)?;
        let end = match enemy.as_ref() {
            Some(enemy) => self.monster.eye(Some(enemy))?,
            None => None,
        };
        let (Some(enemy), Some(start), Some(end)) = (enemy, start, end) else {
            return Ok(false);
        };
        let trace = self.monster.game.host.trace(&Q1TraceRequest {
            start,
            end,
            bounds: POINT,
            ignore: Some(self.monster.id.clone()),
            monsters: true,
            missile: false,
        });
        if trace.actor.as_ref().is_none_or(|actor| !same_actor(actor, &enemy)) || trace.in_open && trace.in_water {
            return Ok(false);
        }
        let range = self
            .monster
            .game
            .world
            .clone()
            .and_then(|world| {
                self.monster
                    .game
                    .entity_ref(&world)
                    .map(|entity| entity.number("enemy_range"))
            })
            .unwrap_or(0.0);
        if range == 0.0 && self.monster.spec.melee {
            self.melee_attack()?;
            return Ok(true);
        }
        if self.monster.spec.missile.is_none()
            || self.monster.game.time < self.monster.monster.attack_finished
            || range == 3.0
        {
            return Ok(false);
        }
        if range == 0.0 {
            self.monster.monster.attack_finished = 0.0;
        }
        let melee = self.monster.spec.melee;
        let chance = if range == 0.0 {
            0.9
        } else if range == 1.0 {
            if melee {
                0.2
            } else {
                0.4
            }
        } else if range == 2.0 {
            if melee {
                0.05
            } else {
                0.1
            }
        } else {
            0.0
        };
        if self.monster.game.host.random() >= chance {
            return Ok(false);
        }
        if let Some(missile) = self.monster.spec.missile {
            self.play(missile)?;
        }
        let delay = 2.0 * self.monster.game.host.random();
        self.attack_finished(delay);
        Ok(true)
    }

    /// Write a source world word (`sourceWord`).
    pub fn source_word(&mut self, name: &str, value: f64) -> Result<(), Q1Error> {
        let Some(world) = self.monster.game.world.clone() else {
            return Err(q1_error("MG3 AI requires the source world globals"));
        };
        let value = fround(value).to_string();
        self.monster.game.update_entity(&world, |entity| {
            entity.fields.insert(name.to_string(), value);
        })
    }

    /// Range category to a target (`rangeCategory`).
    pub fn range_category(&mut self, target: Option<&ActorId>) -> Result<i32, Q1Error> {
        let target = target.cloned().or_else(|| self.monster.monster.enemy.clone());
        let distance = self.monster.range_distance(target.as_ref())?;
        let spawnflags = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .map(|entity| entity.spawnflags)
            .unwrap_or(0);
        let near_sighted = spawnflags & 8192 != 0;
        let (melee, near, mid) = if near_sighted {
            if self.monster.monster.enemy.is_none() {
                (120.0, 300.0, 340.0)
            } else {
                (96.0, 400.0, 800.0)
            }
        } else {
            (120.0, 500.0, 1000.0)
        };
        Ok(if distance < melee {
            0
        } else if distance < near {
            1
        } else if distance < mid {
            2
        } else {
            3
        })
    }

    /// Whether a target is in front (`inFront`).
    pub fn in_front(&mut self, target: &ActorId) -> Result<bool, Q1Error> {
        let Some(body) = self.monster.game.host.bodies.read(target) else {
            return Ok(false);
        };
        let basis = self.make_vectors()?;
        let origin = self.monster.origin()?;
        let facing = dot(normalize(vsub(body.origin, origin)), basis.forward);
        let entity = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let threshold = if entity.spawnflags & 8192 != 0 {
            fround(0.866)
        } else {
            fround(0.3)
        };
        let facing = f64::from(facing);
        Ok(facing > threshold || entity.classname == "monster_orb" && -facing > threshold)
    }

    /// Scan for a target (`findTarget`).
    pub fn find_target(&mut self) -> Result<bool, Q1Error> {
        if let Some(find_target) = self.hooks()?.find_target {
            return find_target(self);
        }
        let entity = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        if entity.spawnflags & 32 != 0 {
            let mut targets = Vec::new();
            for id in self.monster.game.find(&entity.target.clone()) {
                if self.monster.game.is_damageable(&id) {
                    targets.push(id);
                }
            }
            if !targets.is_empty() {
                let index =
                    ((targets.len() as f64 * self.monster.game.host.random()).floor() as usize).min(targets.len() - 1);
                let target = targets[index].clone();
                self.monster.monster.enemy = Some(target.clone());
                self.found(&target)?;
                return Ok(true);
            }
        }
        let candidate = if self.monster.game.sight_time >= self.monster.game.time - 0.1 && entity.spawnflags & 3 == 0 {
            let sight = self.monster.game.sight_entity.clone();
            let sight_enemy = sight.as_ref().and_then(|sight| {
                self.monster
                    .game
                    .entity_ref(sight)
                    .and_then(|entity| entity.monster.as_ref())
                    .and_then(|monster| monster.enemy.clone())
            });
            if same(sight_enemy.as_ref(), self.monster.monster.enemy.as_ref()) {
                return Ok(false);
            }
            sight
        } else {
            let owned = entity.actor.clone();
            self.monster.game.host.check_client(&owned)
        };
        let Some(candidate) = candidate else {
            return Ok(false);
        };
        if same(Some(&candidate), self.monster.monster.enemy.as_ref()) {
            return Ok(false);
        }
        let actor = self.monster.game.entity_ref(&candidate).cloned();
        let player = self.monster.game.player_ref(&candidate).cloned();
        if actor.map(|actor| actor.movement_flags).unwrap_or(0) & 128 != 0
            || player
                .as_ref()
                .and_then(|player| player.powerups.get(&Q1Powerup::Invisibility).copied())
                .unwrap_or(0.0)
                > self.monster.game.time
        {
            return Ok(false);
        }
        let range = self.range_category(Some(&candidate))?;
        if range == 3 || !self.monster.visible(Some(&candidate))? {
            return Ok(false);
        }
        let hostile = player.map(|player| player.hostile_until).or_else(|| {
            self.monster
                .game
                .entity_ref(&candidate)
                .map(|actor| actor.number("show_hostile"))
        });
        let hostile = hostile.unwrap_or(0.0);
        if range == 1 && hostile < self.monster.game.time && !self.in_front(&candidate)?
            || range == 2 && !self.in_front(&candidate)?
        {
            return Ok(false);
        }
        self.monster.monster.enemy = Some(candidate.clone());
        if !self.monster.game.is_player(&candidate) {
            let enemy = self
                .monster
                .game
                .entity_ref(&candidate)
                .and_then(|actor| actor.monster.as_ref())
                .and_then(|monster| monster.enemy.clone());
            self.monster.monster.enemy = enemy.clone();
            if enemy.as_ref().is_none_or(|enemy| !self.monster.game.is_player(enemy)) {
                self.monster.monster.enemy = None;
                return Ok(false);
            }
        }
        let enemy = self
            .monster
            .monster
            .enemy
            .clone()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        self.found(&enemy)?;
        Ok(true)
    }

    /// Apply liquid damage (`checkContentsDamage`).
    pub fn check_contents_damage(&mut self) -> Result<bool, Q1Error> {
        let entity = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        if entity.water_level == 0
            || self.monster.game.health(&self.monster.id.clone()) <= 0.0
            || self.monster.game.time < entity.number("dmgtime")
            || entity.spawnflags & 16384 != 0
        {
            return Ok(false);
        }
        if entity.water_type != -4 && entity.water_type != -5 {
            return Ok(false);
        }
        let Some(world) = self.monster.game.world.clone() else {
            return Err(q1_error("MG3 contents damage requires the world actor"));
        };
        let lava = entity.water_type == -5;
        let until = fround(self.monster.game.time + if lava { 0.2 } else { 1.0 });
        self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
            entity.fields.insert(String::from("dmgtime"), until.to_string());
        })?;
        let amount = if lava {
            if entity.classname == "monster_zombie" {
                120.0
            } else {
                30.0 * f64::from(entity.water_level)
            }
        } else {
            4.0 * f64::from(entity.water_level)
        };
        self.monster.game.damage(
            &self.monster.id.clone(),
            Some(&world),
            Some(&world),
            amount,
            &Q1DamageParams::default(),
        );
        Ok(lava && entity.classname == "monster_zombie" || self.monster.game.health(&self.monster.id.clone()) <= 0.0)
    }

    /// Run one steering step (`ai`).
    pub fn ai(&mut self, mode: MonsterAi, distance: f64) -> Result<(), Q1Error> {
        if let Some(ai) = self.hooks()?.ai {
            return ai(self, mode, distance);
        }
        match mode {
            MonsterAi::Stand => {
                if self.check_contents_damage()? || self.find_target()? {
                    return Ok(());
                }
                if self.monster.game.time > self.monster.monster.pause_until {
                    let walk = self.monster.spec.walk;
                    self.play(walk)?;
                }
                Ok(())
            }
            MonsterAi::Walk => {
                if self.check_contents_damage()? {
                    return Ok(());
                }
                self.source_word("movedist", distance)?;
                if self.find_target()? {
                    return Ok(());
                }
                let goal = self
                    .monster
                    .game
                    .find(&self.monster.monster.path.clone())
                    .first()
                    .cloned()
                    .or_else(|| self.monster.game.world.clone());
                if let Some(goal) = goal {
                    let owned = self
                        .monster
                        .game
                        .entity_ref(&self.monster.id.clone())
                        .map(|entity| entity.actor.clone())
                        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                    self.monster.game.host.move_to_goal(&owned, &goal, distance, None);
                }
                Ok(())
            }
            MonsterAi::Turn => {
                if self.find_target()? {
                    return Ok(());
                }
                self.monster.change_yaw()
            }
            MonsterAi::Painforward => {
                let (owned, yaw) = self
                    .monster
                    .game
                    .entity_ref(&self.monster.id.clone())
                    .map(|entity| (entity.actor.clone(), entity.ideal_yaw))
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                self.monster.game.host.walk_move(&owned, yaw, distance);
                Ok(())
            }
            MonsterAi::Run => {
                self.monster.monster.mode = Q1MonsterMode::Run;
                self.run(distance)
            }
            _ => self.monster.ai(mode, distance),
        }
    }

    /// Chase the current target (`huntTarget`).
    pub fn hunt_target(&mut self) -> Result<(), Q1Error> {
        let Some(target) = self.monster.target()? else {
            return Ok(());
        };
        self.monster.monster.mode = Q1MonsterMode::Run;
        let origin = self.monster.origin()?;
        let yaw = yaw_for(vsub(target, origin));
        self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
            entity.ideal_yaw = yaw;
        })?;
        self.monster.controller.next_frame = self.monster.spec.run.to_string();
        self.monster.delay(0.1)?;
        self.attack_finished(1.0);
        Ok(())
    }

    /// Publish enemy knowledge to the world (`updateRunKnowledge`).
    pub fn update_run_knowledge(&mut self) -> Result<(), Q1Error> {
        let range = self.range_category(None)?;
        self.source_word("enemy_range", f64::from(range))?;
        if let Some(target) = self.monster.target()? {
            let origin = self.monster.origin()?;
            self.source_word("enemy_yaw", yaw_for(vsub(target, origin)))?;
        }
        Ok(())
    }

    /// Cooperative re-target scan (`searchForCoopTarget`).
    pub fn search_for_coop_target(&mut self) -> Result<bool, Q1Error> {
        if self.monster.game.options().coop && self.monster.monster.search_until < self.monster.game.time {
            return self.find_target();
        }
        Ok(false)
    }

    /// Select a melee attack (`meleeAttack`).
    pub fn melee_attack(&mut self) -> Result<(), Q1Error> {
        if let Some(melee) = self.hooks()?.melee_attack {
            return melee(self);
        }
        self.monster.melee_attack()
    }

    /// Attempt an attack (`tryAttack`).
    pub fn try_attack(&mut self) -> Result<bool, Q1Error> {
        if let Some(attack) = self.hooks()?.try_attack {
            return attack(self);
        }
        mg3_default_try_attack(self)
    }

    /// Retaliate against an attacker (`retaliate`).
    pub fn retaliate(&mut self, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
        let Some(attacker) = attacker.cloned() else {
            return Ok(());
        };
        if same_actor(&attacker, &self.monster.id.clone()) {
            return Ok(());
        }
        let classname = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        if self.monster.game.host.classname(&attacker) == classname {
            return Ok(());
        }
        if let Some(world) = self.monster.game.world.clone() {
            if same_actor(&attacker, &world) {
                return Ok(());
            }
        }
        if let Some(enemy) = self.monster.monster.enemy.clone() {
            if same_actor(&attacker, &enemy) {
                return Ok(());
            }
            if self.monster.game.is_player(&enemy) {
                self.monster.monster.old_enemy = Some(enemy);
            }
        }
        self.found(&attacker)
    }

    /// React to pain (`pain`).
    pub fn pain(&mut self, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
        if let Some(pain) = self.hooks()?.pain {
            let attacker = attacker.cloned();
            return pain(self, attacker.as_ref(), damage);
        }
        let attacker = attacker.cloned();
        mg3_default_pain(self, attacker.as_ref(), damage)
    }

    /// React to pain without consulting the pain hook.
    pub fn pain_default(&mut self, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
        let attacker = attacker.cloned();
        mg3_default_pain(self, attacker.as_ref(), damage)
    }

    /// Die (`die`).
    pub fn die(&mut self, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
        if let Some(die) = self.hooks()?.die {
            let attacker = attacker.cloned();
            return die(self, attacker.as_ref());
        }
        let attacker = attacker.cloned();
        self.monster.die(attacker.as_ref())
    }

    /// Use a monster (`use`).
    pub fn use_monster(&mut self, activator: Option<&ActorId>) -> Result<(), Q1Error> {
        if let Some(use_monster) = self.hooks()?.use_monster {
            let activator = activator.cloned();
            return use_monster(self, activator.as_ref());
        }
        let activator = activator.cloned();
        self.monster.use_monster(activator.as_ref())
    }

    /// Start a monster after map spawn (`start`).
    pub fn start(&mut self) -> Result<(), Q1Error> {
        if let Some(start) = self.hooks()?.start {
            return start(self);
        }
        self.monster.start()
    }

    /// Advance the patrol route (`updateRoute`).
    pub fn update_route(&mut self) -> Result<(), Q1Error> {
        let goal = self.monster.route()?;
        if goal.is_none() || self.monster.monster.pause_until > self.monster.game.time {
            self.monster.monster.mode = Q1MonsterMode::Stand;
            let stand = self.monster.spec.stand;
            return self.play(stand);
        }
        if self.monster.monster.mode == Q1MonsterMode::Stand {
            self.monster.monster.mode = Q1MonsterMode::Walk;
            let walk = self.monster.spec.walk;
            return self.play(walk);
        }
        Ok(())
    }

    /// Play a named frame (`play`).
    pub fn play(&mut self, name: &str) -> Result<(), Q1Error> {
        if let Some(play) = self.hooks()?.play {
            return play(self, name);
        }
        self.play_default(name)
    }

    /// Play a named frame without consulting the play hook
    /// (`BaseMonster.play`). Actions without frames run directly,
    /// following the heavy override.
    pub fn play_default(&mut self, name: &str) -> Result<(), Q1Error> {
        if self.play_preamble()? {
            return Ok(());
        }
        let source = mg3_monster_source(&self.monster.prefix.clone())?;
        if !source.frames.contains_key(name) {
            if let Some(action) = source.actions.get(name) {
                return action(&mut self.monster);
            }
        }
        let frame = source
            .frames
            .get(name)
            .copied()
            .or_else(|| monster_frame(name).copied())
            .ok_or_else(|| q1_error(format!("Missing Q1 source animation {name}")))?;
        self.play_ops(name, frame)
    }

    /// Run the play preamble: liveness and enemy eligibility. Returns
    /// whether the preamble fully handled the play.
    pub fn play_preamble(&mut self) -> Result<bool, Q1Error> {
        if !self.monster.game.is_live(&self.monster.id.clone()) {
            return Ok(true);
        }
        if self.monster.game.health(&self.monster.id.clone()) > 0.0 {
            if let Some(enemy) = self.monster.monster.enemy.clone() {
                if !monster_target_eligible(
                    self.monster.game.health(&enemy),
                    self.monster.game.monster_target(&enemy).as_ref(),
                ) {
                    let previous = self.monster.monster.old_enemy.clone();
                    self.monster.monster.enemy = match previous {
                        Some(previous)
                            if monster_target_eligible(
                                self.monster.game.health(&previous),
                                self.monster.game.monster_target(&previous).as_ref(),
                            ) =>
                        {
                            Some(previous)
                        }
                        _ => None,
                    };
                    self.monster.monster.old_enemy = None;
                    self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
                        entity.attack_state = Q1AttackState::Straight
                    })?;
                    let fallback = if self.monster.monster.enemy.is_none() {
                        if self.monster.route()?.is_none() {
                            self.monster.spec.stand
                        } else {
                            self.monster.spec.walk
                        }
                    } else {
                        self.monster.spec.run
                    };
                    self.play(fallback)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Enter a resolved frame: schedule the continuation and run its
    /// operations through the mg3 driver.
    pub fn play_ops(&mut self, name: &str, frame: MonsterFrame) -> Result<(), Q1Error> {
        let source = mg3_monster_source(&self.monster.prefix.clone())?;
        self.monster.controller.current_frame = name.to_string();
        self.monster.controller.next_frame = frame.next.to_string();
        let model_frame = frame.frame;
        self.monster
            .game
            .update_entity(&self.monster.id.clone(), |entity| entity.frame = model_frame)?;
        let prefix = self.monster.prefix.clone();
        self.monster
            .game
            .schedule(&self.monster.id.clone(), 0.1, &format!("{prefix}:monster_frame"))?;
        if self.monster.game.options().edition == Q1Edition::Classic {
            if name == "boss_idle1" || name == "f_death2" {
                return Ok(());
            }
            if name == "f_death21" {
                use crate::q1::foundation::types::Q1Solid;
                self.monster
                    .game
                    .update_entity(&self.monster.id.clone(), |entity| entity.solid = Q1Solid::None)?;
                return self.monster.game.link(&self.monster.id.clone());
            }
            if name == "sham_magic11" && self.monster.game.options().skill == 3 {
                return cast_lightning(&mut self.monster);
            }
        }
        for op in frame.operations {
            if !self.monster.game.is_live(&self.monster.id.clone()) {
                return Ok(());
            }
            match op {
                MonsterOperation::Ai { mode, distance } => self.ai(*mode, *distance)?,
                MonsterOperation::Solid { solid } => {
                    self.monster
                        .game
                        .update_entity(&self.monster.id.clone(), |entity| entity.solid = *solid)?;
                    self.monster.game.link(&self.monster.id.clone())?;
                }
                MonsterOperation::Lightstyle { pattern } => {
                    self.monster.game.host.emit(Q1Event::Lightstyle {
                        style: 0,
                        pattern: (*pattern).to_string(),
                    });
                }
                MonsterOperation::Action { name } => match source.actions.get(*name) {
                    Some(action) => action(&mut self.monster)?,
                    None => monster_action(&mut self.monster, name)?,
                },
                MonsterOperation::Sound {
                    path,
                    channel,
                    attenuation,
                    comparison,
                    chance,
                } => {
                    let draw = if chance.is_some() {
                        self.monster.game.host.random()
                    } else {
                        0.0
                    };
                    let play = match chance {
                        None => true,
                        Some(chance) => match comparison {
                            crate::q1::base::animation::SoundComparison::Greater => draw > *chance,
                            crate::q1::base::animation::SoundComparison::Less => draw < *chance,
                        },
                    };
                    if play {
                        self.monster
                            .game
                            .sound(&self.monster.id.clone(), path, *channel, *attenuation, 1.0)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Run combat steering (`run`).
    pub fn run(&mut self, distance: f64) -> Result<(), Q1Error> {
        if let Some(run) = self.hooks()?.run {
            return run(self, distance);
        }
        if self.check_contents_damage()? {
            return Ok(());
        }
        self.source_word("movedist", distance)?;
        let enemy = self.monster.monster.enemy.clone();
        if enemy
            .as_ref()
            .is_none_or(|enemy| self.monster.game.health(enemy) <= 0.0)
        {
            self.monster.monster.enemy = None;
            let old = self.monster.monster.old_enemy.clone();
            if old.as_ref().is_some_and(|old| self.monster.game.health(old) > 0.0) {
                self.monster.monster.enemy = old;
                self.hunt_target()?;
            } else {
                let fallback = if self.monster.monster.path.is_empty() {
                    self.monster.spec.stand
                } else {
                    self.monster.spec.walk
                };
                self.play(fallback)?;
            }
            return Ok(());
        }
        let hostile = fround(self.monster.game.time + 1.0);
        self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
            entity.fields.insert(String::from("show_hostile"), hostile.to_string());
        })?;
        let seen = self.monster.visible(None)?;
        self.source_word("enemy_visible", f64::from(seen))?;
        if seen {
            self.monster.monster.search_until = fround(self.monster.game.time + 5.0);
        }
        if self.search_for_coop_target()? {
            return Ok(());
        }
        self.update_run_knowledge()?;
        let attack_state = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .map(|entity| entity.attack_state)
            .unwrap_or(Q1AttackState::Straight);
        if attack_state == Q1AttackState::Missile || attack_state == Q1AttackState::Melee {
            let yaw = self
                .monster
                .game
                .world
                .clone()
                .and_then(|world| {
                    self.monster
                        .game
                        .entity_ref(&world)
                        .map(|entity| entity.number("enemy_yaw"))
                })
                .unwrap_or(0.0);
            self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
                entity.ideal_yaw = yaw;
            })?;
            self.monster.change_yaw()?;
            let body_yaw = f64::from(
                self.monster
                    .game
                    .body(&self.monster.id.clone())
                    .map(|body| body.angles.y)
                    .unwrap_or(0.0),
            );
            let ideal = self
                .monster
                .game
                .entity_ref(&self.monster.id.clone())
                .map(|entity| entity.ideal_yaw)
                .unwrap_or(0.0);
            let delta = ((body_yaw - ideal) % 360.0 + 360.0) % 360.0;
            if delta <= 45.0 || delta >= 315.0 {
                if attack_state == Q1AttackState::Melee {
                    self.melee_attack()?;
                } else if let Some(missile) = self.monster.spec.missile {
                    self.play(missile)?;
                }
                self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
                    entity.attack_state = Q1AttackState::Straight
                })?;
            }
            return Ok(());
        }
        if seen && self.try_attack()? {
            return Ok(());
        }
        if self.monster.controller.sliding {
            let yaw = self
                .monster
                .game
                .world
                .clone()
                .and_then(|world| {
                    self.monster
                        .game
                        .entity_ref(&world)
                        .map(|entity| entity.number("enemy_yaw"))
                })
                .unwrap_or(0.0);
            self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
                entity.ideal_yaw = yaw;
            })?;
            self.monster.change_yaw()?;
            let offset = if self.monster.controller.lefty { 90.0 } else { -90.0 };
            let (owned, ideal, classname) = self
                .monster
                .game
                .entity_ref(&self.monster.id.clone())
                .map(|entity| (entity.actor.clone(), entity.ideal_yaw, entity.classname.clone()))
                .ok_or_else(|| q1_error("Missing Q1 entity"))?;
            if self.monster.game.host.walk_move(&owned, ideal + offset, distance) {
                return Ok(());
            }
            self.monster.controller.lefty = !self.monster.controller.lefty;
            if classname == "monster_orb" {
                self.monster.controller.sliding = false;
                self.monster.game.update_entity(&self.monster.id.clone(), |entity| {
                    entity.attack_state = Q1AttackState::Straight
                })?;
                self.monster.controller.next_frame = self.monster.spec.run.to_string();
                return Ok(());
            }
            self.monster.game.host.walk_move(&owned, ideal - offset, distance);
            return Ok(());
        }
        let horde = self
            .monster
            .game
            .world
            .clone()
            .and_then(|world| {
                self.monster
                    .game
                    .entity_ref(&world)
                    .map(|entity| entity.number("isHordeMode"))
            })
            .unwrap_or(0.0)
            != 0.0;
        if horde {
            self.path_to_goal(distance)
        } else {
            self.monster.move_to_enemy(distance)
        }
    }

    /// Step toward the enemy, pathfinding while unseen (`pathToGoal`).
    pub fn path_to_goal(&mut self, distance: f64) -> Result<(), Q1Error> {
        let target = self.monster.target()?;
        let allow = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .map(|entity| entity.number("allowPathFind"))
            .unwrap_or(0.0);
        let Some(target) = target else {
            return self.monster.move_to_enemy(distance);
        };
        if allow == 0.0 {
            return self.monster.move_to_enemy(distance);
        }
        let world_number = |name: &str| {
            self.monster
                .game
                .world
                .clone()
                .and_then(|world| self.monster.game.entity_ref(&world).map(|entity| entity.number(name)))
                .unwrap_or(0.0)
        };
        let seen = world_number("enemy_visible") != 0.0;
        let range = world_number("enemy_range");
        let style = self
            .monster
            .game
            .entity_ref(&self.monster.id.clone())
            .map(|entity| entity.number("combat_style"))
            .unwrap_or(0.0);
        if (!seen || style == 2.0 && range > 1.0 || style == 3.0 && range > 2.0)
            && walk_mg3_path_to_goal(&mut self.monster, distance, target)? == Mg3PathResult::InProgress
        {
            return Ok(());
        }
        self.monster.move_to_enemy(distance)
    }
}

/// Attempt an attack with mg3 virtual dispatch (`tryAttack`).
/// Mirrors [`BaseMonster::try_attack`]; melee selection runs through
/// the melee hook and frames play through the mg3 driver.
fn mg3_default_try_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let target = monster.monster.target()?;
    let enemy = monster.monster.monster.enemy.clone();
    let (Some(target), Some(enemy)) = (target, enemy) else {
        return Ok(false);
    };
    let distance = monster.monster.range_distance(None)?;
    if monster.monster.spec.species == Q1MonsterSpecies::Demon {
        if distance < 120.0 {
            monster
                .monster
                .game
                .update_entity(&monster.monster.id.clone(), |entity| {
                    entity.attack_state = Q1AttackState::Melee;
                })?;
            return Ok(true);
        }
        let body = monster.monster.game.body(&monster.monster.id.clone())?;
        let Some(other) = monster.monster.game.host.bodies.read(&enemy) else {
            return Ok(false);
        };
        let delta = vsub(target, body.origin);
        let horizontal = (f64::from(delta.x).powi(2) + f64::from(delta.y).powi(2)).sqrt();
        let height = f64::from(other.bounds.max.z - other.bounds.min.z);
        if f64::from(body.origin.z + body.bounds.min.z) > f64::from(target.z + other.bounds.min.z) + height * 0.75
            || f64::from(body.origin.z + body.bounds.max.z) < f64::from(target.z + other.bounds.min.z) + height * 0.25
            || horizontal < 100.0
            || horizontal > 200.0 && monster.monster.game.host.random() < 0.9
        {
            return Ok(false);
        }
        monster
            .monster
            .game
            .sound_simple(&monster.monster.id.clone(), "demon/djump.wav")?;
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Missile;
            })?;
        return Ok(true);
    }
    let classname = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let specialized = classname == "monster_ogre" || monster.monster.spec.species == Q1MonsterSpecies::Shambler;
    let clear_shot = |monster: &mut Mg3Monster| -> Result<bool, Q1Error> {
        let Some(enemy) = monster.monster.monster.enemy.clone() else {
            return Ok(false);
        };
        let start = monster.monster.eye(None)?;
        let end = monster.monster.eye(Some(&enemy))?;
        let (Some(start), Some(end)) = (start, end) else {
            return Ok(false);
        };
        let trace = monster.monster.game.host.trace(&Q1TraceRequest {
            start,
            end,
            bounds: POINT,
            ignore: Some(monster.monster.id.clone()),
            monsters: true,
            missile: false,
        });
        Ok(trace.actor.as_ref().is_some_and(|actor| same_actor(actor, &enemy))
            && (monster.monster.spec.species == Q1MonsterSpecies::Wizard || !(trace.in_open && trace.in_water)))
    };
    if !specialized && monster.monster.spec.species != Q1MonsterSpecies::Wizard && !clear_shot(monster)? {
        return Ok(false);
    }
    if distance < 120.0
        && monster.monster.spec.melee
        && (!specialized || monster.monster.game.can_damage(&enemy, &monster.monster.id.clone()))
    {
        if specialized {
            monster
                .monster
                .game
                .update_entity(&monster.monster.id.clone(), |entity| {
                    entity.attack_state = Q1AttackState::Melee;
                })?;
        } else {
            monster.melee_attack()?;
        }
        return Ok(true);
    }
    if monster.monster.spec.missile.is_none() || monster.monster.game.time < monster.monster.monster.attack_finished {
        return Ok(false);
    }
    if distance >= 1000.0 || monster.monster.spec.species == Q1MonsterSpecies::Shambler && distance > 600.0 {
        if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
            mg3_wizard_move(monster, false)?;
        }
        return Ok(false);
    }
    if (specialized || monster.monster.spec.species == Q1MonsterSpecies::Wizard) && !clear_shot(monster)? {
        if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
            mg3_wizard_move(monster, false)?;
        }
        return Ok(false);
    }
    if specialized {
        let delay = (if monster.monster.spec.species == Q1MonsterSpecies::Shambler {
            2.0
        } else {
            1.0
        }) + 2.0 * monster.monster.game.host.random();
        monster.attack_finished(delay);
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Missile;
            })?;
        return Ok(true);
    }
    if distance < 120.0 && monster.monster.spec.species != Q1MonsterSpecies::Wizard {
        monster.monster.monster.attack_finished = 0.0;
    }
    let chance = if distance < 120.0 {
        0.9
    } else if distance < 500.0 {
        if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
            0.6
        } else if monster.monster.spec.melee {
            0.2
        } else {
            0.4
        }
    } else if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
        0.2
    } else if monster.monster.spec.melee {
        0.05
    } else {
        0.1
    };
    if monster.monster.game.host.random() >= chance {
        if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
            mg3_wizard_move(monster, distance < 500.0)?;
        }
        return Ok(false);
    }
    if monster.monster.spec.species == Q1MonsterSpecies::Wizard {
        monster.monster.controller.sliding = false;
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Missile;
            })?;
        return Ok(true);
    }
    if monster.monster.spec.species == Q1MonsterSpecies::Zombie {
        let rolled = monster.monster.game.host.random();
        monster.play(if rolled < 0.3 {
            "zombie_atta1"
        } else if rolled < 0.6 {
            "zombie_attb1"
        } else {
            "zombie_attc1"
        })?;
    } else if let Some(missile) = monster.monster.spec.missile {
        monster.play(missile)?;
    }
    let delay = 2.0 * monster.monster.game.host.random();
    monster.attack_finished(delay);
    Ok(true)
}

/// Slide a wizard sideways (`wizardMove`).
fn mg3_wizard_move(monster: &mut Mg3Monster, sliding: bool) -> Result<(), Q1Error> {
    if monster.monster.controller.sliding == sliding {
        return Ok(());
    }
    monster.monster.controller.sliding = sliding;
    monster.play(if sliding { "wiz_side1" } else { "wiz_run1" })
}

/// React to pain with mg3 virtual dispatch (`pain`). Mirrors
/// [`BaseMonster::pain`]; retaliation acquires through mg3 policy and
/// frames play through the mg3 driver.
fn mg3_default_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    let flags = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.movement_flags)
        .unwrap_or(0);
    if flags & 32 != 0 {
        monster.retaliate(attacker.as_ref())?;
    }
    match monster.monster.spec.species {
        Q1MonsterSpecies::Zombie => {
            let owned = monster
                .monster
                .game
                .entity_ref(&monster.monster.id.clone())
                .map(|entity| entity.actor.clone())
                .ok_or_else(|| q1_error("Missing Q1 entity"))?;
            monster.monster.game.host.combat.set_health(&owned, 60.0)?;
            if damage < 9.0 || monster.monster.controller.in_pain == 2.0 {
                return Ok(());
            }
            if damage >= 25.0 {
                monster.monster.controller.in_pain = 2.0;
                return monster.play("zombie_paine1");
            }
            if monster.monster.controller.in_pain != 0.0 {
                monster.monster.monster.pain_finished = monster.monster.game.time + 3.0;
                return Ok(());
            }
            if monster.monster.monster.pain_finished > monster.monster.game.time {
                monster.monster.controller.in_pain = 2.0;
                return monster.play("zombie_paine1");
            }
            monster.monster.controller.in_pain = 1.0;
            let rolled = monster.monster.game.host.random();
            monster.play(if rolled < 0.25 {
                "zombie_paina1"
            } else if rolled < 0.5 {
                "zombie_painb1"
            } else if rolled < 0.75 {
                "zombie_painc1"
            } else {
                "zombie_paind1"
            })
        }
        Q1MonsterSpecies::Fish => monster.play("f_pain1"),
        Q1MonsterSpecies::Wizard => {
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "wizard/wpain.wav")?;
            if monster.monster.game.host.random() * 70.0 > damage {
                return Ok(());
            }
            monster.play("wiz_pain1")
        }
        Q1MonsterSpecies::Shambler => {
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "shambler/shurt2.wav")?;
            if monster.monster.game.health(&monster.monster.id.clone()) <= 0.0
                || monster.monster.game.host.random() * 400.0 > damage
                || monster.monster.monster.pain_finished > monster.monster.game.time
            {
                return Ok(());
            }
            monster.monster.monster.pain_finished = monster.monster.game.time + 2.0;
            monster.play("sham_pain1")
        }
        Q1MonsterSpecies::Demon => {
            if monster
                .monster
                .game
                .entity_ref(&monster.monster.id.clone())
                .and_then(|entity| entity.touch.clone())
                .is_some()
                || monster.monster.monster.pain_finished > monster.monster.game.time
            {
                return Ok(());
            }
            monster.monster.monster.pain_finished = monster.monster.game.time + 1.0;
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "demon/dpain1.wav")?;
            if monster.monster.game.host.random() * 200.0 > damage {
                return Ok(());
            }
            monster.play("demon1_pain1")
        }
        Q1MonsterSpecies::Knight => {
            if monster.monster.monster.pain_finished > monster.monster.game.time {
                return Ok(());
            }
            let rolled = monster.monster.game.host.random();
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "knight/khurt.wav")?;
            monster.monster.monster.pain_finished = monster.monster.game.time + 1.0;
            monster.play(if rolled < 0.85 { "knight_pain1" } else { "knight_painb1" })
        }
        Q1MonsterSpecies::Enforcer => {
            let rolled = monster.monster.game.host.random();
            if monster.monster.monster.pain_finished > monster.monster.game.time {
                return Ok(());
            }
            monster.monster.game.sound_simple(
                &monster.monster.id.clone(),
                if rolled < 0.5 {
                    "enforcer/pain1.wav"
                } else {
                    "enforcer/pain2.wav"
                },
            )?;
            monster.monster.monster.pain_finished = monster.monster.game.time + if rolled < 0.7 { 1.0 } else { 2.0 };
            monster.play(if rolled < 0.2 {
                "enf_paina1"
            } else if rolled < 0.4 {
                "enf_painb1"
            } else if rolled < 0.7 {
                "enf_painc1"
            } else {
                "enf_paind1"
            })
        }
        Q1MonsterSpecies::Ogre => {
            if monster.monster.monster.pain_finished > monster.monster.game.time {
                return Ok(());
            }
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "ogre/ogpain1.wav")?;
            let rolled = monster.monster.game.host.random();
            monster.monster.monster.pain_finished = monster.monster.game.time + if rolled < 0.75 { 1.0 } else { 2.0 };
            monster.play(if rolled < 0.25 {
                "ogre_pain1"
            } else if rolled < 0.5 {
                "ogre_painb1"
            } else if rolled < 0.75 {
                "ogre_painc1"
            } else if rolled < 0.88 {
                "ogre_paind1"
            } else {
                "ogre_paine1"
            })
        }
        Q1MonsterSpecies::Hellknight => {
            if monster.monster.monster.pain_finished > monster.monster.game.time {
                return Ok(());
            }
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "hknight/pain1.wav")?;
            if monster.monster.game.time - monster.monster.monster.pain_finished <= 5.0
                && monster.monster.game.host.random() * 30.0 > damage
            {
                return Ok(());
            }
            monster.monster.monster.pain_finished = monster.monster.game.time + 1.0;
            monster.play("hknight_pain1")
        }
        Q1MonsterSpecies::Shalrath => {
            if monster.monster.monster.pain_finished > monster.monster.game.time {
                return Ok(());
            }
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "shalrath/pain.wav")?;
            monster.monster.monster.pain_finished = monster.monster.game.time + 3.0;
            monster.play("shal_pain1")
        }
        Q1MonsterSpecies::Tarbaby | Q1MonsterSpecies::Boss | Q1MonsterSpecies::Oldone => Ok(()),
        _ => Ok(()),
    }
}

fn mg3_monster_jump_touch_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let other = other.clone();
    let mut monster = Mg3Monster::load(game, id)?;
    monster_jump_touch(&mut monster.monster, &other)?;
    monster.finish()
}

fn mg3_monster_frame_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    let next = monster.monster.controller.next_frame.clone();
    monster.play(&next)?;
    monster.finish()
}

fn mg3_monster_route_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    monster.update_route()?;
    monster.finish()
}

fn mg3_monster_start_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    monster.start()?;
    monster.finish()
}

fn mg3_monster_stand_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    let stand = monster.monster.spec.stand;
    monster.play(stand)?;
    monster.finish()
}

fn mg3_monster_found_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    if let Some(enemy) = monster.monster.monster.enemy.clone() {
        monster.found(&enemy)?;
    }
    monster.finish()
}

fn mg3_monster_pain_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
    damage: f64,
) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    monster.pain(attacker.as_ref(), damage)?;
    monster.finish()
}

fn mg3_monster_die_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    monster.die(attacker.as_ref())?;
    monster.finish()
}

fn mg3_monster_use_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    monster.use_monster(activator.as_ref())?;
    monster.finish()
}

fn mg3_boss_awake_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    monster.monster.awake(activator.as_ref())?;
    monster.finish()
}

/// Register mg3 monster callbacks for a prefix. The source must be
/// registered first through [`register_mg3_monster_source`].
pub fn register_mg3_monster_callbacks(game: &mut Q1EntityServices, prefix: &str) -> Result<(), Q1Error> {
    mg3_monster_source(prefix)?;
    let action = |handler: Q1ActionHandler| Q1CallbackHandlers {
        action: Some(handler),
        ..Default::default()
    };
    game.named.register(
        &format!("{prefix}:monster_jump_touch"),
        Q1CallbackHandlers {
            touch: Some(mg3_monster_jump_touch_handler as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:monster_frame"),
        action(mg3_monster_frame_handler as Q1ActionHandler),
    )?;
    game.named.register(
        &format!("{prefix}:monster_route"),
        action(mg3_monster_route_handler as Q1ActionHandler),
    )?;
    game.named.register(
        &format!("{prefix}:monster_start"),
        action(mg3_monster_start_handler as Q1ActionHandler),
    )?;
    game.named.register(
        &format!("{prefix}:monster_stand"),
        action(mg3_monster_stand_handler as Q1ActionHandler),
    )?;
    game.named.register(
        &format!("{prefix}:monster_found"),
        action(mg3_monster_found_handler as Q1ActionHandler),
    )?;
    game.named.register(
        &format!("{prefix}:monster_pain"),
        Q1CallbackHandlers {
            pain: Some(mg3_monster_pain_handler as Q1PainHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:monster_die"),
        Q1CallbackHandlers {
            die: Some(mg3_monster_die_handler as Q1DieHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:monster_use"),
        Q1CallbackHandlers {
            use_callback: Some(mg3_monster_use_handler as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:boss_awake"),
        Q1CallbackHandlers {
            use_callback: Some(mg3_boss_awake_handler as Q1UseHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}
