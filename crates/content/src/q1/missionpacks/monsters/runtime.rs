//! Mission-pack monster controllers
//! (`src/content/q1/missionpacks/monsters/runtime.ts`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, OnceLock};

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;

use crate::monsters::monster_target_eligible;
use crate::q1::base::animation::{MonsterAi, MonsterOperation, SoundComparison};
use crate::q1::base::monster_actions::monster_jump_touch;
use crate::q1::base::monsters::{
    register_monster_source, BaseMonster, BaseMonsterState, MonsterSource, MonsterSourceRegistration,
};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies, BASE_SPECIES};
use crate::q1::foundation::callbacks::{callback_name, Q1CallbackHandlers, Q1StateExtension};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity::{
    Q1Actor, Q1AttackState, Q1Monster, Q1MonsterMode, Q1MonsterSpecies,
};
use crate::q1::foundation::entity_services::{Q1EntityServices, Q1SpawnHandler};
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::host::Q1ReleaseHook;
use crate::q1::foundation::types::{
    length, vsub, yaw_for, Q1Basis, Q1Event, ZERO,
};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, int, obj, str as save_str, SaveJson, SaveReader};

use super::charm::{
    charmer, find_charmed_target, find_hipnotic_target, hunt_charmer, walk_with_charmer,
};
use super::types::{MissionMonsterHooks, PackMonsterDefinition};

/// Pack id text without assuming `Copy` on the pack enum.
fn pack_id(pack: &Q1MissionPack) -> &'static str {
    match pack {
        Q1MissionPack::Hipnotic => "hipnotic",
        Q1MissionPack::Rogue => "rogue",
    }
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

fn copy_handlers(handlers: &Q1CallbackHandlers) -> Q1CallbackHandlers {
    Q1CallbackHandlers {
        trajectory: handlers.trajectory,
        action: handlers.action,
        use_callback: handlers.use_callback,
        touch: handlers.touch,
        pain: handlers.pain,
        die: handlers.die,
        blocked: handlers.blocked,
    }
}

/// Mission-pack monster controller view (`MissionMonster`).
///
/// The view borrows the game and the pack runtime mutably and stages the
/// entity, shared monster state, and controller scalars as plain fields so
/// callbacks read donor-shaped `monster.entity` / `monster.state` /
/// `monster.enemy` values. Every method flushes staged fields to the game
/// on entry (`sync`) and reloads snapshots on exit (`refresh`); free
/// functions in the sibling modules do the same. Direct field writes
/// therefore persist when the next monster method runs.
pub struct MissionMonster<'m> {
    /// Entity services.
    pub game: &'m mut Q1EntityServices,
    /// Staged entity snapshot.
    pub entity: Q1Actor,
    /// Monster definition.
    pub definition: &'static PackMonsterDefinition,
    /// Pack runtime.
    pub runtime: &'m mut Q1MissionPackMonsters,
    /// Spawn defaults.
    pub spec: &'static MonsterSpecies,
    /// Shared monster state.
    pub state: Q1Monster,
    /// Body origin snapshot.
    pub origin: Vec3,
    /// Current enemy.
    pub enemy: Option<ActorId>,
    /// Enemy position snapshot.
    pub target: Option<Vec3>,
    /// Strafe direction.
    pub lefty: bool,
    /// Next frame name.
    pub next_frame: String,
    /// Whether death was counted.
    pub counted_death: bool,
    /// Controller extras (current frame, pain/counter/idle state).
    pub controller: BaseMonsterState,
}

impl<'m> MissionMonster<'m> {
    /// Monster actor id.
    #[must_use]
    pub fn id(&self) -> &ActorId {
        &self.entity.actor.id
    }

    /// Callback prefix (`source.callbackPrefix`).
    #[must_use]
    pub fn prefix(&self) -> String {
        pack_id(&self.runtime.pack).to_string()
    }

    fn is_live(&self) -> bool {
        self.game.entity_ref(&self.entity.actor.id).is_some()
    }

    /// Flush staged fields, then reload snapshots. Every method calls this
    /// on entry and `refresh` on exit, so direct field writes persist at
    /// the next monster-method boundary in both directions.
    pub fn sync(&mut self) {
        self.refresh();
    }

    /// Flush staged fields, then reload entity, state, and positional
    /// snapshots.
    pub fn refresh(&mut self) {
        let id = self.entity.actor.id.clone();
        if self.game.entity_ref(&id).is_none() {
            return;
        }
        self.state.enemy = self.enemy.clone();
        self.entity.monster = Some(self.state.clone());
        self.flush_entity();
        self.controller.lefty = self.lefty;
        self.controller.next_frame = self.next_frame.clone();
        self.controller.counted_death = self.counted_death;
        let owned = self.entity.actor.clone();
        self.runtime.controllers.insert(owned, self.controller.clone());
        let Some(entity) = self.game.entity_ref(&id).cloned() else {
            return;
        };
        let species = self.spec.species;
        let path = entity.target.clone();
        self.state = entity
            .monster
            .clone()
            .unwrap_or_else(|| default_monster_state(species, &path));
        self.entity = entity;
        self.enemy = self.state.enemy.clone();
        self.origin = self.game.body(&id).map(|body| body.origin).unwrap_or(ZERO);
        self.target = self
            .enemy
            .as_ref()
            .and_then(|enemy| self.game.host.bodies.read(enemy))
            .map(|body| body.origin);
    }

    /// Write the staged entity snapshot back without a full sync.
    pub fn flush_entity(&mut self) {
        let id = self.entity.actor.id.clone();
        if self.game.entity_ref(&id).is_none() {
            return;
        }
        let staged = self.entity.clone();
        let _ = self.game.update_entity(&id, |entity| *entity = staged);
    }

    /// Persist staged state at the end of a callback.
    pub fn finish(&mut self) {
        self.sync();
    }

    /// Run base-game logic through a transient base view, writing the
    /// controller and monster state back afterwards.
    pub fn with_base<R>(&mut self, run: impl FnOnce(&mut BaseMonster) -> R) -> R {
        self.sync();
        let id = self.entity.actor.id.clone();
        let mut base = BaseMonster {
            game: &mut *self.game,
            id,
            spec: self.spec,
            prefix: self.prefix(),
            controller: self.controller.clone(),
            monster: self.state.clone(),
        };
        let result = run(&mut base);
        let BaseMonster { controller, monster, .. } = base;
        self.lefty = controller.lefty;
        self.next_frame = controller.next_frame.clone();
        self.counted_death = controller.counted_death;
        self.controller = controller.clone();
        self.state = monster.clone();
        self.enemy = monster.enemy.clone();
        let owned = self.entity.actor.clone();
        self.runtime.controllers.insert(owned, controller);
        self.refresh();
        result
    }

    /// Spawn (`spawn`).
    pub fn spawn(&mut self) {
        self.sync();
        match self.definition.spawn.clone() {
            Some(spawn) => spawn(self),
            None => self.spawn_default(),
        }
        self.refresh();
    }

    /// Base spawn pipeline (`spawnDefault`).
    pub fn spawn_default(&mut self) {
        self.with_base(|base| {
            let _ = base.spawn();
        });
    }

    /// Start (`start`).
    pub fn start(&mut self) {
        self.sync();
        match self.definition.start.clone() {
            Some(start) => start(self),
            None => self.start_default(),
        }
        self.refresh();
    }

    /// Base start with Hipnotic path-goal wiring (`startDefault`).
    pub fn start_default(&mut self) {
        self.sync();
        let path = self.game.find(&self.state.path).first().cloned();
        self.entity.references.insert("goalentity".to_string(), path.clone());
        self.entity.references.insert("movetarget".to_string(), path);
        self.flush_entity();
        self.with_base(|base| {
            let _ = base.start();
        });
        self.refresh();
    }

    /// Play a frame or run a named action (`play`).
    pub fn play(&mut self, name: &str) {
        self.sync();
        if !self.is_live() {
            return;
        }
        let id = self.entity.actor.id.clone();
        if self.game.health(&id) > 0.0 {
            if let Some(enemy) = self.enemy.clone() {
                let eligible = monster_target_eligible(
                    self.game.health(&enemy),
                    self.game.monster_target(&enemy).as_ref(),
                );
                if !eligible {
                    let previous = self.state.old_enemy.clone();
                    self.enemy = match previous {
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
                    self.state.old_enemy = None;
                    let _ = self
                        .game
                        .update_entity(&id, |entity| entity.attack_state = Q1AttackState::Straight);
                    let routed = self
                        .entity
                        .references
                        .get("goalentity")
                        .and_then(|goal| goal.clone())
                        .is_some()
                        || !self.state.path.is_empty();
                    let fallback = if self.enemy.is_none() {
                        if routed { self.spec.walk } else { self.spec.stand }
                    } else {
                        self.spec.run
                    };
                    self.refresh();
                    self.play(fallback);
                    return;
                }
            }
        }
        let frame = self
            .definition
            .frames
            .iter()
            .find(|(frame_name, _)| *frame_name == name)
            .map(|(_, frame)| *frame);
        let Some(frame) = frame else {
            if let Some(action) = self
                .definition
                .actions
                .iter()
                .find(|(action_name, _)| *action_name == name)
                .map(|(_, action)| action.clone())
            {
                action(self);
            }
            self.refresh();
            return;
        };
        self.controller.current_frame = name.to_string();
        self.next_frame = frame.next.to_string();
        self.entity.frame = frame.frame;
        self.flush_entity();
        let think = format!("{}:monster_frame", self.prefix());
        let _ = self.game.schedule(&id, 0.1, &think);
        for op in frame.operations {
            if !self.is_live() {
                self.refresh();
                return;
            }
            match op {
                MonsterOperation::Ai { mode, distance } => self.ai(*mode, *distance),
                MonsterOperation::Solid { solid } => {
                    let solid = *solid;
                    let _ = self.game.update_entity(&id, |entity| entity.solid = solid);
                    let _ = self.game.link(&id);
                    self.refresh();
                }
                MonsterOperation::Lightstyle { pattern } => {
                    self.game.host.emit(Q1Event::Lightstyle {
                        style: 0,
                        pattern: (*pattern).to_string(),
                    });
                }
                MonsterOperation::Action { name } => {
                    if let Some(action) = self
                        .definition
                        .actions
                        .iter()
                        .find(|(action_name, _)| *action_name == *name)
                        .map(|(_, action)| action.clone())
                    {
                        action(self);
                    }
                }
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
                            SoundComparison::Greater => draw > *chance,
                            SoundComparison::Less => draw < *chance,
                        },
                    };
                    if play {
                        let _ = self.game.sound(&id, path, *channel, *attenuation, 1.0);
                    }
                }
            }
        }
        self.refresh();
    }

    /// Pain (`pain`).
    pub fn pain(&mut self, attacker: Option<&ActorId>, damage: f64) {
        self.sync();
        if self.definition.base_behavior {
            let attacker = attacker.cloned();
            self.pain_default(attacker.as_ref(), damage);
        } else {
            let attacker = attacker.cloned();
            self.retaliate(attacker.as_ref());
            let pain = self.definition.pain.clone();
            pain(self, attacker.as_ref(), damage);
        }
        self.refresh();
    }

    /// Base pain (`painDefault`).
    pub fn pain_default(&mut self, attacker: Option<&ActorId>, damage: f64) {
        let attacker = attacker.cloned();
        self.with_base(|base| {
            let _ = base.pain(attacker.as_ref(), damage);
        });
    }

    /// Base death (`dieDefault`).
    pub fn die_default(&mut self, attacker: Option<&ActorId>) {
        let attacker = attacker.cloned();
        self.with_base(|base| {
            let _ = base.die(attacker.as_ref());
        });
    }

    /// Death (`die`).
    pub fn die(&mut self, attacker: Option<&ActorId>) {
        self.sync();
        if self.entity.number("charmed") != 0.0 {
            self.entity.effects &= !8;
        }
        if self.definition.base_behavior {
            let attacker = attacker.cloned();
            self.flush_entity();
            self.die_default(attacker.as_ref());
            self.refresh();
            return;
        }
        if self.counted_death {
            self.refresh();
            return;
        }
        let attacker = attacker.cloned();
        self.enemy = attacker.clone();
        let id = self.entity.actor.id.clone();
        if self.game.health(&id) < -99.0 {
            let owned = self.entity.actor.clone();
            let _ = self.game.host.combat.set_health(&owned, -99.0);
        }
        self.entity.touch = None;
        self.flush_entity();
        let _ = self.game.set_damageable(&id, false);
        self.count_kill();
        let die = self.definition.die.clone();
        die(self, attacker.as_ref());
        self.refresh();
    }

    /// Melee attack (`meleeAttack`).
    pub fn melee_attack(&mut self) {
        self.sync();
        if self.definition.base_behavior {
            self.with_base(|base| {
                let _ = base.melee_attack();
            });
        } else if let Some(melee) = self.definition.melee.clone() {
            melee(self);
        }
        self.refresh();
    }

    /// Attack check (`tryAttack`).
    pub fn try_attack(&mut self) -> bool {
        self.sync();
        let hit = match self.definition.check_attack.clone() {
            Some(check_attack) => check_attack(self),
            None => self.check_attack_default(),
        };
        self.refresh();
        hit
    }

    /// Base attack check (`checkAttackDefault`).
    pub fn check_attack_default(&mut self) -> bool {
        self.with_base(|base| base.try_attack().unwrap_or(false))
    }

    /// Target found (`found`). Charmed monsters ignore their owner and its
    /// other charmed monsters.
    pub fn found(&mut self, target: &ActorId) {
        self.sync();
        if let Some(owner) = charmer(self) {
            let owned = self
                .game
                .entity(target)
                .and_then(|entity| entity.references.get("charmer").cloned())
                .flatten();
            if target == &owner || owned == Some(owner) {
                self.enemy = None;
                self.refresh();
                return;
            }
        }
        let target = target.clone();
        match self.definition.found.clone() {
            Some(found) => found(self, &target),
            None => self.found_default(&target),
        }
        self.refresh();
    }

    /// Base target-found with Hipnotic sight handling (`foundDefault`).
    pub fn found_default(&mut self, target: &ActorId) {
        self.sync();
        if pack_id(&self.runtime.pack) != "hipnotic" {
            let target = target.clone();
            self.with_base(|base| {
                let _ = base.found(&target);
            });
            self.refresh();
            return;
        }
        let target = target.clone();
        self.enemy = Some(target.clone());
        if self.game.is_player(&target) {
            self.game.sight_entity = Some(self.entity.actor.id.clone());
            self.game.sight_time = self.game.time;
        }
        let hostile = self.game.time + 1.0;
        self.entity
            .fields
            .insert("show_hostile".to_string(), (hostile as f32).to_string());
        let mut sound = self.spec.sight.to_string();
        if matches!(self.spec.species, Q1MonsterSpecies::Enforcer) {
            let roll = (self.game.host.random() * 3.0 + 0.5).floor() as i32;
            let track = if roll == 1 {
                1
            } else if roll == 2 {
                2
            } else if roll == 0 {
                3
            } else {
                4
            };
            sound = format!("enforcer/sight{track}.wav");
        } else if matches!(self.spec.species, Q1MonsterSpecies::Fish)
            || (matches!(self.spec.species, Q1MonsterSpecies::Gremlin)
                && self.entity.number("stoleweapon") != 0.0)
        {
            sound = String::new();
        }
        if !sound.is_empty() {
            let id = self.entity.actor.id.clone();
            let _ = self.game.sound_simple(&id, &sound);
        }
        self.state.mode = Q1MonsterMode::Run;
        self.next_frame = self.spec.run.to_string();
        self.entity
            .references
            .insert("goalentity".to_string(), Some(target.clone()));
        let enemy_origin = self
            .game
            .host
            .bodies
            .read(&target)
            .map(|body| body.origin)
            .unwrap_or(ZERO);
        self.entity.ideal_yaw = yaw_for(vsub(enemy_origin, self.origin));
        self.flush_entity();
        self.delay(0.1);
        self.attack_finished(1.0);
        self.refresh();
    }

    /// Steering (`ai`).
    pub fn ai(&mut self, mode: MonsterAi, distance: f64) {
        self.sync();
        if matches!(mode, MonsterAi::Walk) && walk_with_charmer(self, distance) {
            self.refresh();
            return;
        }
        match self.definition.ai.clone() {
            Some(ai) => ai(self, mode, distance),
            None => self.ai_default(mode, distance),
        }
        self.refresh();
    }

    /// Base steering (`aiDefault`).
    pub fn ai_default(&mut self, mode: MonsterAi, distance: f64) {
        self.with_base(|base| {
            let _ = base.ai(mode, distance);
        });
    }

    /// Combat run with charmer following and dodging strafe (`run`).
    pub fn run(&mut self, distance: f64) {
        self.sync();
        if let Some(owner) = charmer(self) {
            let lost = match self.enemy.as_ref() {
                None => true,
                Some(enemy) if enemy == &owner => true,
                Some(enemy) => self.game.health(enemy) <= 0.0,
            };
            if lost {
                self.enemy = None;
                hunt_charmer(self, false);
                self.refresh();
                return;
            }
        }
        if !matches!(self.entity.attack_state, Q1AttackState::Dodging) {
            self.with_base(|base| {
                let _ = base.run(distance);
            });
            self.refresh();
            return;
        }
        let lost = match self.enemy.as_ref() {
            None => true,
            Some(enemy) => self.game.health(enemy) <= 0.0,
        };
        if lost {
            self.entity.attack_state = Q1AttackState::Straight;
            self.flush_entity();
            self.with_base(|base| {
                let _ = base.run(distance);
            });
            self.refresh();
            return;
        }
        let Some(target) = self.target else {
            self.refresh();
            return;
        };
        let seen = self.visible(None);
        if seen {
            self.state.search_until = self.game.time + 5.0;
        }
        if self.search_for_coop_target() {
            self.refresh();
            return;
        }
        self.make_vectors();
        if seen && self.try_attack() {
            self.refresh();
            return;
        }
        self.delay(0.1);
        let offset = if self.lefty { 40.0 } else { -40.0 };
        if self.game.time > self.entity.number("ltime") {
            self.lefty = !self.lefty;
            let until = self.game.time + 0.8;
            self.entity
                .fields
                .insert("ltime".to_string(), (until as f32).to_string());
        }
        self.entity.ideal_yaw = yaw_for(vsub(target, self.origin));
        let yaw = self.entity.ideal_yaw;
        self.flush_entity();
        let owned = self.entity.actor.clone();
        if self.game.host.walk_move(&owned, yaw + offset, distance) {
            self.change_yaw();
            self.refresh();
            return;
        }
        self.lefty = !self.lefty;
        let until = self.game.time + 0.8;
        self.entity
            .fields
            .insert("ltime".to_string(), (until as f32).to_string());
        self.flush_entity();
        self.game.host.walk_move(&owned, yaw - offset, distance);
        self.change_yaw();
        self.refresh();
    }

    /// Chase the enemy (`moveToEnemy`).
    pub fn move_to_enemy(&mut self, distance: f64) {
        self.sync();
        if pack_id(&self.runtime.pack) == "hipnotic" {
            let world = self.game.world.clone();
            let run_straight = world
                .as_ref()
                .and_then(|world| self.game.entity(world))
                .map(|entity| entity.number("RUN_STRAIGHT"))
                .unwrap_or(0.0);
            if run_straight != 0.0 && self.game.time > self.entity.number("endtime") {
                if let Some(world) = world {
                    let _ = self.game.update_entity(&world, |entity| {
                        entity.fields.insert("RUN_STRAIGHT".to_string(), "0".to_string());
                    });
                }
                let id = self.entity.actor.id.clone();
                let yaw = self
                    .game
                    .body(&id)
                    .map(|body| f64::from(body.angles.y))
                    .unwrap_or(0.0);
                let owned = self.entity.actor.clone();
                if self.game.host.walk_move(&owned, yaw, distance) {
                    self.refresh();
                    return;
                }
                let endtime = self.game.time + 3.0;
                self.entity
                    .fields
                    .insert("endtime".to_string(), (endtime as f32).to_string());
                self.flush_entity();
            }
        }
        self.with_base(|base| {
            let _ = base.move_to_enemy(distance);
        });
        self.refresh();
    }

    /// Cooperative target search (`searchForCoopTarget`). Charmed monsters
    /// never pick up co-op targets.
    pub fn search_for_coop_target(&mut self) -> bool {
        self.sync();
        let hit = self.entity.number("charmed") == 0.0
            && self.with_base(|base| base.search_for_coop_target().unwrap_or(false));
        self.refresh();
        hit
    }

    /// Refresh the angle basis (`makeVectors`).
    pub fn make_vectors(&mut self) -> Q1Basis {
        self.sync();
        let id = self.entity.actor.id.clone();
        let angles = self.game.body(&id).map(|body| body.angles).unwrap_or(ZERO);
        let basis = self.game.make_vectors(angles);
        self.refresh();
        basis
    }

    /// Scan for a target (`findTarget`).
    pub fn find_target(&mut self) -> bool {
        self.sync();
        let found = if pack_id(&self.runtime.pack) == "hipnotic" {
            find_charmed_target(self).unwrap_or_else(|| find_hipnotic_target(self))
        } else {
            self.with_base(|base| base.find_target().unwrap_or(false))
        };
        self.refresh();
        found
    }

    /// Retaliate (`retaliate`). Never retaliates against the charmer.
    pub fn retaliate(&mut self, attacker: Option<&ActorId>) {
        self.sync();
        let owner = charmer(self);
        let is_owner = match (attacker, owner) {
            (Some(attacker), Some(owner)) => attacker == &owner,
            _ => false,
        };
        if !is_owner {
            let attacker = attacker.cloned();
            self.with_base(|base| {
                let _ = base.retaliate(attacker.as_ref());
            });
        }
        self.refresh();
    }

    /// Use (`use`).
    pub fn use_entity(&mut self, activator: Option<&ActorId>) {
        self.sync();
        match self.definition.use_.clone() {
            Some(use_) => {
                let activator = activator.cloned();
                use_(self, activator.as_ref());
            }
            None => {
                let activator = activator.cloned();
                self.with_base(|base| {
                    let _ = base.use_monster(activator.as_ref());
                });
            }
        }
        self.refresh();
    }

    /// Delay the next frame (`delay`).
    pub fn delay(&mut self, seconds: f64) {
        self.with_base(|base| {
            let _ = base.delay(seconds);
        });
    }

    /// Set the attack cooldown (`attackFinished`).
    pub fn attack_finished(&mut self, seconds: f64) {
        self.with_base(|base| base.attack_finished(seconds));
    }

    /// Turn toward the ideal yaw (`changeYaw`).
    pub fn change_yaw(&mut self) {
        self.with_base(|base| {
            let _ = base.change_yaw();
        });
    }

    /// Face the enemy (`face`).
    pub fn face(&mut self) {
        self.with_base(|base| {
            let _ = base.face();
        });
    }

    /// Eye position (`eye`).
    pub fn eye(&mut self, target: Option<&ActorId>) -> Option<Vec3> {
        self.with_base(|base| base.eye(target).unwrap_or(None))
    }

    /// Range to the enemy in 400-unit units (`rangeDistance`).
    pub fn range_distance(&mut self, target: Option<&ActorId>) -> f64 {
        self.with_base(|base| base.range_distance(target).unwrap_or(f64::INFINITY))
    }

    /// Distance to the current target (`distance`).
    pub fn distance(&mut self) -> f64 {
        match self.target {
            Some(target) => f64::from(length(vsub(target, self.origin))),
            None => f64::INFINITY,
        }
    }

    /// Visibility check (`visible`).
    pub fn visible(&mut self, target: Option<&ActorId>) -> bool {
        self.with_base(|base| base.visible(target).unwrap_or(false))
    }

    /// Count a kill (`countKill`).
    pub fn count_kill(&mut self) {
        self.with_base(|base| {
            let _ = base.count_kill();
        });
    }

    /// Patrol-route advance (`updateRoute`, for the route callback).
    fn update_route(&mut self) {
        self.with_base(|base| {
            let _ = base.update_route();
        });
    }

    /// Capture controller checkpoint state (`capture`).
    #[must_use]
    pub fn capture(&self) -> SaveJson {
        self.controller.capture()
    }

    /// Restore controller checkpoint state (`restore`). Pack frames live in
    /// definitions rather than the base table, so frame names are not
    /// validated here.
    pub fn restore(&mut self, reader: SaveReader) -> Result<(), Q1Error> {
        self.sync();
        self.controller = BaseMonsterState {
            species: self.spec.species,
            prefix: self.prefix(),
            current_frame: reader.field("currentFrame").string()?,
            next_frame: reader.field("nextFrame").string()?,
            in_pain: reader.field("inPain").number()?,
            counter: reader.field("counter").number()?,
            idle_until: reader.field("idleUntil").number()?,
            lefty: reader.field("lefty").boolean()?,
            sliding: reader.field("sliding").boolean()?,
            lightning_count: reader.field("lightningCount").number()?,
            counted_death: reader.field("countedDeath").boolean()?,
        };
        self.lefty = self.controller.lefty;
        self.next_frame = self.controller.next_frame.clone();
        self.counted_death = self.controller.counted_death;
        let owned = self.entity.actor.clone();
        self.runtime
            .controllers
            .insert(owned, self.controller.clone());
        self.refresh();
        Ok(())
    }
}

/// Mission-pack monster runtime (`Q1MissionPackMonsters`).
///
/// Unlike the donor, the runtime does not hold the game or the base
/// provider: both are passed per call because the game is mutably borrowed
/// by every view. Definitions are leaked at registration so views and the
/// frame driver can borrow them; per-actor controllers live in
/// `controllers` and are checkpointed by the state extension.
#[derive(Clone)]
pub struct Q1MissionPackMonsters {
    /// Authored gremlin count (`authoredGremlins`).
    pub authored_gremlins: u32,
    /// Split-spawned gremlin count (`spawnedGremlins`).
    pub spawned_gremlins: u32,
    /// Mission pack.
    pub pack: Q1MissionPack,
    /// Runtime hooks.
    pub hooks: MissionMonsterHooks,
    /// Definitions by classname.
    definitions: HashMap<String, &'static PackMonsterDefinition>,
    /// Per-actor controllers.
    controllers: HashMap<OwnedActor, BaseMonsterState>,
    /// Whether pack callbacks and extensions are installed.
    installed: bool,
}

impl Q1MissionPackMonsters {
    /// Fresh runtime (`constructor`). Installs pack sources, frame-driver
    /// callbacks, checkpointing, and the release hook on the game, then
    /// publishes itself as the installed pack runtime for the `fn`
    /// callbacks. There is no base monster registry; views load straight
    /// from the definitions and controllers stored here.
    pub fn new(
        game: &mut Q1EntityServices,
        pack: Q1MissionPack,
        hooks: MissionMonsterHooks,
    ) -> Result<Self, Q1Error> {
        let mut this = Self::uninstalled(pack, hooks);
        this.ensure_installed(game)?;
        *lock_pack(&this.pack) = this.clone();
        Ok(this)
    }

    /// Uninstalled runtime for the static slots.
    fn uninstalled(pack: Q1MissionPack, hooks: MissionMonsterHooks) -> Self {
        Self {
            authored_gremlins: 0,
            spawned_gremlins: 0,
            pack,
            hooks,
            definitions: HashMap::new(),
            controllers: HashMap::new(),
            installed: false,
        }
    }

    /// Definition for a classname, if registered.
    #[must_use]
    pub fn definition(&self, classname: &str) -> Option<&'static PackMonsterDefinition> {
        self.definitions.get(classname).copied()
    }

    /// Whether an actor has a mission controller.
    #[must_use]
    pub fn contains(&self, actor: &OwnedActor) -> bool {
        self.controllers.contains_key(actor)
    }

    /// Load a controller view (`require`).
    pub fn require<'m>(
        &'m mut self,
        game: &'m mut Q1EntityServices,
        id: &ActorId,
    ) -> Result<MissionMonster<'m>, Q1Error> {
        let prefix = pack_id(&self.pack);
        let entity = game
            .entity_ref(id)
            .cloned()
            .ok_or_else(|| q1_error(format!("Missing {prefix} monster entity")))?;
        let definition = self.definitions.get(&entity.classname).copied().ok_or_else(|| {
            q1_error(format!("Unknown {prefix} monster {}", entity.classname))
        })?;
        let owned = entity.actor.clone();
        let controller = self.controllers.get(&owned).cloned().unwrap_or_else(|| {
            BaseMonsterState::new(definition.spec.species, prefix.to_string(), definition.spec.stand)
        });
        let state = entity
            .monster
            .clone()
            .unwrap_or_else(|| default_monster_state(definition.spec.species, &entity.target));
        game.update_entity(id, |entity| {
            entity
                .fields
                .insert("source.monsterCallbackPrefix".to_string(), prefix.to_string());
            if entity.monster.is_none() {
                entity.monster = Some(state.clone());
            }
        })?;
        let entity = game
            .entity_ref(id)
            .cloned()
            .ok_or_else(|| q1_error(format!("Missing {prefix} monster entity")))?;
        let origin = game.body(id).map(|body| body.origin).unwrap_or(ZERO);
        let enemy = state.enemy.clone();
        let target = enemy
            .as_ref()
            .and_then(|enemy| game.host.bodies.read(enemy))
            .map(|body| body.origin);
        Ok(MissionMonster {
            game,
            entity,
            definition,
            runtime: self,
            spec: definition.spec,
            state,
            origin,
            enemy,
            target,
            lefty: controller.lefty,
            next_frame: controller.next_frame.clone(),
            counted_death: controller.counted_death,
            controller,
        })
    }

    /// Load a controller view for an actor, if mission-controlled (`context`).
    pub fn context<'m>(
        &'m mut self,
        game: &'m mut Q1EntityServices,
        actor: &ActorId,
    ) -> Option<MissionMonster<'m>> {
        let owned = game.host.actors.resolve_owned(actor)?;
        if !self.controllers.contains_key(&owned) {
            return None;
        }
        let id = owned.id().clone();
        self.require(game, &id).ok()
    }

    /// Register a definition (`register`).
    pub fn register(
        &mut self,
        game: &mut Q1EntityServices,
        definition: PackMonsterDefinition,
    ) -> Result<(), Q1Error> {
        self.ensure_installed(game)?;
        let prefix = pack_id(&self.pack);
        for (name, handlers) in &definition.callbacks {
            game.named
                .register(&format!("{prefix}:{name}"), copy_handlers(handlers))?;
        }
        let spec = definition.spec;
        let leaked: &'static PackMonsterDefinition = Box::leak(Box::new(definition));
        for classname in spec.classnames {
            if self.definitions.contains_key(*classname) {
                return Err(q1_error(format!("Duplicate {prefix} monster {classname}")));
            }
            self.definitions.insert((*classname).to_string(), leaked);
            game.register_spawn(classname, self.spawn_handler())?;
        }
        *lock_pack(&self.pack) = self.clone();
        Ok(())
    }

    /// Spawn handler (`spawn`).
    pub fn spawn(&mut self, game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
        if game.options().deathmatch != 0 {
            return game.remove(id);
        }
        let id = id.clone();
        let prefix = pack_id(&self.pack);
        let entity = game
            .entity_ref(&id)
            .cloned()
            .ok_or_else(|| q1_error(format!("Missing {prefix} monster entity")))?;
        let definition = self.definitions.get(&entity.classname).copied().ok_or_else(|| {
            q1_error(format!("Unknown {prefix} monster {}", entity.classname))
        })?;
        let controller = BaseMonsterState::new(
            definition.spec.species,
            prefix.to_string(),
            definition.spec.stand,
        );
        self.controllers.insert(entity.actor.clone(), controller);
        let mut monster = self.require(game, &id)?;
        monster.spawn();
        monster.finish();
        Ok(())
    }

    /// Charm a base monster into mission control (`charm`). Associated
    /// function (used as a `fn` pointer): the pack is picked by definition
    /// availability, Hipnotic first. Deviation: the Rust base game keeps no
    /// monster controller registry, so charm builds a fresh controller
    /// instead of migrating the base one.
    pub fn charm(game: &mut Q1EntityServices, id: &ActorId, owner: &ActorId) -> Result<(), Q1Error> {
        let classname = game
            .entity_ref(id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        let pack = definition_pack(&classname)
            .ok_or_else(|| q1_error(format!("Missing source charm controller for {classname}")))?;
        let prefix = pack_id(&pack);
        let id = id.clone();
        let owner = owner.clone();
        game.update_entity(&id, |entity| {
            entity.references.insert("charmer".to_string(), Some(owner));
            entity.fields.insert("charmed".to_string(), "1".to_string());
        })?;
        let mut local = lock_pack(&pack).clone();
        let owned = game
            .entity_ref(&id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error(format!("Missing {prefix} charm entity")))?;
        if !local.controllers.contains_key(&owned) {
            let definition = local.definitions.get(&classname).copied().ok_or_else(|| {
                q1_error(format!("Missing source charm controller for {classname}"))
            })?;
            local.controllers.insert(
                owned,
                BaseMonsterState::new(
                    definition.spec.species,
                    prefix.to_string(),
                    definition.spec.stand,
                ),
            );
            game.update_entity(&id, |entity| {
                entity
                    .fields
                    .insert("source.monsterCallbackPrefix".to_string(), prefix.to_string());
            })?;
            let current = game
                .entity_ref(&id)
                .cloned()
                .ok_or_else(|| q1_error(format!("Missing {prefix} charm entity")))?;
            let rename = |callback: &Option<String>| -> Option<String> {
                let name = callback_name(callback.as_ref())?;
                if let Some(rest) = name.strip_prefix("base:") {
                    Some(format!("{prefix}:{rest}"))
                } else {
                    Some(name)
                }
            };
            let think = rename(&current.think).map(|name| game.named.action(&name)).transpose()?;
            let use_callback = rename(&current.use_callback)
                .map(|name| game.named.use_callback(&name))
                .transpose()?;
            let touch = rename(&current.touch).map(|name| game.named.touch(&name)).transpose()?;
            let pain = rename(&current.pain).map(|name| game.named.pain(&name)).transpose()?;
            let die = rename(&current.die).map(|name| game.named.die(&name)).transpose()?;
            let path_end = rename(&current.path_end)
                .map(|name| game.named.action(&name))
                .transpose()?;
            game.update_entity(&id, |entity| {
                entity.think = think;
                entity.use_callback = use_callback;
                entity.touch = touch;
                entity.pain = pain;
                entity.die = die;
                entity.path_end = path_end;
            })?;
        }
        *lock_pack(&pack) = local;
        Ok(())
    }

    /// Spawn-function pointer for this pack.
    fn spawn_handler(&self) -> Q1SpawnHandler {
        match &self.pack {
            Q1MissionPack::Hipnotic => hipnotic_spawn_handler,
            Q1MissionPack::Rogue => rogue_spawn_handler,
        }
    }

    /// Install pack sources, frame-driver callbacks, checkpointing, and the
    /// release hook. Runs once, on the first registration.
    fn ensure_installed(&mut self, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        if self.installed {
            return Ok(());
        }
        self.installed = true;
        let prefix = pack_id(&self.pack);
        register_monster_source(MonsterSourceRegistration {
            source: MonsterSource {
                prefix,
                frames: None,
                actions: None,
            },
            load: mission_load_controller,
            store: mission_store_controller,
        });
        let action = |handler: crate::q1::foundation::callbacks::Q1ActionHandler| {
            Q1CallbackHandlers {
                action: Some(handler),
                ..Default::default()
            }
        };
        game.named.register(
            &format!("{prefix}:monster_jump_touch"),
            Q1CallbackHandlers {
                touch: Some(mission_jump_touch_handler),
                ..Default::default()
            },
        )?;
        game.named.register(
            &format!("{prefix}:monster_frame"),
            action(mission_frame_handler),
        )?;
        game.named.register(
            &format!("{prefix}:monster_route"),
            action(mission_route_handler),
        )?;
        game.named.register(
            &format!("{prefix}:monster_start"),
            action(mission_start_handler),
        )?;
        game.named.register(
            &format!("{prefix}:monster_stand"),
            action(mission_stand_handler),
        )?;
        game.named.register(
            &format!("{prefix}:monster_found"),
            action(mission_found_handler),
        )?;
        game.named.register(
            &format!("{prefix}:monster_pain"),
            Q1CallbackHandlers {
                pain: Some(mission_pain_handler),
                ..Default::default()
            },
        )?;
        game.named.register(
            &format!("{prefix}:monster_die"),
            Q1CallbackHandlers {
                die: Some(mission_die_handler),
                ..Default::default()
            },
        )?;
        game.named.register(
            &format!("{prefix}:monster_use"),
            Q1CallbackHandlers {
                use_callback: Some(mission_use_handler),
                ..Default::default()
            },
        )?;
        game.named.register(
            &format!("{prefix}:boss_awake"),
            Q1CallbackHandlers {
                use_callback: Some(mission_awake_handler),
                ..Default::default()
            },
        )?;
        if matches!(self.pack, Q1MissionPack::Hipnotic) {
            for spec in BASE_SPECIES {
                for classname in spec.classnames {
                    let definition = PackMonsterDefinition {
                        spec,
                        base_behavior: true,
                        frames: &[],
                        actions: Vec::new(),
                        callbacks: Vec::new(),
                        spawn: None,
                        start: None,
                        pain: Rc::new(|monster, attacker, damage| {
                            monster.pain_default(attacker, damage);
                        }),
                        die: Rc::new(|monster, attacker| {
                            monster.die_default(attacker);
                        }),
                        melee: None,
                        check_attack: None,
                        found: None,
                        ai: None,
                        use_: None,
                    };
                    let leaked: &'static PackMonsterDefinition = Box::leak(Box::new(definition));
                    self.definitions.insert((*classname).to_string(), leaked);
                    if !matches!(spec.movement, MonsterMovement::Boss) {
                        let _ = game.replace_spawn(classname, self.spawn_handler());
                    }
                }
            }
        }
        game.register_state_extension(Box::new(MissionMonstersExtension {
            id: format!("q1:{prefix}:monsters"),
            prefix,
        }))?;
        game.register_release_hook(Rc::new(RefCell::new(MissionMonstersRelease { prefix })));
        Ok(())
    }
}

/// Installed per-pack runtimes backing the `fn` spawn and named callbacks.
static HIPNOTIC_MONSTERS: OnceLock<Mutex<Q1MissionPackMonsters>> = OnceLock::new();
/// Installed per-pack runtimes backing the `fn` spawn and named callbacks.
static ROGUE_MONSTERS: OnceLock<Mutex<Q1MissionPackMonsters>> = OnceLock::new();

/// Installed runtime for a pack.
#[must_use]
pub fn mission_pack_monsters(pack: &Q1MissionPack) -> &'static Mutex<Q1MissionPackMonsters> {
    match pack {
        Q1MissionPack::Hipnotic => HIPNOTIC_MONSTERS.get_or_init(|| {
            Mutex::new(Q1MissionPackMonsters::uninstalled(
                Q1MissionPack::Hipnotic,
                MissionMonsterHooks::default(),
            ))
        }),
        Q1MissionPack::Rogue => ROGUE_MONSTERS.get_or_init(|| {
            Mutex::new(Q1MissionPackMonsters::uninstalled(
                Q1MissionPack::Rogue,
                MissionMonsterHooks::default(),
            ))
        }),
    }
}

fn lock_pack(pack: &Q1MissionPack) -> MutexGuard<'static, Q1MissionPackMonsters> {
    mission_pack_monsters(pack)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn pack_for_entity(game: &Q1EntityServices, id: &ActorId) -> Option<Q1MissionPack> {
    let prefix = game
        .entity_ref(id)?
        .fields
        .get("source.monsterCallbackPrefix")?;
    match prefix.as_str() {
        "hipnotic" => Some(Q1MissionPack::Hipnotic),
        "rogue" => Some(Q1MissionPack::Rogue),
        _ => None,
    }
}

/// Pick the pack runtime holding a definition, Hipnotic first.
fn definition_pack(classname: &str) -> Option<Q1MissionPack> {
    for pack in [Q1MissionPack::Hipnotic, Q1MissionPack::Rogue] {
        if lock_pack(&pack).definitions.contains_key(classname) {
            return Some(pack);
        }
    }
    None
}

/// Run a dispatched callback against a locally cloned runtime. The static
/// lock is never held across game calls, so synchronous reentrant
/// dispatches (damage during a frame action) cannot deadlock.
fn with_pack_view<R>(
    game: &mut Q1EntityServices,
    id: &ActorId,
    run: impl FnOnce(&mut MissionMonster) -> R,
) -> Result<Option<R>, Q1Error> {
    let Some(pack) = pack_for_entity(game, id) else {
        return Ok(None);
    };
    let mut local = lock_pack(&pack).clone();
    let mut monster = local.require(game, id)?;
    let result = run(&mut monster);
    monster.finish();
    *lock_pack(&pack) = local;
    Ok(Some(result))
}

fn hipnotic_spawn_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let mut local = lock_pack(&Q1MissionPack::Hipnotic).clone();
    local.spawn(game, &id)?;
    *lock_pack(&Q1MissionPack::Hipnotic) = local;
    Ok(())
}

fn rogue_spawn_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let mut local = lock_pack(&Q1MissionPack::Rogue).clone();
    local.spawn(game, &id)?;
    *lock_pack(&Q1MissionPack::Rogue) = local;
    Ok(())
}

fn mission_frame_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    with_pack_view(game, &id, |monster| {
        let next = monster.next_frame.clone();
        monster.play(&next);
    })?;
    Ok(())
}

fn mission_route_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    with_pack_view(game, &id, |monster| {
        monster.update_route();
    })?;
    Ok(())
}

fn mission_start_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    with_pack_view(game, &id, |monster| {
        monster.start();
    })?;
    Ok(())
}

fn mission_stand_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    with_pack_view(game, &id, |monster| {
        let stand = monster.spec.stand;
        monster.play(stand);
    })?;
    Ok(())
}

fn mission_found_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    with_pack_view(game, &id, |monster| {
        if let Some(enemy) = monster.enemy.clone() {
            monster.found(&enemy);
        }
    })?;
    Ok(())
}

fn mission_pain_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
    damage: f64,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let attacker = attacker.cloned();
    with_pack_view(game, &id, |monster| {
        monster.pain(attacker.as_ref(), damage);
    })?;
    Ok(())
}

fn mission_die_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let attacker = attacker.cloned();
    with_pack_view(game, &id, |monster| {
        monster.die(attacker.as_ref());
    })?;
    Ok(())
}

fn mission_use_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let activator = activator.cloned().or_else(|| other.cloned());
    with_pack_view(game, &id, |monster| {
        monster.use_entity(activator.as_ref());
    })?;
    Ok(())
}

fn mission_awake_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let activator = activator.cloned().or_else(|| other.cloned());
    with_pack_view(game, &id, |monster| {
        monster.with_base(|base| {
            let _ = base.awake(activator.as_ref());
        });
    })?;
    Ok(())
}

fn mission_jump_touch_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    with_pack_view(game, &id, |monster| {
        monster.with_base(|base| {
            let _ = monster_jump_touch(base, &other);
        });
    })?;
    Ok(())
}

fn mission_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let pack = pack_for_entity(game, id).ok_or_else(|| q1_error("Unknown mission monster source"))?;
    let guard = lock_pack(&pack);
    let definition = guard
        .definitions
        .get(classname)
        .copied()
        .ok_or_else(|| q1_error(format!("Unknown mission monster {classname}")))?;
    let owned = game
        .entity_ref(id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing mission monster entity"))?;
    let controller = guard.controllers.get(&owned).cloned().unwrap_or_else(|| {
        BaseMonsterState::new(
            definition.spec.species,
            pack_id(&pack).to_string(),
            definition.spec.stand,
        )
    });
    Ok((controller, definition.spec))
}

fn mission_store_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    controller: BaseMonsterState,
) -> Result<(), Q1Error> {
    let pack = pack_for_entity(game, id).ok_or_else(|| q1_error("Unknown mission monster source"))?;
    let owned = game
        .entity_ref(id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing mission monster entity"))?;
    lock_pack(&pack).controllers.insert(owned, controller);
    Ok(())
}

/// Checkpoint extension (`q1:{pack}:monsters`).
struct MissionMonstersExtension {
    /// Extension id.
    id: String,
    /// Pack id text.
    prefix: &'static str,
}

impl MissionMonstersExtension {
    fn pack(&self) -> Q1MissionPack {
        match self.prefix {
            "hipnotic" => Q1MissionPack::Hipnotic,
            _ => Q1MissionPack::Rogue,
        }
    }
}

impl Q1StateExtension for MissionMonstersExtension {
    fn id(&self) -> &str {
        &self.id
    }

    fn capture(&self, game: &Q1EntityServices) -> Vec<u8> {
        let guard = lock_pack(&self.pack());
        let mut monsters = Vec::new();
        for (owned, controller) in guard.controllers.iter() {
            let Some(entity) = game.entity_ref(owned.id()) else {
                continue;
            };
            monsters.push(obj(vec![
                (
                    "actor",
                    obj(vec![
                        ("slot", int(i64::from(owned.id().slot()))),
                        ("generation", int(i64::from(owned.id().generation()))),
                    ]),
                ),
                ("definition", save_str(&entity.classname)),
                ("state", controller.capture()),
            ]));
        }
        encode_checkpoint_value(&obj(vec![
            ("version", int(1)),
            ("authoredGremlins", int(i64::from(guard.authored_gremlins))),
            ("spawnedGremlins", int(i64::from(guard.spawnedGremlins))),
            ("monsters", arr(monsters)),
        ]))
    }

    fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
        let value = decode_checkpoint_value(bytes)?;
        let root = SaveReader::at(&value, &self.id);
        root.field("version").literal_i64(1)?;
        let authored = root.field("authoredGremlins").number()? as u32;
        let spawned = root.field("spawnedGremlins").number()? as u32;
        let entries = root.field("monsters").list(|saved| {
            let actor = saved.field("actor");
            let slot = actor.field("slot").integer(0)?;
            let generation = actor.field("generation").integer(0)?;
            let definition = saved.field("definition").string()?;
            let state = saved.field("state");
            Ok::<_, Q1Error>((slot, generation, definition, state))
        })?;
        let mut guard = lock_pack(&self.pack());
        guard.authored_gremlins = authored;
        guard.spawned_gremlins = spawned;
        guard.controllers.clear();
        for (slot, generation, definition, state) in entries {
            let saved = SavedActorId {
                slot: u32::try_from(slot).unwrap_or(u32::MAX),
                generation: u32::try_from(generation).unwrap_or(u32::MAX),
            };
            let owned = game
                .host
                .actors
                .resolve_saved(&saved)
                .ok_or_else(|| Q1Error::from(root.fail("missing saved monster actor")))?;
            if game.entity_ref(owned.id()).is_none() {
                return Err(Q1Error::from(root.fail("missing saved monster source entity")));
            }
            let definition = guard
                .definitions
                .get(&definition)
                .copied()
                .ok_or_else(|| Q1Error::from(root.fail("unknown saved mission pack monster")))?;
            guard.controllers.insert(
                owned,
                BaseMonsterState {
                    species: definition.spec.species,
                    prefix: self.prefix.to_string(),
                    current_frame: state.field("currentFrame").string()?,
                    next_frame: state.field("nextFrame").string()?,
                    in_pain: state.field("inPain").number()?,
                    counter: state.field("counter").number()?,
                    idle_until: state.field("idleUntil").number()?,
                    lefty: state.field("lefty").boolean()?,
                    sliding: state.field("sliding").boolean()?,
                    lightning_count: state.field("lightningCount").number()?,
                    counted_death: state.field("countedDeath").boolean()?,
                },
            );
        }
        Ok(())
    }

    fn clone_state(
        &mut self,
        game: &mut Q1EntityServices,
        source: &ActorId,
        target: &ActorId,
    ) -> Result<(), Q1Error> {
        let source_owned = game.host.actors.resolve_owned(source);
        let target_owned = game.host.actors.resolve_owned(target);
        let (Some(source_owned), Some(target_owned)) = (source_owned, target_owned) else {
            return Ok(());
        };
        let mut guard = lock_pack(&self.pack());
        if let Some(controller) = guard.controllers.get(&source_owned).cloned() {
            guard.controllers.insert(target_owned, controller);
        }
        Ok(())
    }
}

/// Actor-release cleanup.
struct MissionMonstersRelease {
    /// Pack id text.
    prefix: &'static str,
}

impl Q1ReleaseHook for MissionMonstersRelease {
    fn on_release(&mut self, _game: &mut Q1EntityServices, actor: &OwnedActor) {
        let pack = match self.prefix {
            "hipnotic" => Q1MissionPack::Hipnotic,
            _ => Q1MissionPack::Rogue,
        };
        lock_pack(&pack).controllers.remove(actor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::q1::foundation::entity::Q1MonsterSpecies;
    use crate::q1::missionpacks::types::test_game;

    use super::super::helpers::HULL_BOUNDS;

    fn test_definition(pinged: Rc<Cell<bool>>) -> PackMonsterDefinition {
        PackMonsterDefinition {
            spec: Box::leak(Box::new(MonsterSpecies {
                species: Q1MonsterSpecies::Gremlin,
                kill_string: None,
                classnames: &["monster_gremlin"],
                model: "grem",
                head: None,
                health: 100.0,
                gib_health: -35.0,
                gibs: &[],
                bounds: HULL_BOUNDS,
                stand: "gremlin_stand1",
                walk: "gremlin_walk1",
                run: "gremlin_run1",
                sight: "",
                missile: None,
                melee: true,
                movement: MonsterMovement::Walk,
            })),
            base_behavior: false,
            frames: &[],
            actions: vec![(
                "ping",
                Rc::new(move |_monster: &mut MissionMonster| {
                    pinged.set(true);
                }),
            )],
            callbacks: Vec::new(),
            spawn: None,
            start: None,
            pain: Rc::new(|_, _, _| {}),
            die: Rc::new(|_, _| {}),
            melee: None,
            check_attack: None,
            found: None,
            ai: None,
            use_: None,
        }
    }

    #[test]
    fn require_and_play_action() {
        let pinged = Rc::new(Cell::new(false));
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default()).expect("new");
        runtime.register(&mut game, test_definition(pinged.clone())).expect("register");
        let id = game.create("monster_gremlin", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.spec.model, "grem");
        monster.play("ping");
        assert!(pinged.get());
        monster.finish();
    }

    #[test]
    fn charm_is_noop_when_controlled() {
        let pinged = Rc::new(Cell::new(false));
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default()).expect("new");
        runtime.register(&mut game, test_definition(pinged)).expect("register");
        let id = game.create("monster_gremlin", None, None).expect("create");
        runtime.spawn(&mut game, &id).expect("spawn");
        let owner = game.create("player", None, None).expect("owner");
        Q1MissionPackMonsters::charm(&mut game, &id, &owner).expect("charm");
        let entity = game.entity(&id).expect("entity").clone();
        assert_eq!(entity.number("charmed"), 1.0);
    }
}
