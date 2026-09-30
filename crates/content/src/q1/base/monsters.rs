//! Base monster AI (`src/content/q1/base/monsters.ts`).
//!
//! ai.qc/fight.qc and base monster QuakeC. Copyright (C) 1996-2022 id
//! Software LLC. GPL-2.0-or-later.
//!
//! Source animation state only. Bodies, health, armor, damage and
//! callback ordering stay in the shared host.
//!
//! The donor keeps one `BaseMonster` object per entity behind a map.
//! Here the per-entity scalars live in [`BaseMonsterState`] in the
//! creature store; callbacks load a [`BaseMonster`] view, run the
//! donor logic, then finish it back. Mission-pack lanes reuse the
//! frame driver through [`register_monster_source`] with their own
//! frames, actions, and controller stores.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::{same_actor, ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::monsters::monster_target_eligible;
use crate::q1::base::animation::{MonsterAi, MonsterFrame, MonsterOperation};
use crate::q1::base::frames::monster_frame;
use crate::q1::base::monster_actions::{monster_action, monster_jump_touch};
use crate::q1::base::projectiles::cast_lightning;
use crate::q1::base::species::{precache_id1_monster, species_by_classname, MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::{Q1AttackState, Q1Monster, Q1MonsterMode, Q1MonsterSpecies};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::host::Q1GoalMode;
use crate::q1::foundation::monsters::{path_end_time, throw_gib, throw_head};
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, yaw_for, Q1Edition, Q1Effect, Q1Event, Q1MoveType, Q1Solid,
    Q1SoundChannel, Q1TraceRequest, POINT, ZERO,
};
use crate::q1::{q1_error, Q1Error};
use crate::value::{boolean, num, obj, str as save_str, SaveJson, SaveReader};

/// Source animation state for one monster (`BaseMonster` fields beyond
/// the shared `Q1Monster`). Checkpointed per monster.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseMonsterState {
    /// Monster species.
    pub species: Q1MonsterSpecies,
    /// Callback prefix.
    pub prefix: String,
    /// Current frame name.
    pub current_frame: String,
    /// Next frame name.
    pub next_frame: String,
    /// Zombie pain stage.
    pub in_pain: f64,
    /// Generic action counter.
    pub counter: f64,
    /// Idle-sound cooldown.
    pub idle_until: f64,
    /// Strafe direction.
    pub lefty: bool,
    /// Whether circle-strafing.
    pub sliding: bool,
    /// Lightning bolts cast.
    pub lightning_count: f64,
    /// Whether death was counted.
    pub counted_death: bool,
}

impl BaseMonsterState {
    /// Fresh controller for a species.
    #[must_use]
    pub fn new(species: Q1MonsterSpecies, prefix: String, stand: &str) -> Self {
        Self {
            species,
            prefix,
            current_frame: stand.to_string(),
            next_frame: stand.to_string(),
            in_pain: 0.0,
            counter: 0.0,
            idle_until: 0.0,
            lefty: false,
            sliding: false,
            lightning_count: 0.0,
            counted_death: false,
        }
    }

    /// Capture controller checkpoint state (`capture`).
    #[must_use]
    pub fn capture(&self) -> SaveJson {
        obj(vec![
            ("currentFrame", save_str(&self.current_frame)),
            ("nextFrame", save_str(&self.next_frame)),
            ("inPain", num(self.in_pain)),
            ("counter", num(self.counter)),
            ("idleUntil", num(self.idle_until)),
            ("lefty", boolean(self.lefty)),
            ("sliding", boolean(self.sliding)),
            ("lightningCount", num(self.lightning_count)),
            ("countedDeath", boolean(self.counted_death)),
        ])
    }

    /// Restore controller checkpoint state (`restore`).
    pub fn restore(species: Q1MonsterSpecies, prefix: String, reader: SaveReader) -> Result<Self, Q1Error> {
        Self::restore_with_source(species, prefix, None, reader)
    }

    /// Restore with source-frame validation (`restore`).
    pub fn restore_with_source(
        species: Q1MonsterSpecies,
        prefix: String,
        source: Option<&MonsterSource>,
        reader: SaveReader,
    ) -> Result<Self, Q1Error> {
        let current_frame = reader.field("currentFrame").string()?;
        let next_frame = reader.field("nextFrame").string()?;
        let known = |name: &str| {
            monster_frame(name).is_some()
                || source.is_some_and(|source| {
                    source.frames.is_some_and(|frames| frames.contains_key(name))
                        || source.actions.is_some_and(|actions| actions.contains_key(name))
                })
        };
        if !known(&current_frame) || !known(&next_frame) {
            return Err(Q1Error::from(reader.fail("unknown source monster continuation")));
        }
        Ok(Self {
            species,
            prefix,
            current_frame,
            next_frame,
            in_pain: reader.field("inPain").number()?,
            counter: reader.field("counter").number()?,
            idle_until: reader.field("idleUntil").number()?,
            lefty: reader.field("lefty").boolean()?,
            sliding: reader.field("sliding").boolean()?,
            lightning_count: reader.field("lightningCount").number()?,
            counted_death: reader.field("countedDeath").boolean()?,
        })
    }
}

/// Monster frame-action handler (`BaseMonsterSource["actions"]` value).
pub type MonsterActionHandler = fn(monster: &mut BaseMonster) -> Result<(), Q1Error>;

/// Alternate monster source (`BaseMonsterSource`).
#[derive(Debug, Clone, Copy)]
pub struct MonsterSource {
    /// Callback prefix.
    pub prefix: &'static str,
    /// Alternate frames, if any.
    pub frames: Option<&'static HashMap<String, MonsterFrame>>,
    /// Alternate actions, if any.
    pub actions: Option<&'static HashMap<String, MonsterActionHandler>>,
}

/// Controller loader for a monster source.
pub type MonsterControllerLoad = fn(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error>;
/// Controller store for a monster source.
pub type MonsterControllerStore =
    fn(game: &Q1EntityServices, id: &ActorId, controller: BaseMonsterState) -> Result<(), Q1Error>;

/// Monster source registration for the frame driver.
#[derive(Debug, Clone, Copy)]
pub struct MonsterSourceRegistration {
    /// Source frames and actions.
    pub source: MonsterSource,
    /// Controller loader.
    pub load: MonsterControllerLoad,
    /// Controller store.
    pub store: MonsterControllerStore,
}

fn monster_sources() -> &'static Mutex<HashMap<&'static str, MonsterSourceRegistration>> {
    static SOURCES: OnceLock<Mutex<HashMap<&'static str, MonsterSourceRegistration>>> = OnceLock::new();
    SOURCES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn base_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = super::creatures::monster_controller(game, id, classname)?;
    let spec = species_by_classname(classname).ok_or_else(|| q1_error("unknown base monster"))?;
    Ok((controller, spec))
}

/// Register a monster source for the frame driver. Re-registering a
/// prefix keeps the first entry, so repeated base registration across
/// games is safe.
pub fn register_monster_source(registration: MonsterSourceRegistration) {
    let mut sources = monster_sources()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    sources.entry(registration.source.prefix).or_insert(registration);
}

fn monster_source(prefix: &str) -> Result<MonsterSourceRegistration, Q1Error> {
    monster_sources()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(prefix)
        .copied()
        .ok_or_else(|| q1_error(format!("Unknown Q1 monster source: {prefix}")))
}

fn default_monster_state(species: Q1MonsterSpecies, path: &str) -> Q1Monster {
    Q1Monster {
        species,
        mode: Q1MonsterMode::Stand,
        frame_index: 0,
        sequence: Vec::new(),
        first_frame: 0,
        enemy: None,
        old_enemy: None,
        path: path.to_string(),
        pause_until: 0.0,
        attack_finished: 0.0,
        pain_finished: 0.0,
        search_until: 0.0,
        death_drop: false,
        refired: false,
    }
}

/// Base monster controller view (`BaseMonster`). Loaded per callback
/// and finished back; see the module docs.
pub struct BaseMonster<'g> {
    /// Entity services.
    pub game: &'g mut Q1EntityServices,
    /// Monster actor.
    pub id: ActorId,
    /// Spawn defaults.
    pub spec: &'static MonsterSpecies,
    /// Callback prefix.
    pub prefix: String,
    /// Animation controller.
    pub controller: BaseMonsterState,
    /// Shared monster state.
    pub monster: Q1Monster,
}

impl<'g> BaseMonster<'g> {
    /// Load a monster controller view.
    pub fn load(game: &'g mut Q1EntityServices, id: &ActorId) -> Result<Self, Q1Error> {
        let id = id.clone();
        let entity = game
            .entity_ref(&id)
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let prefix = entity
            .fields
            .get("source.monsterCallbackPrefix")
            .cloned()
            .unwrap_or_else(|| String::from("base"));
        let source = monster_source(&prefix)?;
        let (controller, spec) = (source.load)(game, &id, &entity.classname)?;
        let monster = entity
            .monster
            .clone()
            .unwrap_or_else(|| default_monster_state(spec.species, &entity.target));
        Ok(Self {
            game,
            id,
            spec,
            prefix,
            controller,
            monster,
        })
    }

    /// Write the controller and monster state back. Released entities
    /// were cleaned by the release hook and are not resurrected.
    pub fn finish(self) -> Result<(), Q1Error> {
        if self.game.entity_ref(&self.id).is_none() {
            return Ok(());
        }
        let store = monster_source(&self.prefix)?.store;
        store(&*self.game, &self.id, self.controller)?;
        let monster = self.monster;
        self.game
            .update_entity(&self.id, |entity| entity.monster = Some(monster))
    }

    /// Build a fresh controller for spawning.
    pub fn spawn_new(
        game: &'g mut Q1EntityServices,
        id: &ActorId,
        spec: &'static MonsterSpecies,
    ) -> Result<Self, Q1Error> {
        let id = id.clone();
        let controller = BaseMonsterState::new(spec.species, String::from("base"), spec.stand);
        let target = game
            .entity_ref(&id)
            .map(|entity| entity.target.clone())
            .unwrap_or_default();
        let monster = game
            .entity_ref(&id)
            .and_then(|entity| entity.monster.clone())
            .unwrap_or_else(|| default_monster_state(spec.species, &target));
        game.update_entity(&id, |entity| {
            entity
                .fields
                .insert(String::from("source.monsterCallbackPrefix"), String::from("base"));
            if entity.monster.is_none() {
                entity.monster = Some(monster.clone());
            }
        })?;
        super::creatures::store_monster_controller(game, &id, controller.clone())?;
        Ok(Self {
            game,
            id,
            spec,
            prefix: String::from("base"),
            controller,
            monster,
        })
    }

    fn owned(&self) -> Result<OwnedActor, Q1Error> {
        self.game
            .entity_ref(&self.id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))
    }

    /// Current enemy.
    #[must_use]
    pub fn enemy(&self) -> Option<ActorId> {
        self.monster.enemy.clone()
    }

    /// Set the current enemy.
    pub fn set_enemy(&mut self, enemy: Option<ActorId>) {
        self.monster.enemy = enemy;
    }

    /// Monster origin.
    pub fn origin(&self) -> Result<Vec3, Q1Error> {
        self.game.body(&self.id).map(|body| body.origin)
    }

    /// Enemy origin, if any.
    pub fn target(&mut self) -> Result<Option<Vec3>, Q1Error> {
        let Some(enemy) = self.monster.enemy.clone() else {
            return Ok(None);
        };
        Ok(self.game.host.bodies.read(&enemy).map(|body| body.origin))
    }

    /// Distance to the enemy, if any.
    pub fn distance(&mut self) -> Result<f64, Q1Error> {
        let Some(target) = self.target()? else {
            return Ok(f64::INFINITY);
        };
        Ok(f64::from(length(vsub(target, self.origin()?))))
    }

    /// Play a named frame (`play`).
    pub fn play(&mut self, name: &str) -> Result<(), Q1Error> {
        if !self.game.is_live(&self.id) {
            return Ok(());
        }
        if self.game.health(&self.id) > 0.0 {
            if let Some(enemy) = self.monster.enemy.clone() {
                if !monster_target_eligible(self.game.health(&enemy), self.game.monster_target(&enemy).as_ref()) {
                    let previous = self.monster.old_enemy.clone();
                    self.monster.enemy = match previous {
                        Some(previous)
                            if monster_target_eligible(
                                self.game.health(&previous),
                                self.game.monster_target(&previous).as_ref(),
                            ) =>
                        {
                            Some(previous)
                        }
                        _ => None,
                    };
                    self.monster.old_enemy = None;
                    self.game
                        .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Straight)?;
                    let fallback = if self.monster.enemy.is_none() {
                        if self.route()?.is_none() {
                            self.spec.stand
                        } else {
                            self.spec.walk
                        }
                    } else {
                        self.spec.run
                    };
                    return self.play(fallback);
                }
            }
        }
        let source = monster_source(&self.prefix)?;
        let frame = source
            .source
            .frames
            .and_then(|frames| frames.get(name).copied())
            .or_else(|| monster_frame(name).copied())
            .ok_or_else(|| q1_error(format!("Missing Q1 source animation {name}")))?;
        self.controller.current_frame = name.to_string();
        self.controller.next_frame = frame.next.to_string();
        let model_frame = frame.frame;
        self.game.update_entity(&self.id, |entity| entity.frame = model_frame)?;
        self.game
            .schedule(&self.id, 0.1, &format!("{}:monster_frame", self.prefix))?;
        if self.game.options().edition == Q1Edition::Classic {
            if name == "boss_idle1" || name == "f_death2" {
                return Ok(());
            }
            if name == "f_death21" {
                self.game
                    .update_entity(&self.id, |entity| entity.solid = Q1Solid::None)?;
                return self.game.link(&self.id);
            }
            if name == "sham_magic11" && self.game.options().skill == 3 {
                return cast_lightning(self);
            }
        }
        for op in frame.operations {
            if !self.game.is_live(&self.id) {
                return Ok(());
            }
            match op {
                MonsterOperation::Ai { mode, distance } => self.ai(*mode, *distance)?,
                MonsterOperation::Solid { solid } => {
                    self.game.update_entity(&self.id, |entity| entity.solid = *solid)?;
                    self.game.link(&self.id)?;
                }
                MonsterOperation::Lightstyle { pattern } => {
                    self.game.host.emit(Q1Event::Lightstyle {
                        style: 0,
                        pattern: (*pattern).to_string(),
                    });
                }
                MonsterOperation::Action { name } => match source.source.actions.and_then(|actions| actions.get(*name))
                {
                    Some(action) => action(self)?,
                    None => monster_action(self, name)?,
                },
                MonsterOperation::Sound {
                    path,
                    channel,
                    attenuation,
                    comparison,
                    chance,
                } => {
                    let draw = if chance.is_some() { self.game.host.random() } else { 0.0 };
                    let play = match chance {
                        None => true,
                        Some(chance) => match comparison {
                            crate::q1::base::animation::SoundComparison::Greater => draw > *chance,
                            crate::q1::base::animation::SoundComparison::Less => draw < *chance,
                        },
                    };
                    if play {
                        self.game.sound(&self.id.clone(), path, *channel, *attenuation, 1.0)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Delay the next frame (`delay`).
    pub fn delay(&mut self, seconds: f64) -> Result<(), Q1Error> {
        self.game
            .schedule(&self.id, seconds, &format!("{}:monster_frame", self.prefix))
    }

    /// Update the monster yaw (`changeYaw`).
    pub fn change_yaw(&mut self) -> Result<(), Q1Error> {
        let owned = self.owned()?;
        self.game.host.change_yaw(&owned);
        Ok(())
    }

    /// Face the enemy (`face`).
    pub fn face(&mut self) -> Result<(), Q1Error> {
        let Some(target) = self.target()? else { return Ok(()) };
        let origin = self.origin()?;
        let yaw = yaw_for(vsub(target, origin));
        self.game.update_entity(&self.id, |entity| entity.ideal_yaw = yaw)?;
        self.change_yaw()
    }

    /// Save and return the angle basis (`makeVectors`).
    pub fn make_vectors(&mut self) -> Result<crate::q1::foundation::types::Q1Basis, Q1Error> {
        let angles = self.game.body(&self.id).map(|body| body.angles)?;
        let adjusted = if self.game.options().edition == Q1Edition::Rerelease {
            Vec3 {
                x: -angles.x,
                y: angles.y,
                z: angles.z,
            }
        } else {
            angles
        };
        Ok(self.game.make_vectors(adjusted))
    }

    /// Eye position for an actor (`eye`).
    pub fn eye(&mut self, target: Option<&ActorId>) -> Result<Option<Vec3>, Q1Error> {
        let id = target.cloned().unwrap_or_else(|| self.id.clone());
        let Some(body) = self.game.host.bodies.read(&id) else {
            return Ok(None);
        };
        let entity = self.game.entity_ref(&id).cloned();
        let offset = match entity {
            Some(ref entity) if entity.fields.contains_key("view_ofs") => entity.vector("view_ofs"),
            Some(ref entity)
                if entity
                    .monster
                    .as_ref()
                    .is_some_and(|monster| monster.species == Q1MonsterSpecies::Fish) =>
            {
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 10.0,
                }
            }
            _ => Vec3 {
                x: 0.0,
                y: 0.0,
                z: self
                    .game
                    .monster_target(&id)
                    .map(|seen| seen.view_height as f32)
                    .unwrap_or(25.0),
            },
        };
        Ok(Some(vadd(body.origin, offset)))
    }

    /// Eye-to-eye range to a target (`rangeDistance`).
    pub fn range_distance(&mut self, target: Option<&ActorId>) -> Result<f64, Q1Error> {
        let enemy = self.monster.enemy.clone();
        let start = self.eye(None)?;
        let end = self.eye(target.or(enemy.as_ref()))?;
        match (start, end) {
            (Some(start), Some(end)) => Ok(f64::from(length(vsub(end, start)))),
            _ => Ok(f64::INFINITY),
        }
    }

    /// Whether a target is visible (`visible`).
    pub fn visible(&mut self, target: Option<&ActorId>) -> Result<bool, Q1Error> {
        let enemy = self.monster.enemy.clone();
        let target = target.cloned().or(enemy);
        let Some(target) = target else { return Ok(false) };
        let start = self.eye(None)?;
        let end = self.eye(Some(&target))?;
        let (Some(start), Some(end)) = (start, end) else {
            return Ok(false);
        };
        let trace = self.game.host.trace(&Q1TraceRequest {
            start,
            end,
            bounds: POINT,
            ignore: Some(self.id.clone()),
            monsters: false,
            missile: false,
        });
        Ok(trace.fraction == 1.0 && !(trace.in_open && trace.in_water))
    }

    /// Acquire a target (`found`).
    pub fn found(&mut self, target: &ActorId) -> Result<(), Q1Error> {
        let target = target.clone();
        self.monster.enemy = Some(target.clone());
        self.monster.mode = Q1MonsterMode::Run;
        self.monster.search_until = self.game.time + 5.0;
        if let Some(mission) = self.game.monster_missions.get_mut(&self.id) {
            mission.found_target();
        }
        self.attack_finished(1.0);
        if self.game.is_player(&target) {
            self.game.sight_entity = Some(self.id.clone());
            self.game.sight_time = self.game.time;
        }
        if let Some(body) = self.game.host.bodies.read(&target) {
            let origin = self.origin()?;
            let yaw = yaw_for(vsub(body.origin, origin));
            self.game.update_entity(&self.id, |entity| entity.ideal_yaw = yaw)?;
        }
        let mut sound = self.spec.sight.to_string();
        if self.spec.species == Q1MonsterSpecies::Enforcer && self.game.options().edition == Q1Edition::Classic {
            let rolled = (self.game.host.random() * 3.0 + 0.5).floor() as i32;
            let track = if rolled == 1 {
                1
            } else if rolled == 2 {
                2
            } else if rolled == 0 {
                3
            } else {
                4
            };
            sound = format!("enforcer/sight{track}.wav");
        }
        if !sound.is_empty() {
            self.game.sound_simple(&self.id.clone(), &sound)?;
        }
        self.controller.next_frame = self.spec.run.to_string();
        self.delay(0.1)
    }

    /// Scan for a target (`findTarget`).
    pub fn find_target(&mut self) -> Result<bool, Q1Error> {
        let spawnflags = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.spawnflags)
            .unwrap_or(0);
        let ambush = self
            .game
            .monster_missions
            .get(&self.id)
            .map(|mission| mission.ambush())
            .unwrap_or(spawnflags & 3 != 0);
        let candidate = match self.game.sight_entity.clone() {
            Some(sight) if self.game.sight_time >= self.game.time - 0.1 && !ambush => self
                .game
                .entity_ref(&sight)
                .and_then(|entity| entity.monster.as_ref())
                .and_then(|monster| monster.enemy.clone()),
            _ => {
                let owned = self.owned()?;
                self.game.host.check_client(&owned)
            }
        };
        let Some(candidate) = candidate else { return Ok(false) };
        if self.game.health(&candidate) <= 0.0 {
            return Ok(false);
        }
        let observed = self.game.monster_target(&candidate);
        let Some(observed) = observed else { return Ok(false) };
        if observed.invisible || observed.notarget {
            return Ok(false);
        }
        let Some(body) = self.game.host.bodies.read(&candidate) else {
            return Ok(false);
        };
        let origin = self.origin()?;
        let delta = vsub(body.origin, origin);
        let distance = self.range_distance(Some(&candidate))?;
        if distance >= 1000.0 || !self.visible(Some(&candidate))? {
            return Ok(false);
        }
        let basis = self.make_vectors()?;
        let front = dot(normalize(delta), basis.forward) > 0.3;
        if distance >= 500.0 && !front
            || (120.0..500.0).contains(&distance)
                && observed.hostile_until.is_none_or(|until| until < self.game.time)
                && !front
        {
            return Ok(false);
        }
        self.found(&candidate)?;
        Ok(true)
    }

    /// Current patrol goal (`route`).
    pub fn route(&mut self) -> Result<Option<ActorId>, Q1Error> {
        if let Some(mission) = self.game.monster_missions.get(&self.id) {
            return Ok(mission.route());
        }
        if self.monster.path.is_empty() {
            return Ok(None);
        }
        Ok(self.game.find(&self.monster.path.clone()).first().cloned())
    }

    /// Advance the patrol route (`updateRoute`).
    pub fn update_route(&mut self) -> Result<(), Q1Error> {
        let goal = self.route()?;
        if goal.is_none() || self.monster.pause_until > self.game.time {
            self.monster.mode = Q1MonsterMode::Stand;
            let stand = self.spec.stand;
            return self.play(stand);
        }
        if self.monster.mode == Q1MonsterMode::Stand {
            self.monster.mode = Q1MonsterMode::Walk;
            let walk = self.spec.walk;
            return self.play(walk);
        }
        Ok(())
    }

    /// Run one steering step (`ai`).
    pub fn ai(&mut self, mode: MonsterAi, distance: f64) -> Result<(), Q1Error> {
        match mode {
            MonsterAi::Stand => self.monster.mode = Q1MonsterMode::Stand,
            MonsterAi::Walk => self.monster.mode = Q1MonsterMode::Walk,
            MonsterAi::Run => self.monster.mode = Q1MonsterMode::Run,
            _ => {}
        }
        match mode {
            MonsterAi::Stand => {
                if !self.find_target()?
                    && self.game.time > self.monster.pause_until
                    && self.route()?.is_some()
                    && self.spec.walk != self.spec.stand
                {
                    let walk = self.spec.walk;
                    self.play(walk)?;
                }
                Ok(())
            }
            MonsterAi::Turn => {
                if !self.find_target()? {
                    self.face()?;
                }
                Ok(())
            }
            MonsterAi::Walk => {
                if self.find_target()? || self.game.time < self.monster.pause_until {
                    return Ok(());
                }
                let Some(path) = self.route()? else {
                    self.monster.pause_until = path_end_time(self.game.time);
                    let stand = self.spec.stand;
                    return self.play(stand);
                };
                let owned = self.owned()?;
                self.game.host.move_to_goal(&owned, &path, distance, None);
                Ok(())
            }
            MonsterAi::Run => self.run(distance),
            MonsterAi::Face => self.face(),
            MonsterAi::Charge => {
                self.face()?;
                if let Some(enemy) = self.monster.enemy.clone() {
                    let owned = self.owned()?;
                    self.game.host.move_to_goal(&owned, &enemy, distance, None);
                }
                Ok(())
            }
            MonsterAi::ChargeSide => {
                self.face()?;
                if let Some(target) = self.target()? {
                    let basis = self.make_vectors()?;
                    let origin = self.origin()?;
                    let yaw = yaw_for(vsub(vsub(target, vscale(basis.right, 30.0)), origin));
                    let owned = self.owned()?;
                    self.game.host.walk_move(&owned, yaw, 20.0);
                }
                Ok(())
            }
            MonsterAi::MeleeSide => {
                self.ai(MonsterAi::ChargeSide, 0.0)?;
                self.melee(60.0, 3.0, 3, true)?;
                Ok(())
            }
            MonsterAi::Melee => {
                self.melee(60.0, 3.0, 3, false)?;
                Ok(())
            }
            MonsterAi::Pain => {
                let yaw = self.game.body(&self.id).map(|body| body.angles.y)? + 180.0;
                let owned = self.owned()?;
                self.game.host.walk_move(&owned, f64::from(yaw), distance);
                Ok(())
            }
            MonsterAi::Painforward | MonsterAi::Forward => {
                let yaw = self.game.body(&self.id).map(|body| body.angles.y)?;
                let owned = self.owned()?;
                self.game.host.walk_move(&owned, f64::from(yaw), distance);
                Ok(())
            }
        }
    }

    /// Run combat steering (`run`).
    pub fn run(&mut self, distance: f64) -> Result<(), Q1Error> {
        let eligible = match self.monster.enemy.clone() {
            Some(enemy) => monster_target_eligible(self.game.health(&enemy), self.game.monster_target(&enemy).as_ref()),
            None => false,
        };
        if !eligible {
            self.game
                .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Straight)?;
            if let Some(old) = self.monster.old_enemy.clone() {
                if monster_target_eligible(self.game.health(&old), self.game.monster_target(&old).as_ref()) {
                    self.monster.enemy = Some(old);
                    self.monster.old_enemy = None;
                } else {
                    self.monster.enemy = None;
                    self.monster.old_enemy = None;
                    let fallback = if self.route()?.is_none() {
                        self.spec.stand
                    } else {
                        self.spec.walk
                    };
                    return self.play(fallback);
                }
            } else {
                self.monster.enemy = None;
                self.monster.old_enemy = None;
                let fallback = if self.route()?.is_none() {
                    self.spec.stand
                } else {
                    self.spec.walk
                };
                return self.play(fallback);
            }
        }
        let Some(enemy) = self.monster.enemy.clone() else {
            return Ok(());
        };
        let combat_route = self
            .game
            .monster_missions
            .get(&self.id)
            .map(|mission| mission.combat_route());
        if let Some(goal) = combat_route.as_ref().and_then(|route| route.goal.clone()) {
            let owned = self.owned()?;
            self.game
                .host
                .move_to_goal(&owned, &goal, distance, Some(Q1GoalMode::Contact));
            return Ok(());
        }
        let seen = self.visible(Some(&enemy))?;
        if let Some(world) = self.game.world.clone() {
            self.game.update_entity(&world, |entity| {
                entity.fields.insert(
                    String::from("enemy_visible"),
                    if seen { String::from("1") } else { String::from("0") },
                );
            })?;
        }
        if seen {
            self.monster.search_until = self.game.time + 5.0;
        }
        if self.search_for_coop_target()? {
            return Ok(());
        }
        self.update_run_knowledge()?;
        let attack_state = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.attack_state)
            .unwrap_or(Q1AttackState::Straight);
        if attack_state != Q1AttackState::Straight {
            self.face()?;
            let ideal = self
                .game
                .entity_ref(&self.id)
                .map(|entity| entity.ideal_yaw)
                .unwrap_or(0.0);
            let yaw = self.game.body(&self.id).map(|body| body.angles.y).unwrap_or(0.0);
            let delta = (f64::from(yaw) - ideal + 360.0) % 360.0;
            if delta <= 45.0 || delta >= 315.0 {
                if attack_state == Q1AttackState::Melee {
                    self.melee_attack()?;
                } else if let Some(missile) = self.spec.missile {
                    self.play(missile)?;
                }
                self.game
                    .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Straight)?;
            }
            return Ok(());
        }
        if seen && self.try_attack()? {
            return Ok(());
        }
        if combat_route.is_some_and(|route| route.stand_ground) {
            return Ok(());
        }
        if self.controller.sliding {
            self.face()?;
            let ideal = self
                .game
                .entity_ref(&self.id)
                .map(|entity| entity.ideal_yaw)
                .unwrap_or(0.0);
            let direction = ideal + if self.controller.lefty { 90.0 } else { -90.0 };
            let owned = self.owned()?;
            if !self.game.host.walk_move(&owned, direction, distance) {
                self.controller.lefty = !self.controller.lefty;
                let owned = self.owned()?;
                self.game.host.walk_move(&owned, direction + 180.0, distance);
            }
        } else {
            self.move_to_enemy(distance)?;
        }
        Ok(())
    }

    /// Publish enemy knowledge to the world (`updateRunKnowledge`).
    pub fn update_run_knowledge(&mut self) -> Result<(), Q1Error> {
        let target = self.target()?.unwrap_or(ZERO);
        let origin = self.origin()?;
        let delta = vsub(target, origin);
        let basis = self.make_vectors()?;
        let front = dot(normalize(delta), basis.forward) > 0.3;
        let distance = self.range_distance(None)?;
        let yaw = yaw_for(delta);
        if let Some(world) = self.game.world.clone() {
            self.game.update_entity(&world, |entity| {
                entity.fields.insert(
                    String::from("enemy_infront"),
                    if front { String::from("1") } else { String::from("0") },
                );
                entity.fields.insert(
                    String::from("enemy_range"),
                    if distance < 120.0 {
                        String::from("0")
                    } else if distance < 500.0 {
                        String::from("1")
                    } else if distance < 1000.0 {
                        String::from("2")
                    } else {
                        String::from("3")
                    },
                );
                entity
                    .fields
                    .insert(String::from("enemy_yaw"), f64::from(yaw as f32).to_string());
            })?;
        }
        Ok(())
    }

    /// Step toward the enemy (`moveToEnemy`).
    pub fn move_to_enemy(&mut self, distance: f64) -> Result<(), Q1Error> {
        if let Some(enemy) = self.monster.enemy.clone() {
            let owned = self.owned()?;
            self.game.host.move_to_goal(&owned, &enemy, distance, None);
        }
        Ok(())
    }

    /// Cooperative re-target scan (`searchForCoopTarget`).
    pub fn search_for_coop_target(&mut self) -> Result<bool, Q1Error> {
        if self.game.options().coop && self.monster.search_until < self.game.time {
            return self.find_target();
        }
        Ok(false)
    }

    /// Delay the next missile attack (`attackFinished`).
    pub fn attack_finished(&mut self, seconds: f64) {
        self.monster.refired = false;
        if self.game.options().edition == Q1Edition::Rerelease || self.game.options().skill != 3 {
            self.monster.attack_finished = self.game.time + seconds;
        }
    }

    fn clear_shot(&mut self) -> Result<bool, Q1Error> {
        let Some(enemy) = self.monster.enemy.clone() else {
            return Ok(false);
        };
        let start = self.eye(None)?;
        let end = self.eye(Some(&enemy))?;
        let (Some(start), Some(end)) = (start, end) else {
            return Ok(false);
        };
        let trace = self.game.host.trace(&Q1TraceRequest {
            start,
            end,
            bounds: POINT,
            ignore: Some(self.id.clone()),
            monsters: true,
            missile: false,
        });
        Ok(trace.actor.as_ref().is_some_and(|actor| same_actor(actor, &enemy))
            && (self.spec.species == Q1MonsterSpecies::Wizard || !(trace.in_open && trace.in_water)))
    }

    fn wizard_move(&mut self, sliding: bool) -> Result<(), Q1Error> {
        if self.controller.sliding == sliding {
            return Ok(());
        }
        self.controller.sliding = sliding;
        self.play(if sliding { "wiz_side1" } else { "wiz_run1" })
    }

    /// Attempt an attack (`tryAttack`).
    pub fn try_attack(&mut self) -> Result<bool, Q1Error> {
        let target = self.target()?;
        let enemy = self.monster.enemy.clone();
        let (Some(target), Some(enemy)) = (target, enemy) else {
            return Ok(false);
        };
        let distance = self.range_distance(None)?;
        if self.spec.species == Q1MonsterSpecies::Demon {
            if distance < 120.0 {
                self.game
                    .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Melee)?;
                return Ok(true);
            }
            let body = self.game.body(&self.id)?;
            let Some(other) = self.game.host.bodies.read(&enemy) else {
                return Ok(false);
            };
            let delta = vsub(target, body.origin);
            let horizontal = (f64::from(delta.x).powi(2) + f64::from(delta.y).powi(2)).sqrt();
            let height = f64::from(other.bounds.max.z - other.bounds.min.z);
            if f64::from(body.origin.z + body.bounds.min.z) > f64::from(target.z + other.bounds.min.z) + height * 0.75
                || f64::from(body.origin.z + body.bounds.max.z)
                    < f64::from(target.z + other.bounds.min.z) + height * 0.25
                || horizontal < 100.0
                || horizontal > 200.0 && self.game.host.random() < 0.9
            {
                return Ok(false);
            }
            self.game.sound_simple(&self.id.clone(), "demon/djump.wav")?;
            self.game
                .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Missile)?;
            return Ok(true);
        }
        let classname = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        let specialized = classname == "monster_ogre" || self.spec.species == Q1MonsterSpecies::Shambler;
        if !specialized && self.spec.species != Q1MonsterSpecies::Wizard && !self.clear_shot()? {
            return Ok(false);
        }
        if distance < 120.0 && self.spec.melee && (!specialized || self.game.can_damage(&enemy, &self.id)) {
            if specialized {
                self.game
                    .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Melee)?;
            } else {
                self.melee_attack()?;
            }
            return Ok(true);
        }
        if self.spec.missile.is_none() || self.game.time < self.monster.attack_finished {
            return Ok(false);
        }
        if distance >= 1000.0 || self.spec.species == Q1MonsterSpecies::Shambler && distance > 600.0 {
            if self.spec.species == Q1MonsterSpecies::Wizard {
                self.wizard_move(false)?;
            }
            return Ok(false);
        }
        if (specialized || self.spec.species == Q1MonsterSpecies::Wizard) && !self.clear_shot()? {
            if self.spec.species == Q1MonsterSpecies::Wizard {
                self.wizard_move(false)?;
            }
            return Ok(false);
        }
        if specialized {
            let delay = (if self.spec.species == Q1MonsterSpecies::Shambler {
                2.0
            } else {
                1.0
            }) + 2.0 * self.game.host.random();
            self.attack_finished(delay);
            self.game
                .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Missile)?;
            return Ok(true);
        }
        if distance < 120.0 && self.spec.species != Q1MonsterSpecies::Wizard {
            self.monster.attack_finished = 0.0;
        }
        let chance = if distance < 120.0 {
            0.9
        } else if distance < 500.0 {
            if self.spec.species == Q1MonsterSpecies::Wizard {
                0.6
            } else if self.spec.melee {
                0.2
            } else {
                0.4
            }
        } else if self.spec.species == Q1MonsterSpecies::Wizard {
            0.2
        } else if self.spec.melee {
            0.05
        } else {
            0.1
        };
        if self.game.host.random() >= chance {
            if self.spec.species == Q1MonsterSpecies::Wizard {
                self.wizard_move(distance < 500.0)?;
            }
            return Ok(false);
        }
        if self.spec.species == Q1MonsterSpecies::Wizard {
            self.controller.sliding = false;
            self.game
                .update_entity(&self.id, |entity| entity.attack_state = Q1AttackState::Missile)?;
            return Ok(true);
        }
        if self.spec.species == Q1MonsterSpecies::Zombie {
            let rolled = self.game.host.random();
            self.play(if rolled < 0.3 {
                "zombie_atta1"
            } else if rolled < 0.6 {
                "zombie_attb1"
            } else {
                "zombie_attc1"
            })?;
        } else if let Some(missile) = self.spec.missile {
            self.play(missile)?;
        }
        let delay = 2.0 * self.game.host.random();
        self.attack_finished(delay);
        Ok(true)
    }

    /// Select a melee attack (`meleeAttack`).
    pub fn melee_attack(&mut self) -> Result<(), Q1Error> {
        match self.spec.species {
            Q1MonsterSpecies::Knight => {
                let range = self.range_distance(None)?;
                self.play(if range < 80.0 { "knight_atk1" } else { "knight_runatk1" })
            }
            Q1MonsterSpecies::Demon => self.play("demon1_atta1"),
            Q1MonsterSpecies::Ogre => {
                let roll = self.game.host.random();
                self.play(if roll > 0.5 { "ogre_smash1" } else { "ogre_swing1" })
            }
            Q1MonsterSpecies::Hellknight => {
                self.game
                    .sound(&self.id.clone(), "hknight/slash1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
                let next = super::provider::next_hell_knight_melee(self.game)?;
                self.play(&next)
            }
            Q1MonsterSpecies::Shambler => {
                let chance = self.game.host.random();
                self.play(if chance > 0.6 || self.game.health(&self.id) == 600.0 {
                    "sham_smash1"
                } else if chance > 0.3 {
                    "sham_swingr1"
                } else {
                    "sham_swingl1"
                })
            }
            Q1MonsterSpecies::Tarbaby => self.play("tbaby_jump1"),
            Q1MonsterSpecies::Fish => self.play("f_attack1"),
            _ => Ok(()),
        }
    }

    /// Apply melee damage (`melee`).
    pub fn melee(&mut self, range: f64, scale: f64, rolls: i32, sight: bool) -> Result<f64, Q1Error> {
        let Some(enemy) = self.monster.enemy.clone() else {
            return Ok(0.0);
        };
        if self.distance()? > range || sight && !self.game.can_damage(&enemy, &self.id) {
            return Ok(0.0);
        }
        let mut damage = 0.0;
        for _ in 0..rolls {
            damage = f64::from((damage + self.game.host.random()) as f32);
        }
        damage = f64::from((damage * scale) as f32);
        self.game
            .damage_direct(&enemy, Some(&self.id.clone()), Some(&self.id.clone()), damage);
        Ok(damage)
    }

    /// Retaliate against an attacker (`retaliate`).
    pub fn retaliate(&mut self, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
        let Some(attacker) = attacker.cloned() else {
            return Ok(());
        };
        if same_actor(&attacker, &self.id) {
            return Ok(());
        }
        let classname = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        if self.game.host.classname(&attacker) == classname {
            return Ok(());
        }
        if let Some(world) = self.game.world.clone() {
            if same_actor(&attacker, &world) {
                return Ok(());
            }
        }
        if let Some(enemy) = self.monster.enemy.clone() {
            if same_actor(&attacker, &enemy) {
                return Ok(());
            }
            if self.game.is_player(&enemy) {
                self.monster.old_enemy = Some(enemy);
            }
        }
        self.found(&attacker)
    }

    /// React to pain (`pain`).
    pub fn pain(&mut self, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
        let attacker = attacker.cloned();
        let flags = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.movement_flags)
            .unwrap_or(0);
        if flags & 32 != 0 {
            self.retaliate(attacker.as_ref())?;
        }
        match self.spec.species {
            Q1MonsterSpecies::Zombie => {
                let owned = self.owned()?;
                self.game.host.combat.set_health(&owned, 60.0)?;
                if damage < 9.0 || self.controller.in_pain == 2.0 {
                    return Ok(());
                }
                if damage >= 25.0 {
                    self.controller.in_pain = 2.0;
                    return self.play("zombie_paine1");
                }
                if self.controller.in_pain != 0.0 {
                    self.monster.pain_finished = self.game.time + 3.0;
                    return Ok(());
                }
                if self.monster.pain_finished > self.game.time {
                    self.controller.in_pain = 2.0;
                    return self.play("zombie_paine1");
                }
                self.controller.in_pain = 1.0;
                let rolled = self.game.host.random();
                self.play(if rolled < 0.25 {
                    "zombie_paina1"
                } else if rolled < 0.5 {
                    "zombie_painb1"
                } else if rolled < 0.75 {
                    "zombie_painc1"
                } else {
                    "zombie_paind1"
                })
            }
            Q1MonsterSpecies::Fish => self.play("f_pain1"),
            Q1MonsterSpecies::Wizard => {
                self.game.sound_simple(&self.id.clone(), "wizard/wpain.wav")?;
                if self.game.host.random() * 70.0 > damage {
                    return Ok(());
                }
                self.play("wiz_pain1")
            }
            Q1MonsterSpecies::Shambler => {
                self.game.sound_simple(&self.id.clone(), "shambler/shurt2.wav")?;
                if self.game.health(&self.id) <= 0.0
                    || self.game.host.random() * 400.0 > damage
                    || self.monster.pain_finished > self.game.time
                {
                    return Ok(());
                }
                self.monster.pain_finished = self.game.time + 2.0;
                self.play("sham_pain1")
            }
            Q1MonsterSpecies::Demon => {
                if self
                    .game
                    .entity_ref(&self.id)
                    .and_then(|entity| entity.touch.clone())
                    .is_some()
                    || self.monster.pain_finished > self.game.time
                {
                    return Ok(());
                }
                self.monster.pain_finished = self.game.time + 1.0;
                self.game.sound_simple(&self.id.clone(), "demon/dpain1.wav")?;
                if self.game.host.random() * 200.0 > damage {
                    return Ok(());
                }
                self.play("demon1_pain1")
            }
            Q1MonsterSpecies::Knight => {
                if self.monster.pain_finished > self.game.time {
                    return Ok(());
                }
                let rolled = self.game.host.random();
                self.game.sound_simple(&self.id.clone(), "knight/khurt.wav")?;
                self.monster.pain_finished = self.game.time + 1.0;
                self.play(if rolled < 0.85 { "knight_pain1" } else { "knight_painb1" })
            }
            Q1MonsterSpecies::Enforcer => {
                let rolled = self.game.host.random();
                if self.monster.pain_finished > self.game.time {
                    return Ok(());
                }
                self.game.sound_simple(
                    &self.id.clone(),
                    if rolled < 0.5 {
                        "enforcer/pain1.wav"
                    } else {
                        "enforcer/pain2.wav"
                    },
                )?;
                self.monster.pain_finished = self.game.time + if rolled < 0.7 { 1.0 } else { 2.0 };
                self.play(if rolled < 0.2 {
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
                if self.monster.pain_finished > self.game.time {
                    return Ok(());
                }
                self.game.sound_simple(&self.id.clone(), "ogre/ogpain1.wav")?;
                let rolled = self.game.host.random();
                self.monster.pain_finished = self.game.time + if rolled < 0.75 { 1.0 } else { 2.0 };
                self.play(if rolled < 0.25 {
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
                if self.monster.pain_finished > self.game.time {
                    return Ok(());
                }
                self.game.sound_simple(&self.id.clone(), "hknight/pain1.wav")?;
                if self.game.time - self.monster.pain_finished <= 5.0 && self.game.host.random() * 30.0 > damage {
                    return Ok(());
                }
                self.monster.pain_finished = self.game.time + 1.0;
                self.play("hknight_pain1")
            }
            Q1MonsterSpecies::Shalrath => {
                if self.monster.pain_finished > self.game.time {
                    return Ok(());
                }
                self.game.sound_simple(&self.id.clone(), "shalrath/pain.wav")?;
                self.monster.pain_finished = self.game.time + 3.0;
                self.play("shal_pain1")
            }
            Q1MonsterSpecies::Tarbaby | Q1MonsterSpecies::Boss | Q1MonsterSpecies::Oldone => Ok(()),
            _ => Ok(()),
        }
    }

    /// Count a kill (`countKill`).
    pub fn count_kill(&mut self) -> Result<(), Q1Error> {
        if self.controller.counted_death {
            return Ok(());
        }
        self.controller.counted_death = true;
        let enemy = self.monster.enemy.clone();
        if let Some(mission) = self.game.monster_missions.get_mut(&self.id) {
            mission.killed(enemy.as_ref());
        } else if super::provider::count_monster_kill(self)? {
            self.game.killed_monsters += 1;
            let (total, found) = (self.game.total_monsters, self.game.killed_monsters);
            self.game.host.emit(Q1Event::MonsterKilled {
                actor: self.id.clone(),
                total,
                found,
            });
        }
        if self.game.options().edition == Q1Edition::Rerelease {
            let flags = self
                .game
                .entity_ref(&self.id)
                .map(|entity| entity.movement_flags)
                .unwrap_or(0);
            if flags & 32 != 0 {
                if let Some(enemy) = enemy.clone() {
                    if !same_actor(&enemy, &self.id)
                        && self
                            .game
                            .entity_ref(&enemy)
                            .map(|entity| entity.movement_flags)
                            .unwrap_or(0)
                            & 32
                            != 0
                    {
                        self.game.host.emit(Q1Event::Achievement {
                            player: None,
                            id: String::from("ACH_FRIENDLY_FIRE"),
                        });
                    }
                }
            }
        }
        self.game
            .update_entity(&self.id, |entity| entity.movement_flags &= !3)?;
        if !self.game.monster_missions.contains_key(&self.id) {
            let enemy = self.monster.enemy.clone();
            self.game.use_targets(&self.id.clone(), enemy.as_ref())?;
        }
        Ok(())
    }

    /// Die (`die`).
    pub fn die(&mut self, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
        if self.controller.counted_death {
            return Ok(());
        }
        self.monster.enemy = attacker.cloned();
        if self.game.health(&self.id) < -99.0 {
            let owned = self.owned()?;
            self.game.host.combat.set_health(&owned, -99.0)?;
        }
        self.game.set_damageable(&self.id.clone(), false)?;
        self.game.update_entity(&self.id, |entity| entity.touch = None)?;
        if self.spec.species == Q1MonsterSpecies::Oldone {
            return super::provider::base_finale(self);
        }
        self.count_kill()?;
        let health = self.game.health(&self.id);
        if health < self.spec.gib_health {
            if let Some(head) = self.spec.head {
                self.game.sound_simple(
                    &self.id.clone(),
                    if self.spec.species == Q1MonsterSpecies::Zombie {
                        "zombie/z_gib.wav"
                    } else {
                        "player/udeath.wav"
                    },
                )?;
                let origin = self.origin()?;
                for gib in self.spec.gibs {
                    throw_gib(self.game, origin, gib, health)?;
                }
                return throw_head(self.game, &self.id.clone(), head, health);
            }
        }
        match self.spec.species {
            Q1MonsterSpecies::Knight => {
                self.game.sound_simple(&self.id.clone(), "knight/kdeath.wav")?;
                {
                    let roll = self.game.host.random();
                    self.play(if roll < 0.5 { "knight_die1" } else { "knight_dieb1" })
                }
            }
            Q1MonsterSpecies::Enforcer => {
                self.game.sound_simple(&self.id.clone(), "enforcer/death1.wav")?;
                {
                    let roll = self.game.host.random();
                    self.play(if roll > 0.5 { "enf_die1" } else { "enf_fdie1" })
                }
            }
            Q1MonsterSpecies::Demon => self.play("demon1_die1"),
            Q1MonsterSpecies::Ogre => {
                self.game.sound_simple(&self.id.clone(), "ogre/ogdth.wav")?;
                {
                    let roll = self.game.host.random();
                    self.play(if roll < 0.5 { "ogre_die1" } else { "ogre_bdie1" })
                }
            }
            Q1MonsterSpecies::Hellknight => {
                self.game.sound_simple(&self.id.clone(), "hknight/death.wav")?;
                {
                    let roll = self.game.host.random();
                    self.play(if roll > 0.5 { "hknight_die1" } else { "hknight_dieb1" })
                }
            }
            Q1MonsterSpecies::Shambler => {
                self.game.sound_simple(&self.id.clone(), "shambler/sdeath.wav")?;
                self.play("sham_death1")
            }
            Q1MonsterSpecies::Wizard => {
                self.game.update_entity(&self.id, |entity| {
                    entity.movement = Q1MoveType::Toss;
                    entity.movement_flags &= !1;
                })?;
                self.play("wiz_death1")
            }
            Q1MonsterSpecies::Shalrath => {
                self.game.sound_simple(&self.id.clone(), "shalrath/death.wav")?;
                self.game
                    .update_entity(&self.id, |entity| entity.solid = Q1Solid::None)?;
                self.game.link(&self.id.clone())?;
                self.play("shal_death1")
            }
            Q1MonsterSpecies::Tarbaby => self.play("tbaby_die1"),
            Q1MonsterSpecies::Fish => self.play("f_death1"),
            Q1MonsterSpecies::Zombie => {
                self.game.sound_simple(&self.id.clone(), "zombie/z_gib.wav")?;
                let origin = self.origin()?;
                let health = self.game.health(&self.id);
                for gib in self.spec.gibs {
                    throw_gib(self.game, origin, gib, health)?;
                }
                throw_head(self.game, &self.id.clone(), "h_zombie", health)
            }
            Q1MonsterSpecies::Boss => self.play("boss_death1"),
            _ => Ok(()),
        }
    }

    /// Spawn a monster (`spawn`).
    pub fn spawn(&mut self) -> Result<(), Q1Error> {
        let source_none = self.prefix == "base";
        if source_none && self.game.uses_id1_precaches() {
            if self.game.options().deathmatch != 0 {
                let id = self.id.clone();
                return self.game.remove(&id);
            }
            precache_id1_monster(self.game, self.spec.species)?;
        }
        let spawnflags = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.spawnflags)
            .unwrap_or(0);
        let crucified = self.spec.species == Q1MonsterSpecies::Zombie && spawnflags & 1 != 0;
        if !crucified {
            if self.game.monster_missions.contains_key(&self.id) {
                if let Some(mission) = self.game.monster_missions.get_mut(&self.id) {
                    mission.spawned();
                }
            } else {
                self.game.total_monsters += 1;
            }
        }
        self.game
            .update_entity(&self.id, |entity| entity.max_health = self.spec.health)?;
        let owned = self.owned()?;
        self.game.host.combat.set_health(&owned, self.spec.health)?;
        let model = format!("progs/{}.mdl", self.spec.model);
        self.game.update_entity(&self.id, |entity| {
            entity.model = model.clone();
            entity.solid = Q1Solid::Slidebox;
            entity.movement = Q1MoveType::Step;
            entity.aimed_damage = true;
        })?;
        if let Some(kill_string) = self.spec.kill_string {
            self.game.update_entity(&self.id, |entity| {
                entity
                    .fields
                    .insert(String::from("killstring"), kill_string.to_string());
            })?;
        }
        let yaw_speed = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.number("yaw_speed"))
            .unwrap_or(0.0);
        let default_yaw = if self.spec.movement == MonsterMovement::Fly || self.spec.movement == MonsterMovement::Swim {
            10.0
        } else {
            20.0
        };
        self.game.update_entity(&self.id, |entity| {
            entity.yaw_speed = if yaw_speed == 0.0 { default_yaw } else { yaw_speed };
        })?;
        let yaw = self.game.body(&self.id).map(|body| body.angles.y)?;
        self.game
            .update_entity(&self.id, |entity| entity.ideal_yaw = f64::from(yaw))?;
        if !crucified && self.spec.movement != MonsterMovement::Boss {
            let bits = if self.spec.movement == MonsterMovement::Fly {
                1
            } else if self.spec.movement == MonsterMovement::Swim {
                2
            } else {
                0
            };
            self.game
                .update_entity(&self.id, |entity| entity.movement_flags |= bits)?;
        }
        self.game.set_bounds(&self.id.clone(), self.spec.bounds)?;
        let prefix = self.prefix.clone();
        let pain = self.game.named.pain(&format!("{prefix}:monster_pain"))?;
        let die = self.game.named.die(&format!("{prefix}:monster_die"))?;
        let path_end = self.game.named.action(&format!("{prefix}:monster_stand"))?;
        let use_callback = self.game.named.use_callback(&format!("{prefix}:monster_use"))?;
        self.game.update_entity(&self.id, |entity| {
            entity.pain = Some(pain);
            entity.die = Some(die);
            entity.path_end = Some(path_end);
            entity.use_callback = Some(use_callback);
        })?;
        if self.spec.species == Q1MonsterSpecies::Boss {
            let awake = self.game.named.use_callback(&format!("{prefix}:boss_awake"))?;
            self.game.update_entity(&self.id, |entity| {
                entity.model = String::new();
                entity.solid = Q1Solid::None;
                entity.use_callback = Some(awake);
            })?;
            return Ok(());
        }
        if self.spec.species == Q1MonsterSpecies::Oldone {
            self.game.set_damageable(&self.id.clone(), true)?;
            self.controller.next_frame = String::from("old_idle1");
            return self.delay(0.1);
        }
        if crucified {
            self.game
                .update_entity(&self.id, |entity| entity.movement = Q1MoveType::None)?;
            return self.play("zombie_cruc1");
        }
        let next_think = self
            .game
            .entity_ref(&self.id)
            .map(|entity| entity.next_think)
            .unwrap_or(0.0);
        let delay = next_think.max(0.0) + self.game.host.random() * 0.5 - self.game.time;
        self.game
            .schedule(&self.id.clone(), delay, &format!("{prefix}:monster_start"))
    }

    /// Start a monster after map spawn (`start`).
    pub fn start(&mut self) -> Result<(), Q1Error> {
        if self.spec.movement == MonsterMovement::Walk {
            let origin = self.origin()?;
            let start = vadd(origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 });
            let bounds = self.game.body(&self.id).map(|body| body.bounds)?;
            let trace = self.game.host.trace(&Q1TraceRequest {
                start,
                end: vadd(
                    start,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: -256.0,
                    },
                ),
                bounds,
                ignore: Some(self.id.clone()),
                monsters: true,
                missile: false,
            });
            if trace.fraction < 1.0 && !trace.all_solid {
                let actor = trace.actor.clone();
                self.game.set_body(
                    &self.id.clone(),
                    &crate::q1::foundation::gameplay::BodyPatch {
                        origin: Some(trace.end),
                        ground: Some(actor),
                        ..Default::default()
                    },
                )?;
                self.game
                    .update_entity(&self.id, |entity| entity.movement_flags |= 512)?;
            } else {
                self.game.set_body(
                    &self.id.clone(),
                    &crate::q1::foundation::gameplay::BodyPatch {
                        origin: Some(start),
                        ..Default::default()
                    },
                )?;
            }
            let owned = self.owned()?;
            self.game.host.walk_move(&owned, 0.0, 0.0);
        }
        if self.spec.species == Q1MonsterSpecies::Fish
            && self.game.options().edition == Q1Edition::Classic
            && !self.game.monster_missions.contains_key(&self.id)
        {
            self.game.total_monsters += 1;
        }
        self.game
            .update_entity(&self.id, |entity| entity.movement_flags |= 32)?;
        self.game.set_damageable(&self.id.clone(), true)?;
        let yaw = self.game.body(&self.id).map(|body| body.angles.y)?;
        self.game
            .update_entity(&self.id, |entity| entity.ideal_yaw = f64::from(yaw))?;
        self.game.link(&self.id.clone())?;
        if self.spec.movement == MonsterMovement::Fly {
            let owned = self.owned()?;
            self.game.host.walk_move(&owned, 0.0, 0.0);
        }
        if self.game.monster_missions.contains_key(&self.id) {
            if let Some(mission) = self.game.monster_missions.get_mut(&self.id) {
                mission.started();
            }
            self.update_route()?;
        } else {
            let path = self.monster.path.clone();
            if !path.is_empty() {
                let goal = self.game.find(&path).first().cloned();
                if self.spec.movement != MonsterMovement::Fly {
                    let end = match goal {
                        Some(ref goal) => self.game.body(goal).map(|body| body.origin).unwrap_or(ZERO),
                        None => ZERO,
                    };
                    let origin = self.origin()?;
                    let yaw = yaw_for(vsub(end, origin));
                    self.game.update_entity(&self.id, |entity| entity.ideal_yaw = yaw)?;
                }
                if self.spec.movement == MonsterMovement::Swim {
                    let walk = self.spec.walk;
                    self.play(walk)?;
                } else {
                    let corner = goal
                        .as_ref()
                        .and_then(|goal| self.game.entity_ref(goal))
                        .is_some_and(|entity| entity.classname == "path_corner");
                    if corner {
                        let walk = self.spec.walk;
                        self.play(walk)?;
                    } else {
                        self.monster.pause_until = 100000000.0;
                        let stand = self.spec.stand;
                        self.play(stand)?;
                    }
                }
            } else {
                self.monster.pause_until = 100000000.0;
                let stand = self.spec.stand;
                self.play(stand)?;
            }
        }
        let think = self.game.entity_ref(&self.id).and_then(|entity| entity.think.clone());
        if self.spec.movement != MonsterMovement::Fly {
            if let Some(think) = think {
                let next_think = self
                    .game
                    .entity_ref(&self.id)
                    .map(|entity| entity.next_think)
                    .unwrap_or(0.0);
                let delay = next_think - self.game.time + self.game.host.random() * 0.5;
                return self.game.schedule(&self.id.clone(), delay, &think);
            }
        }
        Ok(())
    }

    /// Use a monster (`use`).
    pub fn use_monster(&mut self, activator: Option<&ActorId>) -> Result<(), Q1Error> {
        let activator = activator.cloned();
        if let Some(mission) = self.game.monster_missions.get_mut(&self.id) {
            if mission.r#use(activator.as_ref()) {
                return Ok(());
            }
        }
        if self.monster.enemy.is_some() || self.game.health(&self.id) <= 0.0 {
            return Ok(());
        }
        let Some(activator) = activator else { return Ok(()) };
        if !self.game.is_player(&activator) {
            return Ok(());
        }
        let observed = self.game.monster_target(&activator);
        if observed.is_none_or(|seen| seen.invisible || seen.notarget) {
            return Ok(());
        }
        self.monster.enemy = Some(activator);
        self.game
            .schedule(&self.id.clone(), 0.1, &format!("{}:monster_found", self.prefix))
    }

    /// Wake Chthon (`awake`).
    pub fn awake(&mut self, activator: Option<&ActorId>) -> Result<(), Q1Error> {
        self.game.update_entity(&self.id, |entity| {
            entity.model = String::from("progs/boss.mdl");
            entity.solid = Q1Solid::Slidebox;
        })?;
        self.game.set_damageable(&self.id.clone(), false)?;
        let health = if self.game.options().skill == 0 { 1.0 } else { 3.0 };
        let owned = self.owned()?;
        self.game.host.combat.set_health(&owned, health)?;
        self.monster.enemy = activator.cloned();
        let origin = self.origin()?;
        self.game.effect(Q1Effect::LavaSplash, origin, None, 1);
        self.game.link(&self.id.clone())?;
        self.play("boss_rise1")
    }
}

/// Spawn a base monster (`registerSpecies` handler).
pub fn spawn_base_monster(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let classname = game
        .entity_ref(&id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let spec = species_by_classname(&classname).ok_or_else(|| q1_error("unknown base monster"))?;
    let mut monster = BaseMonster::spawn_new(game, &id, spec)?;
    monster.spawn()?;
    monster.finish()
}

/// Register monster callbacks for a prefix (`registerMonsterCallbacks`).
pub fn register_monster_callbacks(game: &mut Q1EntityServices, prefix: &str) -> Result<(), Q1Error> {
    if prefix == "base" {
        register_monster_source(MonsterSourceRegistration {
            source: MonsterSource {
                prefix: "base",
                frames: None,
                actions: None,
            },
            load: base_load_controller,
            store: super::creatures::store_monster_controller,
        });
    } else if monster_source(prefix).is_err() {
        return Err(q1_error(format!("Unknown Q1 monster source: {prefix}")));
    }
    use crate::q1::foundation::callbacks::{
        Q1CallbackHandlers, Q1DieHandler, Q1PainHandler, Q1TouchHandler, Q1UseHandler,
    };
    let action = |handler: crate::q1::foundation::callbacks::Q1ActionHandler| Q1CallbackHandlers {
        action: Some(handler),
        ..Default::default()
    };
    game.named.register(
        &format!("{prefix}:monster_jump_touch"),
        Q1CallbackHandlers {
            touch: Some(monster_jump_touch_handler as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named
        .register(&format!("{prefix}:monster_frame"), action(monster_frame_handler))?;
    game.named
        .register(&format!("{prefix}:monster_route"), action(monster_route_handler))?;
    game.named
        .register(&format!("{prefix}:monster_start"), action(monster_start_handler))?;
    game.named
        .register(&format!("{prefix}:monster_stand"), action(monster_stand_handler))?;
    game.named
        .register(&format!("{prefix}:monster_found"), action(monster_found_handler))?;
    game.named.register(
        &format!("{prefix}:monster_pain"),
        Q1CallbackHandlers {
            pain: Some(monster_pain_handler as Q1PainHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:monster_die"),
        Q1CallbackHandlers {
            die: Some(monster_die_handler as Q1DieHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:monster_use"),
        Q1CallbackHandlers {
            use_callback: Some(monster_use_handler as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}:boss_awake"),
        Q1CallbackHandlers {
            use_callback: Some(boss_awake_handler as Q1UseHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn monster_jump_touch_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let other = other.clone();
    let mut monster = BaseMonster::load(game, id)?;
    monster_jump_touch(&mut monster, &other)?;
    monster.finish()
}

fn monster_frame_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    let next = monster.controller.next_frame.clone();
    monster.play(&next)?;
    monster.finish()
}

fn monster_route_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    monster.update_route()?;
    monster.finish()
}

fn monster_start_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    monster.start()?;
    monster.finish()
}

fn monster_stand_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    let stand = monster.spec.stand;
    monster.play(stand)?;
    monster.finish()
}

fn monster_found_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    if let Some(enemy) = monster.monster.enemy.clone() {
        monster.found(&enemy)?;
    }
    monster.finish()
}

fn monster_pain_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
    damage: f64,
) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    monster.pain(attacker, damage)?;
    monster.finish()
}

fn monster_die_handler(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    monster.die(attacker)?;
    monster.finish()
}

fn monster_use_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    monster.use_monster(activator)?;
    monster.finish()
}

fn boss_awake_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let mut monster = BaseMonster::load(game, id)?;
    monster.awake(activator)?;
    monster.finish()
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    #[test]
    fn spawn_start_and_frame_flow() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let knight = game.create("monster_knight", None, None).expect("knight");
        game.spawn_entity(&knight, None).expect("spawn");
        let entity = game.entity_ref(&knight).cloned().expect("entity");
        assert_eq!(entity.model, "progs/knight.mdl");
        assert_eq!(entity.max_health, 75.0);
        assert_eq!(entity.think.as_deref(), Some("base:monster_start"));
        game.invoke_action(&knight, "base:monster_start").expect("start");
        let controller = super::super::creatures::monster_controller(
            &game,
            &knight,
            &game
                .entity_ref(&knight)
                .map(|entity| entity.classname.clone())
                .unwrap_or_default(),
        )
        .expect("controller");
        assert_eq!(controller.current_frame, "knight_stand1");
        assert_eq!(game.entity_ref(&knight).map(|entity| entity.frame), Some(0));
        game.invoke_action(&knight, "base:monster_frame").expect("frame");
        let advanced =
            super::super::creatures::monster_controller(&game, &knight, "monster_knight").expect("controller");
        assert_eq!(advanced.current_frame, "knight_stand2");
    }

    #[test]
    fn death_gibs_below_threshold() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let knight = game.create("monster_knight", None, None).expect("knight");
        game.spawn_entity(&knight, None).expect("spawn");
        let owned = game
            .entity_ref(&knight)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host.combat.set_health(&owned, -50.0).expect("health");
        game.invoke_die(&knight, None).expect("die");
        assert_eq!(
            game.entity_ref(&knight).map(|entity| entity.model.clone()),
            Some(String::from("progs/h_knight.mdl"))
        );
        assert_eq!(game.killed_monsters, 1);
    }

    #[test]
    fn controller_state_round_trips() {
        let state = BaseMonsterState::new(Q1MonsterSpecies::Ogre, String::from("base"), "ogre_stand1");
        let bytes = crate::q1::foundation::checkpoint::encode_checkpoint_value(&state.capture());
        let value = crate::q1::foundation::checkpoint::decode_checkpoint_value(&bytes).expect("decode");
        let restored = BaseMonsterState::restore(
            Q1MonsterSpecies::Ogre,
            String::from("base"),
            SaveReader::at(&value, "test"),
        )
        .expect("restore");
        assert_eq!(restored, state);
    }
}
