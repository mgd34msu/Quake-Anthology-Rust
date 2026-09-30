//! Entity services (`src/content/q2/foundation/entity-services.ts`).
//!
//! One provider's actors and source callbacks; the session still owns the
//! frame and all mutation authorities.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{add3, scale3, sub3, Vec3};
use qa_core::time::SourceTime;

use super::checkpoint::{
    restore_q2_actor, restore_q2_attack, save_q2_attack, Q2EntityCallbackNames, Q2EntityCheckpoint,
    Q2EntityLinks, Q2EntityValues, Q2FoundationCheckpoint,
};
use super::fields::{integer_field, number_field, vector_field, ZERO};
use super::host::{
    Q2Edition, Q2Entity, Q2GameServices, Q2Motion, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2SpawnFields, Q2Think, Q2WeaponTarget,
};
use crate::contract::{ArmorState, ItemId, PoweredProtectionState, RegularArmorState};
use crate::monsters::{AuthoredTarget, MonsterTargetObservation};
use crate::q2::support::contracts::{
    AttackCause, AttackProvenance, BodyState, CombatState, CombatTraitChanges, DamageDelivery, DamageOutcome,
    DamageRequest, DeathReaction, PainReaction, TouchContact, TraceHit, WeaponTrajectoryUpdate,
};
use crate::q2::support::misc::apply_source_damage_modifier;

const MASK_SOLID: i32 = 3;

/// Delayed use think callback (`delayedUse`).
pub fn delayed_use(actor: ActorId, game: &mut Q2GameServices) {
    let target = game.require_entity(&actor).authored_target();
    let activator = game.require_entity(&actor).activator.clone();
    game.use_targets(&target, activator.as_ref(), false);
    game.remove_actor(actor);
}

/// Free an entity die callback (donor registers `freeQ2Entity` as die).
pub fn free_q2_entity_die(actor: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    game.remove_actor(actor);
}

impl Q2GameServices {
    /// Team members from the team master (`pushTeam`).
    pub fn push_team(&self, actor: &ActorId) -> Vec<OwnedActor> {
        let entity = match self.entity(actor) {
            Some(entity) => entity,
            None => return Vec::new(),
        };
        let start = entity
            .team_master
            .as_ref()
            .and_then(|master| self.entity(master))
            .map_or_else(|| actor.clone(), |master| master.actor.id().clone());
        let mut members = Vec::new();
        let mut current = Some(start);
        while let Some(next) = current {
            let Some(entity) = self.entity(&next) else {
                break;
            };
            members.push(entity.actor.clone());
            current = entity.team_chain.clone();
        }
        members
    }

    /// Capture the foundation checkpoint (`capture`).
    pub fn capture_foundation(&self) -> Q2FoundationCheckpoint {
        if self.current_actor.is_some() {
            panic!("Q2 source saves require a completed callback boundary");
        }
        let mut actors: Vec<&ActorId> = self.entities.keys().collect();
        actors.sort_by_key(|actor| (actor.slot(), actor.generation()));
        let mut entities = Vec::new();
        for actor in actors {
            let entity = &self.entities[actor];
            entities.push(Q2EntityCheckpoint {
                actor: qa_core::identity::SavedActorId::from(entity.actor.id()),
                source_slot: self.source_slots.get(actor).copied(),
                classname: entity.spawn.classname.clone(),
                ordinal: entity.spawn.ordinal,
                spawn_values: entity
                    .spawn
                    .values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
                values: Q2EntityValues::capture(entity),
                links: Q2EntityLinks::capture(entity),
                last_attack: entity.last_attack.as_ref().map(save_q2_attack),
                callbacks: Q2EntityCallbackNames {
                    think: self.source_callbacks.think_name(entity.think).map(str::to_string),
                    prethink: self.source_callbacks.think_name(entity.prethink).map(str::to_string),
                    postthink: self.source_callbacks.think_name(entity.postthink).map(str::to_string),
                    use_: self.source_callbacks.use_name(entity.use_).map(str::to_string),
                    touch: self.source_callbacks.touch_name(entity.touch).map(str::to_string),
                    pain: self.source_callbacks.pain_name(entity.pain).map(str::to_string),
                    die: self.source_callbacks.die_name(entity.die).map(str::to_string),
                    blocked: self.source_callbacks.blocked_name(entity.blocked).map(str::to_string),
                },
            });
        }
        let mut freed_slots: Vec<(u32, f64)> =
            self.freed_slots.iter().map(|(slot, time)| (*slot, *time)).collect();
        freed_slots.sort_by_key(|(slot, _)| *slot);
        Q2FoundationCheckpoint {
            version: 1,
            next_source_slot: self.next_source_slot,
            sequence: self.sequence,
            freed_slots,
            counters: self.counters,
            entities,
        }
    }

    /// Restore the foundation checkpoint (`restore`).
    ///
    /// Shared lifetimes and authority tables are restored first; no spawn
    /// callback runs here.
    pub fn restore_foundation(&mut self, checkpoint: &Q2FoundationCheckpoint) {
        if self.current_actor.is_some() {
            panic!("Q2 source restore requires a completed callback boundary");
        }
        self.entities.clear();
        self.source_slots.clear();
        self.freed_slots.clear();
        self.unsupported.clear();
        self.next_source_slot = checkpoint.next_source_slot;
        self.sequence = checkpoint.sequence;
        self.counters = checkpoint.counters;
        for (slot, time) in &checkpoint.freed_slots {
            self.freed_slots.insert(*slot, *time);
        }
        for saved in &checkpoint.entities {
            let actor = restore_q2_actor(self, saved.actor);
            if let Some(source_slot) = saved.source_slot {
                let source = self.host.actors().source_of(actor.id());
                let matches = source.as_ref().is_some_and(|(provider, slot)| {
                    provider == &self.options.provider && *slot == source_slot
                });
                if !matches {
                    panic!("Q2 checkpoint source slot disagrees with the shared registry");
                }
            }
            let mut values = BTreeMap::new();
            for (key, value) in &saved.spawn_values {
                values.insert(key.clone(), value.clone());
            }
            let mut entity = Q2Entity::new(
                actor.clone(),
                Q2SpawnFields {
                    ordinal: saved.ordinal,
                    classname: saved.classname.clone(),
                    values,
                },
            );
            saved.values.restore(&mut entity);
            // Borrow dance: resolve all links first, then assign.
            let links = &saved.links;
            let activator = links.activator.map(|saved| self.host.actors().reference_saved(saved));
            let enemy = links.enemy.map(|saved| self.host.actors().reference_saved(saved));
            let owner = links.owner.map(|saved| self.host.actors().reference_saved(saved));
            let goal = links.goal.map(|saved| self.host.actors().reference_saved(saved));
            let team_master = links.team_master.map(|saved| self.host.actors().reference_saved(saved));
            let team_chain = links.team_chain.map(|saved| self.host.actors().reference_saved(saved));
            let chain = links.chain.map(|saved| self.host.actors().reference_saved(saved));
            let beam = links.beam.map(|saved| self.host.actors().reference_saved(saved));
            let beam2 = links.beam2.map(|saved| self.host.actors().reference_saved(saved));
            let proboscus = links.proboscus.map(|saved| self.host.actors().reference_saved(saved));
            entity.activator = activator;
            entity.enemy = enemy;
            entity.owner = owner;
            entity.goal = goal;
            entity.team_master = team_master;
            entity.team_chain = team_chain;
            entity.chain = chain;
            entity.beam = beam;
            entity.beam2 = beam2;
            entity.proboscus = proboscus;
            entity.last_attack = saved.last_attack.as_ref().map(|saved| {
                restore_q2_attack(saved, &mut |saved| self.host.actors().reference_saved(saved))
            });
            let callbacks = &saved.callbacks;
            entity.think = self.source_callbacks.resolve_think(callbacks.think.as_deref());
            entity.prethink = self.source_callbacks.resolve_think(callbacks.prethink.as_deref());
            entity.postthink = self.source_callbacks.resolve_think(callbacks.postthink.as_deref());
            entity.use_ = self.source_callbacks.resolve_use(callbacks.use_.as_deref());
            entity.touch = self.source_callbacks.resolve_touch(callbacks.touch.as_deref());
            entity.pain = self.source_callbacks.resolve_pain(callbacks.pain.as_deref());
            entity.die = self.source_callbacks.resolve_die(callbacks.die.as_deref());
            entity.blocked = self.source_callbacks.resolve_blocked(callbacks.blocked.as_deref());
            let id = actor.id().clone();
            self.entities.insert(id.clone(), entity);
            if let Some(source_slot) = saved.source_slot {
                self.source_slots.insert(id, source_slot);
            }
            self.bind_callbacks(&actor);
            self.host.register_entity(&actor);
        }
    }

    /// Spawn authored fields (`spawn`).
    pub fn spawn(&mut self, fields: Q2SpawnFields) -> ActorId {
        let actor = self.allocate(fields);
        self.spawn_entity(actor.clone());
        actor
    }

    /// Dispatch an allocated entity through the spawn modules
    /// (`spawnEntity`).
    pub fn spawn_entity(&mut self, actor: ActorId) {
        for index in 0..self.modules.len() {
            let spawn = self.modules[index].spawn;
            if spawn(actor.clone(), self) {
                return;
            }
        }
        let ordinal = self.require_entity(&actor).spawn.ordinal;
        let classname = self.require_entity(&actor).classname.clone();
        self.unsupported.insert(actor);
        self.host
            .diagnostic(&format!("Q2 spawn handler not yet imported: {classname} at authored entity {ordinal}"));
    }

    /// Allocate an entity continuation without spawning (`create`).
    pub fn create(&mut self, classname: &str, values: BTreeMap<String, String>) -> ActorId {
        self.allocate(Q2SpawnFields {
            ordinal: -1,
            classname: classname.to_string(),
            values,
        })
    }

    /// Resolve an item display name (`itemName`).
    pub fn item_name(&self, classname: &str) -> Option<String> {
        for module in &self.modules {
            if let Some(name) = (module.item_name)(classname) {
                return Some(name);
            }
        }
        None
    }

    /// Admit a session player actor (`attachPlayer`).
    pub fn attach_player(&mut self, actor: OwnedActor) -> ActorId {
        self.host.actors().assert_owned(&actor);
        if let Some(existing) = self.entities.get(actor.id()) {
            return existing.actor.id().clone();
        }
        let id = actor.id().clone();
        self.attach(
            actor,
            Q2SpawnFields {
                ordinal: -1,
                classname: "player".to_string(),
                values: BTreeMap::new(),
            },
        );
        let entity = self.require_entity_mut(&id);
        entity.max_health = 100.0;
        entity.view_height = 22;
        id
    }

    /// Allocate an actor plus continuation for spawn fields.
    fn allocate(&mut self, fields: Q2SpawnFields) -> ActorId {
        let actor = self.allocate_actor(&fields, None);
        self.attach(actor, fields)
    }

    /// Allocate a source actor (`allocateActor`).
    pub fn allocate_actor(&mut self, fields: &Q2SpawnFields, definition: Option<&str>) -> OwnedActor {
        let mut freed: Vec<(u32, f64)> = self.freed_slots.iter().map(|(slot, time)| (*slot, *time)).collect();
        freed.sort_by_key(|(slot, _)| *slot);
        let now = self.host.now();
        let reusable = freed
            .iter()
            .find(|(_, freed)| *freed < 2.0 || now - *freed > 0.5)
            .map(|(slot, _)| *slot);
        let slot = if fields.classname == "worldspawn" {
            0
        } else {
            reusable.unwrap_or_else(|| {
                let slot = self.next_source_slot;
                self.next_source_slot += 1;
                slot
            })
        };
        self.freed_slots.remove(&slot);
        let definition = definition.map_or_else(|| format!("q2:{}", fields.classname), str::to_string);
        let actor = self.host.actors().allocate_at_source(&self.options.provider, slot, &definition);
        self.source_slots.insert(actor.id().clone(), slot);
        let angles = if fields.values.contains_key("angles") {
            vector_field(fields, "angles")
        } else {
            Vec3 {
                x: 0.0,
                y: number_field(fields, "angle", 0.0) as f32,
                z: 0.0,
            }
        };
        self.host.bodies().create(
            &actor,
            &BodyState {
                origin: vector_field(fields, "origin"),
                angles,
                velocity: ZERO,
                bounds: qa_core::math::Bounds { min: ZERO, max: ZERO },
                ground: None,
            },
        );
        actor
    }

    /// Attach a continuation to an existing shared actor (`attach`).
    pub fn attach(&mut self, actor: OwnedActor, fields: Q2SpawnFields) -> ActorId {
        self.host.actors().assert_owned(&actor);
        if self.entities.contains_key(actor.id()) {
            panic!("Q2 actor already has an entity continuation");
        }
        if self.host.bodies().read(actor.id()).is_none() {
            panic!("Q2 entity attachment requires an existing shared body");
        }
        let mut entity = Q2Entity::new(actor.clone(), fields);
        let rerelease = self.options.edition == Q2Edition::Rerelease;
        entity.spawnflags = integer_field(&entity.spawn, "spawnflags", 0) & !(if rerelease { 0xff00 } else { 0x1f00 });
        entity.delay = number_field(&entity.spawn, "delay", 0.0);
        entity.wait = number_field(&entity.spawn, "wait", 0.0);
        entity.speed = number_field(&entity.spawn, "speed", 0.0);
        entity.accel = number_field(&entity.spawn, "accel", 0.0);
        entity.decel = number_field(&entity.spawn, "decel", 0.0);
        entity.damage = f64::from(integer_field(&entity.spawn, "dmg", 0));
        entity.count = integer_field(&entity.spawn, "count", 0);
        entity.max_health = integer_field(&entity.spawn, "health", 0) as f64;
        entity.noise = entity.spawn.values.get("noise").cloned().unwrap_or_default();
        entity.map = entity.spawn.values.get("map").cloned().unwrap_or_default();
        entity.volume = number_field(&entity.spawn, "volume", 0.0);
        entity.attenuation = number_field(&entity.spawn, "attenuation", 0.0);
        entity.random = number_field(&entity.spawn, "random", 0.0);
        entity.style = integer_field(&entity.spawn, "style", 0);
        let id = actor.id().clone();
        self.entities.insert(id.clone(), entity);
        self.bind_callbacks(&actor);
        self.host.register_entity(&actor);
        id
    }

    /// Bind an entity continuation for engine dispatch.
    fn bind_callbacks(&mut self, actor: &OwnedActor) {
        self.host.callbacks().bind(actor);
    }

    /// Run a callback inside the current-actor boundary (`invoke`).
    fn invoke(&mut self, actor: ActorId, callback: impl FnOnce(&mut Self)) {
        let previous = self.current_actor.replace(actor);
        callback(self);
        self.current_actor = previous;
    }

    /// Scope `level.current_entity` around a callback (`runActor`).
    pub fn run_actor(&mut self, actor: ActorId, callback: impl FnOnce(&mut Self)) {
        self.invoke(actor, callback);
    }

    /// Dispatch an engine think event to a bound continuation.
    pub fn dispatch_think(&mut self, actor: ActorId) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.think,
            None => return,
        };
        if let Some(entity) = self.entity_mut(&actor) {
            entity.next_think = None;
        }
        self.invoke(actor.clone(), |game| {
            if let Some(callback) = callback {
                callback(actor, game);
            }
        });
    }

    /// Dispatch an engine use event to a bound continuation.
    pub fn dispatch_use(&mut self, actor: ActorId, other: Option<ActorId>, activator: Option<ActorId>) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.use_,
            None => return,
        };
        if let Some(callback) = callback {
            callback(actor, self, other, activator);
        }
    }

    /// Dispatch an engine touch event to a bound continuation.
    pub fn dispatch_touch(&mut self, actor: ActorId, contact: TouchContact) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.touch,
            None => return,
        };
        if let Some(callback) = callback {
            callback(actor, self, contact);
        }
    }

    /// Dispatch an engine pain event to a bound continuation.
    pub fn dispatch_pain(&mut self, actor: ActorId, reaction: PainReaction) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.pain,
            None => return,
        };
        if let Some(callback) = callback {
            callback(actor, self, reaction);
        }
    }

    /// Dispatch an engine death event to a bound continuation.
    pub fn dispatch_die(&mut self, actor: ActorId, reaction: DeathReaction) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.die,
            None => return,
        };
        if let Some(callback) = callback {
            callback(actor, self, reaction);
        }
    }

    /// Step weapon-behavior projectiles (`applyProjectileBehavior`).
    pub fn apply_projectile_behavior(&mut self, actor: ActorId, seconds: f64) {
        if self.entity(&actor).is_none() || self.host.bodies().attachment(&actor).is_some() {
            return;
        }
        let owned = self.owned_of(actor.clone());
        let body = self.body_of(actor.clone());
        let update = self.host.weapon_behavior().as_mut().and_then(|port| port.step(&owned, &body, seconds));
        if let Some(update) = update {
            self.project_trajectory(actor, &update);
        }
    }

    /// Project a trajectory (`projectTrajectory`).
    pub fn project_trajectory(&mut self, actor: ActorId, update: &WeaponTrajectoryUpdate) {
        let motion = self.require_entity(&actor).motion;
        self.source_callbacks.clone().project_trajectory(actor.clone(), self, update);
        let mut body = self.body_of(actor.clone());
        body.origin = update.origin;
        body.velocity = update.velocity;
        body.angles = update.angles;
        self.write_body(actor.clone(), &body, true);
        self.set_motion_kind(actor, motion);
    }

    /// Run pre-physics (`prePhysics`).
    pub fn pre_physics(&mut self, actor: ActorId) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.prethink,
            None => return,
        };
        self.invoke(actor.clone(), |game| {
            if let Some(callback) = callback {
                callback(actor, game);
            }
        });
    }

    /// Run post-physics (`postPhysics`).
    pub fn post_physics(&mut self, actor: ActorId) {
        let callback = match self.entity(&actor) {
            Some(entity) => entity.postthink,
            None => return,
        };
        self.invoke(actor.clone(), |game| {
            if let Some(callback) = callback {
                callback(actor, game);
            }
        });
    }

    /// Release an entity continuation (`remove`).
    pub fn remove_actor(&mut self, actor: ActorId) {
        let owned = match self.entities.get(&actor) {
            Some(entity) => entity.actor.clone(),
            None => return,
        };
        if !self.host.actors().is_live(owned.id()) {
            return;
        }
        self.host.bodies().unlink(&owned);
        let source = self.host.actors().source_of(owned.id());
        let classname = self.require_entity(&actor).classname.clone();
        let retained = classname == "bodyque"
            || source.as_ref().is_some_and(|(provider, slot)| {
                provider == &self.options.provider && *slot <= self.options.max_clients
            });
        if retained {
            return;
        }
        self.cancel_actor(actor.clone());
        self.host.emit(Q2PresentationEvent::Visibility {
            actor: actor.clone(),
            visible: false,
        });
        self.host.actors().release(&owned);
        self.on_actor_released(&owned);
    }

    /// Clean arena state after any actor release.
    ///
    /// Game-initiated releases call this directly; the engine calls it
    /// after every foreign release (see the table boundary docs).
    pub fn on_actor_released(&mut self, actor: &OwnedActor) {
        let id = actor.id();
        self.unsupported.remove(id);
        if let Some(slot) = self.source_slots.get(id).copied() {
            if slot > self.options.max_clients {
                let now = self.host.now();
                self.freed_slots.insert(slot, now);
            }
        }
        self.source_slots.remove(id);
        self.entities.remove(id);
        self.authored_targets.remove(id);
        self.monsters.on_actor_released(id);
        self.weapons.on_actor_released(id);
    }

    /// Resolve a weapon target (`weaponTarget`).
    pub fn weapon_target(&mut self, actor: &ActorId) -> Option<Q2WeaponTarget> {
        if !self.host.actors().is_live(actor) {
            return None;
        }
        if let Some(overridden) = self.host.weapon_target_override(actor) {
            return overridden;
        }
        self.entity(actor).map(|entity| Q2WeaponTarget {
            solid: entity.solid,
            laser_immune: entity.laser_immune,
            damageable_target: entity.damageable_target,
            bfg_explobox: entity.classname == "misc_explobox",
        })
    }

    /// Resolve a monster target observation (`monsterTarget`).
    pub fn monster_target(&mut self, actor: Option<&ActorId>) -> Option<MonsterTargetObservation> {
        let actor = actor?;
        if let Some(overridden) = self.host.monster_target_override(actor) {
            return overridden;
        }
        self.entity(actor).map(|entity| {
            let extra = if self.options.edition == Q2Edition::Rerelease { 0x1008000 } else { 0 };
            MonsterTargetObservation {
                view_height: f64::from(entity.view_height),
                notarget: entity.flags & (32 | extra) != 0,
                invisible: false,
                light_level: Some(f64::from(entity.light_level)),
                hostile_until: None,
            }
        })
    }

    /// Set solidity (`solid`).
    pub fn set_solid(&mut self, actor: ActorId, solid: Q2Solid) {
        {
            let entity = self.require_entity_mut(&actor);
            entity.solid = solid;
        }
        let model = {
            let entity = self.require_entity(&actor);
            entity
                .model
                .strip_prefix('*')
                .and_then(|number| number.parse::<i32>().ok())
        };
        if let Some(model) = model {
            let bounds = self.host.inline_model_bounds(model);
            let mut body = self.body_of(actor.clone());
            body.bounds = bounds;
            self.write_body(actor.clone(), &body, false);
        }
        let owned = self.owned_of(actor.clone());
        self.host.set_solid(&owned, solid, model);
        self.link_actor(actor);
    }

    /// Set motion (`motion`).
    pub fn set_motion_kind(&mut self, actor: ActorId, kind: Q2MotionKind) {
        let owned = self.owned_of(actor.clone());
        let body = self.body_of(actor.clone());
        let entity = self.require_entity_mut(&actor);
        entity.motion = kind;
        let motion = Q2Motion {
            actor: owned,
            kind,
            velocity: body.velocity,
            angular_velocity: entity.angular_velocity,
            gravity: entity.gravity,
            gravity_vector: entity.gravity_vector,
            clip_mask: entity.clip_mask,
            owner: entity.owner.clone(),
        };
        self.host.set_motion(&motion);
    }

    /// Schedule a think callback (`schedule`).
    pub fn schedule(&mut self, actor: ActorId, delay_seconds: f64, think: Q2Think) {
        let owned = self.owned_of(actor.clone());
        let next = if self.options.edition == Q2Edition::Rerelease {
            (js_round(self.host.now() * 1000.0) + js_round(delay_seconds * 1000.0)) / 1000.0
        } else {
            self.host.now() + delay_seconds
        };
        {
            let entity = self.require_entity_mut(&actor);
            entity.think = Some(think);
            entity.next_think = Some(next);
        }
        self.host.schedule(&owned, Some(next));
    }

    /// Cancel a scheduled think callback (`cancel`).
    pub fn cancel_actor(&mut self, actor: ActorId) {
        let owned = match self.entities.get(&actor) {
            Some(entity) => entity.actor.clone(),
            None => return,
        };
        if let Some(entity) = self.entity_mut(&actor) {
            entity.next_think = None;
            entity.think = None;
        }
        self.host.schedule(&owned, None);
    }

    /// Entities named by targetname in source order (`targets`).
    pub fn targets(&mut self, name: &str) -> Vec<ActorId> {
        if name.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<ActorId> = self
            .entities
            .iter()
            .filter(|(_, entity)| entity.targetname == name)
            .map(|(actor, _)| actor.clone())
            .collect();
        found.sort_by_key(|actor| {
            self.host
                .actors()
                .source_of(actor)
                .map_or(actor.slot(), |(_, slot)| slot)
        });
        found
    }

    /// Pick a random target (`pickTarget`).
    pub fn pick_target(&mut self, name: &str) -> Option<ActorId> {
        // G_PickTarget stores at most eight matches before selecting one.
        let targets = self.targets(name);
        let targets = &targets[..targets.len().min(8)];
        if targets.is_empty() {
            return None;
        }
        let pick = (self.host.random() * targets.len() as f64).floor() as usize;
        targets.get(pick).cloned()
    }

    /// Named target actors including foreign authored targets.
    fn target_actors(&mut self, name: &str) -> Vec<AuthoredTarget> {
        if name.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<AuthoredTarget> = self
            .entities
            .values()
            .map(AuthoredTarget::from)
            .chain(self.authored_targets.values().cloned())
            .filter(|target| target.targetname == name)
            .collect();
        found.retain(|target| self.host.actors().is_live(target.actor.id()));
        found.sort_by_key(|target| {
            self.host
                .actors()
                .source_of(target.actor.id())
                .map_or(target.actor.id().slot(), |(_, slot)| slot)
        });
        found
    }

    /// Use an entity's targets (`useTargets`).
    pub fn use_targets(&mut self, entity: &AuthoredTarget, activator: Option<&ActorId>, ignore_delay: bool) {
        if entity.delay != 0.0 && !ignore_delay {
            let delayed = self.create("DelayedUse", BTreeMap::new());
            {
                let target = self.require_entity_mut(&delayed);
                target.activator = activator.cloned();
                target.message.clone_from(&entity.message);
                target.target.clone_from(&entity.target);
                target.killtarget.clone_from(&entity.killtarget);
            }
            self.schedule(delayed, entity.delay, delayed_use as Q2Think);
            return;
        }
        if !entity.message.is_empty() {
            if let Some(activator) = activator {
                let is_monster = self.host.is_monster(activator);
                if !is_monster {
                    self.host.emit(Q2PresentationEvent::CenterPrint {
                        actor: activator.clone(),
                        text: entity.message.clone(),
                        instant: false,
                        duration_seconds: None,
                    });
                    if let Some(body) = self.host.bodies().read(activator) {
                        self.host.emit(Q2PresentationEvent::Sound(crate::q2::foundation::host::Q2SoundEvent {
                            actor: Some(activator.clone()),
                            origin: body.origin,
                            path: "misc/talk1.wav".to_string(),
                            channel: 0,
                            volume: 1.0,
                            attenuation: 1.0,
                            reliable: false,
                            loop_: crate::q2::foundation::host::Q2SoundLoop::Once,
                            loop_owner: None,
                        }));
                    }
                }
            }
        }
        for target in self.target_actors(&entity.killtarget) {
            let actor = target.actor.id().clone();
            if self.entity(&actor).is_none() {
                if self.host.actors().is_live(&actor) {
                    let owned = self.host.actors().resolve_owned(&actor);
                    if let Some(owned) = owned {
                        self.host.actors().release(&owned);
                    }
                }
            } else {
                self.remove_actor(actor);
            }
            if !self.host.actors().is_live(entity.actor.id()) {
                return;
            }
        }
        // Resolve each next slot after callbacks: nested spawns and removals
        // retain G_Find traversal behavior.
        let mut after: i64 = -1;
        loop {
            // Target actors arrive in source order; take the first slot
            // past the previous one.
            let mut next: Option<AuthoredTarget> = None;
            for candidate in self.target_actors(&entity.target) {
                let slot = self
                    .host
                    .actors()
                    .source_of(candidate.actor.id())
                    .map_or(candidate.actor.id().slot(), |(_, slot)| slot);
                if i64::from(slot) > after {
                    next = Some(candidate);
                    break;
                }
            }
            let Some(target) = next else {
                break;
            };
            after = i64::from(
                self.host
                    .actors()
                    .source_of(target.actor.id())
                    .map_or(target.actor.id().slot(), |(_, slot)| slot),
            );
            if target.actor.id() == entity.actor.id() {
                self.host.diagnostic(&format!("Q2 {} targets itself", entity.classname));
            } else if !(target.classname == "func_areaportal"
                && (entity.classname == "func_door" || entity.classname == "func_door_rotating"))
            {
                // The donor routes every use through the engine callback
                // table, which runs the bound continuation. Bound Q2
                // continuations are exactly the arena entities; foreign
                // targets run through engine dispatch.
                let id = target.actor.id().clone();
                if self.entities.contains_key(&id) {
                    self.dispatch_use(id, Some(entity.actor.id().clone()), activator.cloned());
                } else if let Some(owned) = self.host.actors().resolve_owned(&id) {
                    self.host.callbacks().forward_use(&owned, Some(entity.actor.id()), activator);
                }
            }
            if !self.host.actors().is_live(entity.actor.id()) {
                break;
            }
        }
    }

    /// Present an entity (`show`).
    pub fn show(&mut self, actor: ActorId) {
        let entity = self.require_entity(&actor).clone();
        self.host.emit(Q2PresentationEvent::Model(crate::q2::foundation::host::Q2ModelEvent {
            actor: actor.clone(),
            path: entity.model.clone(),
            attached_models: vec![entity.model2.clone(), entity.model3.clone(), entity.model4.clone()],
            frame: entity.frame,
            old_frame: entity.old_frame,
            scale: entity.scale,
            alpha: entity.alpha,
            skin: entity.skin,
            effects: entity.effects,
            render_flags: entity.render_flags,
        }));
        self.host.emit(Q2PresentationEvent::Visibility {
            actor,
            visible: entity.visible,
        });
    }

    /// Emit a sound from an entity (`sound`).
    pub fn sound(&mut self, actor: &ActorId, path: &str, channel: i32, volume: f64, attenuation: f64) {
        let origin = self.body_of(actor.clone()).origin;
        self.host.emit(Q2PresentationEvent::Sound(crate::q2::foundation::host::Q2SoundEvent {
            actor: Some(actor.clone()),
            origin,
            path: path.to_string(),
            channel,
            volume,
            attenuation,
            reliable: false,
            loop_: crate::q2::foundation::host::Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }

    /// Build attack provenance (`attack`).
    pub fn attack(
        &mut self,
        inflictor: ActorId,
        attacker: Option<ActorId>,
        means_of_death: i32,
        flags: i32,
        weapon: Option<ItemId>,
    ) -> AttackProvenance {
        let sequence = self.sequence;
        self.sequence += 1;
        let now = self.host.now();
        AttackProvenance {
            sequence,
            time: SourceTime::Seconds(now as f32),
            attacker,
            inflictor: Some(inflictor),
            originating_projectile: None,
            weapon,
            weapon_provider: self.options.provider.clone(),
            damage_powerup_owner: None,
            combat_provider: self.options.combat_provider.clone(),
            inventory_provider: self.options.inventory_provider.clone(),
            movement_provider: self.options.movement_provider.clone(),
            cause: AttackCause::Q2 {
                means_of_death,
                damage_flags: flags,
                native: None,
            },
        }
    }

    /// Apply direct damage (`damage`).
    #[allow(clippy::too_many_arguments)]
    pub fn damage(
        &mut self,
        target: ActorId,
        inflictor: ActorId,
        attacker: Option<ActorId>,
        amount: f64,
        knockback: f64,
        direction: Vec3,
        point: Vec3,
        normal: Vec3,
        means_of_death: i32,
        flags: i32,
        weapon: Option<ItemId>,
    ) -> DamageOutcome {
        let attack = self.attack(inflictor, attacker, means_of_death, flags, weapon);
        let modifier = self.options.source_damage_modifier.clone();
        let request = DamageRequest {
            attack,
            target,
            amount,
            knockback,
            direction,
            point,
            normal,
            delivery: DamageDelivery::Direct,
        };
        let modified = apply_source_damage_modifier(request, modifier.as_ref(), &mut |actor| {
            self.host.actors().is_live(actor)
        });
        self.host.combat().apply(&modified)
    }

    /// Whether an inflictor can damage a target (`canDamage`).
    pub fn can_damage(&mut self, target: &ActorId, inflictor: &ActorId) -> bool {
        let body = match self.host.bodies().read(target) {
            Some(body) => body,
            None => return false,
        };
        let source = self.body_of(inflictor.clone()).origin;
        let entity = self.entity(target).cloned();
        let destination = if entity.as_ref().is_some_and(|entity| entity.motion == Q2MotionKind::Push) {
            add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5))
        } else {
            body.origin
        };
        let trace = self.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: source,
            end: destination,
            bounds: None,
            ignore: Some(inflictor.clone()),
            mask: MASK_SOLID,
            exclude: Vec::new(),
        });
        if trace.fraction == 1.0 || matches!(&trace.hit, TraceHit::Actor { actor } if actor == target) {
            return true;
        }
        if entity.as_ref().is_some_and(|entity| entity.motion == Q2MotionKind::Push) {
            return false;
        }
        for offset in [
            Vec3 { x: 15.0, y: 15.0, z: 0.0 },
            Vec3 { x: 15.0, y: -15.0, z: 0.0 },
            Vec3 { x: -15.0, y: 15.0, z: 0.0 },
            Vec3 { x: -15.0, y: -15.0, z: 0.0 },
        ] {
            let probe = self.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
                start: source,
                end: add3(destination, offset),
                bounds: None,
                ignore: Some(inflictor.clone()),
                mask: MASK_SOLID,
                exclude: Vec::new(),
            });
            if probe.fraction == 1.0 {
                return true;
            }
        }
        false
    }

    /// Apply radius damage (`radiusDamage`).
    #[allow(clippy::too_many_arguments)]
    pub fn radius_damage(
        &mut self,
        inflictor: ActorId,
        attacker: Option<ActorId>,
        damage: f64,
        ignore: Option<ActorId>,
        radius: f64,
        means_of_death: i32,
        damage_flags: i32,
        weapon: Option<ItemId>,
    ) {
        let origin = self.body_of(inflictor.clone()).origin;
        for target in self.host.nearby(origin, radius) {
            if ignore.as_ref().is_some_and(|ignored| ignored == &target) {
                continue;
            }
            let state = self.host.combat().read(&target);
            let body = self.host.bodies().read(&target);
            let (Some(state), Some(body)) = (state, body) else {
                continue;
            };
            if !state.can_take_damage {
                continue;
            }
            let center = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
            let delta = sub3(center, origin);
            let mut points = damage - 0.5 * f64::from(qa_core::math::length3(delta));
            if attacker.as_ref().is_some_and(|attacker| attacker == &target) {
                points *= 0.5;
            }
            if points <= 0.0 || !self.can_damage(&target, &inflictor) {
                continue;
            }
            let attack = self.attack(inflictor.clone(), attacker.clone(), means_of_death, damage_flags | 1, weapon.clone());
            let modifier = self.options.source_damage_modifier.clone();
            let request = DamageRequest {
                attack,
                target: target.clone(),
                amount: points.trunc(),
                knockback: points.trunc(),
                direction: sub3(body.origin, origin),
                point: origin,
                normal: ZERO,
                delivery: DamageDelivery::Radius,
            };
            let modified = apply_source_damage_modifier(request, modifier.as_ref(), &mut |actor| {
                self.host.actors().is_live(actor)
            });
            self.host.combat().apply(&modified);
        }
    }

    /// Create a combat record for an actor.
    pub fn create_combat(&mut self, actor: &OwnedActor, health: f64, mass: f64, can_take_damage: bool) {
        self.host.combat().create(
            actor,
            &CombatState {
                health,
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass,
                can_take_damage,
                invulnerable: false,
                no_knockback: false,
                team: None,
            },
        );
    }

    /// Set combat traits for an actor.
    pub fn set_combat_traits(&mut self, actor: &OwnedActor, changes: &CombatTraitChanges) {
        self.host.combat().set_traits(actor, changes);
    }
}

/// JavaScript `Math.round` (ties round toward positive infinity).
pub fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Authored target view over arena entities.
impl From<&Q2Entity> for AuthoredTarget {
    fn from(entity: &Q2Entity) -> Self {
        entity.authored_target()
    }
}
