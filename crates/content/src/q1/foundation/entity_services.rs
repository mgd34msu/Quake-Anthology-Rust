//! Q1 entity services (`src/content/q1/foundation/entity-services.ts`).
//!
//! Q1 deathmatch, skill, items, effect, and movement behavior adapted
//! from id Software Quake / Quake rerelease QuakeC.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.
//!
//! Entities live in the services object and callbacks dispatch by
//! registered name plus subject actor id; see
//! [`super::entity`] for the ownership rationale. Insertion-order
//! vectors preserve donor `Map` iteration order.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{same_actor, ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{Arithmetic, DonorSource, NumericOps};
use qa_core::time::SourceTime;

use crate::bsp::Q1Entity;
use crate::contract::{
    ArmorState, InventoryEntry, ItemId, PoweredProtectionState, RegularArmorState, SourceWeaponHandoff,
};
use crate::monsters::{AuthoredTarget, MonsterTargetObservation};

use super::callbacks::{Q1CallbackHandlers, Q1CallbackRegistry, Q1CallbackSlot, Q1StateExtension};
use super::checkpoint::{capture_foundation, restore_foundation, Q1FoundationCheckpoint};
use super::entity::{source_angles, Q1Actor, Q1Move};
use super::extensions::{
    Q1PickupRules, Q1PlayerExtension, Q1PlayerHook, Q1PlayerItemHook, Q1PlayerSecondsHook, Q1WeaponDefinition,
    Q1WeaponRules,
};
use super::gameplay::{
    apply_source_damage_modifier, q1_water_transition, AttackCause, BodyPatch, BodyState, CombatState, CombatTraits,
    DamageDelivery, DamageOutcome, DamagePreparation, DamageRequest, DeathReaction, PainReaction, Q1CombatArithmetic,
    Q1CombatContext, Q1DamageSourceEffects, Q1LethalHealth, Q1LethalReaction, Q1ThinkFrame, TouchContact,
    TransitionIntent,
};
use super::host::{
    DamageAdjust, Q1Contents, Q1CutsceneControl, Q1FoundationHost, Q1ReleaseHook, Q1SourceTarget, Q1TrajectoryUpdate,
    Q1WeaponBehaviorLaunch,
};
use super::precache::Q1PrecacheRegistry;
use super::shambler_damage::foreign_shambler_damage;
use super::types::{
    dot, length, vadd, vectors, vscale, vsub, Q1AutoSwitch, Q1Basis, Q1Effect, Q1Event, Q1FoundationOptions,
    Q1MessageArg, Q1MoveType, Q1Powerup, Q1Presentation, Q1Solid, Q1SoundChannel, Q1TraceRequest, Q1Weapon, POINT,
    WEAPONS, ZERO,
};
use crate::q1::{q1_error, q1_range, Q1Error};

/// Pending intermission.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Intermission {
    /// Destination map.
    pub map: String,
    /// Travel cause.
    pub cause: Option<ActorId>,
    /// Exit availability in seconds.
    pub exit_after: f64,
}

/// Patrol-path advance hook.
pub type Q1PathAdvance = Box<dyn FnMut(&mut Q1EntityServices, &str, Option<ActorId>, f64) -> Result<(), Q1Error>>;
/// Patrol-path follower factory.
pub type Q1PathFollowerFactory = Box<dyn FnMut(&ActorId) -> Option<Q1PathFollower>>;

/// Authored patrol-path follower result.
pub struct Q1PathFollower {
    /// Path target name.
    pub targetname: String,
    /// Current enemy.
    pub enemy: Option<ActorId>,
    /// Path advance hook.
    pub advance: Q1PathAdvance,
}

impl std::fmt::Debug for Q1PathFollower {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Q1PathFollower").finish_non_exhaustive()
    }
}

/// Weapon availability purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1WeaponPurpose {
    /// Best-weapon selection.
    Best,
    /// Firing.
    Fire,
}

/// Player input sample (`playerInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1PlayerInput {
    /// Attack held.
    pub attack: bool,
    /// Jump held.
    pub jump: bool,
    /// Teleport control lock expiry in seconds.
    pub teleport_until: Option<f64>,
}

/// Player admission options (`attachPlayer`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1AttachOptions {
    /// Starting weapon.
    pub weapon: Option<Q1Weapon>,
    /// Whether to initialize the weapon inventory.
    pub initialize_inventory: bool,
    /// Maximum health override.
    pub max_health: Option<f64>,
}

impl Default for Q1AttachOptions {
    fn default() -> Self {
        Self {
            weapon: None,
            initialize_inventory: true,
            max_health: None,
        }
    }
}

/// Damage call parameters (`damage` optional arguments).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1DamageParams {
    /// Attacking weapon.
    pub weapon: Option<Q1Weapon>,
    /// Damage delivery.
    pub delivery: DamageDelivery,
    /// Death type text.
    pub death_type: String,
    /// Armor interaction override.
    pub armor_effect: Option<super::gameplay::Q1ArmorEffect>,
}

/// Named spawn handler.
pub type Q1SpawnHandler = fn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error>;

/// Named path-touch handler.
pub type Q1PathTouchHandler =
    fn(game: &mut Q1EntityServices, corner: &ActorId, mover: &ActorId) -> Result<bool, Q1Error>;

/// Q1 entity services (`Q1EntityServices`).
pub struct Q1EntityServices {
    /// Engine host.
    pub host: Q1FoundationHost,
    /// Named callback registry.
    pub named: Q1CallbackRegistry,
    /// Precache registry.
    pub precaches: Q1PrecacheRegistry,
    base_team_health: bool,
    path_touches: Vec<(String, Q1PathTouchHandler)>,
    source_damage_effects: Vec<(String, Q1DamageSourceEffects)>,
    /// Active pickup rules.
    pub pickup_rules: Option<Q1PickupRules>,
    /// Source pickup admission, when the session resolves supply grants.
    pub pickup_admission: Option<Box<dyn crate::contract::PickupAdmission>>,
    weapon_rules: Vec<(String, Q1WeaponRules)>,
    /// Registered weapons.
    pub registered_weapons: HashMap<Q1Weapon, Q1WeaponDefinition>,
    player_extensions: Vec<(String, Q1PlayerExtension)>,
    /// Forced weapon order.
    pub weapon_order: Option<Vec<Q1Weapon>>,
    weapon_order_owner: Option<String>,
    /// Registered state extensions.
    pub state_extensions: HashMap<String, Box<dyn Q1StateExtension>>,
    state_extension_order: Vec<String>,
    /// Live source entities.
    pub entities: HashMap<ActorId, Q1Actor>,
    entity_order: Vec<ActorId>,
    /// Authored path targets.
    pub authored_targets: HashMap<ActorId, AuthoredTarget>,
    /// Authored monster missions.
    pub monster_missions: HashMap<ActorId, Box<dyn super::gameplay::Q1MonsterMission>>,
    /// Authored patrol-path follower factory.
    pub authored_path_follower: Option<Q1PathFollowerFactory>,
    /// Player arsenal states.
    pub players: HashMap<ActorId, super::types::Q1PlayerState>,
    player_order: Vec<ActorId>,
    release_hooks: Vec<Rc<RefCell<dyn Q1ReleaseHook>>>,
    /// Source time in seconds.
    pub time: f64,
    /// Frame elapsed seconds.
    pub frame_seconds: f64,
    /// Pending forced retouches.
    pub force_retouch: i32,
    /// Saved QC basis.
    pub basis: Q1Basis,
    /// Total secrets.
    pub total_secrets: i32,
    /// Secrets found.
    pub found_secrets: i32,
    /// Total monsters.
    pub total_monsters: i32,
    /// Monsters killed.
    pub killed_monsters: i32,
    /// World type.
    pub world_type: i32,
    /// Map name.
    pub map_name: String,
    /// World actor.
    pub world: Option<ActorId>,
    /// Shared sight entity.
    pub sight_entity: Option<ActorId>,
    /// Sight time in seconds.
    pub sight_time: f64,
    /// Pending intermission.
    pub intermission: Option<Q1Intermission>,
    configured_options: Q1FoundationOptions,
    spawn_options: Option<Q1FoundationOptions>,
    sequence: u64,
    pub(crate) next_dynamic_slot: u32,
    spawners: HashMap<String, Q1SpawnHandler>,
    /// Monster admission overrides.
    pub monster_admission: Option<Box<dyn super::runtime::Q1MonsterAdmission>>,
    /// Threewave grapple service state.
    pub threewave_grapple: Option<super::super::equipment::grapple::ThreewaveGrappleService>,
    /// Threewave weapon hooks.
    pub threewave_weapon: Option<super::super::equipment::weapon::ThreewaveWeaponService>,
}

impl std::fmt::Debug for Q1EntityServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Q1EntityServices").finish_non_exhaustive()
    }
}

fn sub_remove(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.remove(id)
}

fn sub_null(_game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    Ok(())
}

fn delay_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let activator = game.entity_ref(&id).and_then(|entity| entity.activator.clone());
    game.update_entity(&id, |entity| entity.targetname.clear())?;
    game.use_targets(&id, activator.as_ref())?;
    game.remove(&id)
}

impl Q1EntityServices {
    /// Fresh services bound to a host.
    pub fn new(host: Q1FoundationHost, options: Q1FoundationOptions) -> Result<Self, Q1Error> {
        let mut game = Self {
            host,
            named: Q1CallbackRegistry::new(),
            precaches: Q1PrecacheRegistry::new(),
            base_team_health: false,
            path_touches: Vec::new(),
            source_damage_effects: Vec::new(),
            pickup_rules: None,
            pickup_admission: None,
            weapon_rules: Vec::new(),
            registered_weapons: HashMap::new(),
            player_extensions: Vec::new(),
            weapon_order: None,
            weapon_order_owner: None,
            state_extensions: HashMap::new(),
            state_extension_order: Vec::new(),
            entities: HashMap::new(),
            entity_order: Vec::new(),
            authored_targets: HashMap::new(),
            monster_missions: HashMap::new(),
            authored_path_follower: None,
            players: HashMap::new(),
            player_order: Vec::new(),
            release_hooks: Vec::new(),
            time: 0.0,
            frame_seconds: 0.1,
            force_retouch: 2,
            basis: Q1Basis::default(),
            total_secrets: 0,
            found_secrets: 0,
            total_monsters: 0,
            killed_monsters: 0,
            world_type: 0,
            map_name: String::new(),
            world: None,
            sight_entity: None,
            sight_time: 0.0,
            intermission: None,
            configured_options: options,
            spawn_options: None,
            sequence: 0,
            next_dynamic_slot: 1,
            spawners: HashMap::new(),
            monster_admission: None,
            threewave_grapple: None,
            threewave_weapon: None,
        };
        game.named.register(
            "SUB_Remove",
            Q1CallbackHandlers {
                action: Some(sub_remove),
                ..Default::default()
            },
        )?;
        game.named.register(
            "SUB_Null",
            Q1CallbackHandlers {
                action: Some(sub_null),
                ..Default::default()
            },
        )?;
        game.named.register(
            "DelayThink",
            Q1CallbackHandlers {
                action: Some(delay_think),
                ..Default::default()
            },
        )?;
        super::movers::register_mover_callbacks(&mut game)?;
        super::spawns::register_spawn_callbacks(&mut game)?;
        super::pickups::register_pickup_callbacks(&mut game)?;
        super::weapons::register_weapon_callbacks(&mut game)?;
        super::monsters::register_monster_callbacks(&mut game)?;
        Ok(game)
    }

    /// Operating options (spawn overrides active during spawn).
    #[must_use]
    pub fn options(&self) -> &Q1FoundationOptions {
        self.spawn_options.as_ref().unwrap_or(&self.configured_options)
    }

    /// Operating provider.
    #[must_use]
    pub fn provider(&self) -> ProviderId {
        self.options()
            .provider
            .clone()
            .unwrap_or_else(super::types::q1_provider)
    }

    /// Whether the source program allows native precache calls.
    #[must_use]
    pub fn uses_id1_precaches(&self) -> bool {
        matches!(
            self.options().precache_program,
            Some(super::types::Q1PrecacheProgram::Id1)
        )
    }

    /// Declare a precached model.
    pub fn precache_model(&mut self, path: &str) -> Result<String, Q1Error> {
        self.precaches.model(path)
    }

    /// Declare a precached sound.
    pub fn precache_sound(&mut self, path: &str) -> Result<String, Q1Error> {
        self.precaches.sound(path)
    }

    /// Register source damage effects.
    pub fn register_damage_source_effects(&mut self, id: &str, effects: Q1DamageSourceEffects) -> Result<(), Q1Error> {
        if self.source_damage_effects.iter().any(|(candidate, _)| candidate == id) {
            return Err(q1_error(format!("Duplicate Q1 damage source effects: {id}")));
        }
        self.source_damage_effects.push((id.to_string(), effects));
        Ok(())
    }

    /// Run quad preparation stages.
    #[must_use]
    pub fn before_quad(
        &self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> DamagePreparation {
        for (_, effects) in &self.source_damage_effects {
            if let Some(before) = effects.before_quad.as_ref() {
                let preparation = before(request, damage, target, attacker);
                if !matches!(preparation, DamagePreparation::Continue { amount } if amount == damage) {
                    return preparation;
                }
            }
        }
        DamagePreparation::Continue { amount: damage }
    }

    /// Run post-quad preparation stages.
    #[must_use]
    pub fn after_quad(
        &self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> DamagePreparation {
        for (_, effects) in &self.source_damage_effects {
            if let Some(after) = effects.after_quad.as_ref() {
                let preparation = after(request, damage, target, attacker);
                if !matches!(preparation, DamagePreparation::Continue { amount } if amount == damage) {
                    return preparation;
                }
            }
        }
        DamagePreparation::Continue { amount: damage }
    }

    /// Run armor permission stages.
    #[must_use]
    pub fn armor_allowed(
        &self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> bool {
        for (_, effects) in &self.source_damage_effects {
            if let Some(allowed) = effects.armor_allowed.as_ref() {
                if !allowed(request, damage, target, attacker) {
                    return false;
                }
            }
        }
        true
    }

    /// Run protection permission stages.
    #[must_use]
    pub fn protection_applies(
        &self,
        request: &DamageRequest,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> bool {
        for (_, effects) in &self.source_damage_effects {
            if let Some(applies) = effects.protection_applies.as_ref() {
                if !applies(request, target, attacker) {
                    return false;
                }
            }
        }
        true
    }

    /// Run pre-health permission stages.
    #[must_use]
    pub fn before_health(
        &self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> bool {
        for (_, effects) in &self.source_damage_effects {
            if let Some(before) = effects.before_health.as_ref() {
                if !before(request, damage, target, attacker) {
                    return false;
                }
            }
        }
        true
    }

    /// Run post-armor scaling stages.
    #[must_use]
    pub fn after_armor(
        &self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> f64 {
        let mut current = damage;
        for (_, effects) in &self.source_damage_effects {
            if let Some(after) = effects.after_armor.as_ref() {
                current = after(request, current, target, attacker);
            }
        }
        current
    }

    /// Run lethal-health stages.
    #[must_use]
    pub fn lethal_health(
        &self,
        request: &DamageRequest,
        damage: f64,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> Q1LethalHealth {
        let mut current = Q1LethalHealth {
            health: target.health - damage,
            reaction: Q1LethalReaction::Death,
        };
        for (_, effects) in &self.source_damage_effects {
            if let Some(lethal) = effects.lethal_health.as_ref() {
                current = lethal(request, current.health, target, attacker);
            }
        }
        current
    }

    /// Set source gravity through the movement host.
    pub fn set_gravity(&mut self, actor: &ActorId, scale: f64) -> Result<(), Q1Error> {
        self.host.require_set_gravity(actor, scale)
    }

    /// Drive a player camera through the control host.
    pub fn control_player(&mut self, actor: &ActorId, control: &Q1CutsceneControl) -> Result<(), Q1Error> {
        self.host.require_control_player(actor, control)
    }

    /// Enable base team-health rules.
    pub fn set_base_team_health(&mut self) {
        self.base_team_health = true;
    }

    /// Register a path-touch handler.
    pub fn register_path_touch(&mut self, id: &str, handler: Q1PathTouchHandler) -> Result<(), Q1Error> {
        if self.path_touches.iter().any(|(candidate, _)| candidate == id) {
            return Err(q1_error(format!("Duplicate Q1 path touch: {id}")));
        }
        self.path_touches.push((id.to_string(), handler));
        Ok(())
    }

    /// Run path-touch handlers for a corner/mover pair.
    pub fn source_path_touch(&mut self, corner: &ActorId, mover: &ActorId) -> Result<bool, Q1Error> {
        let handlers: Vec<Q1PathTouchHandler> = self.path_touches.iter().map(|(_, handler)| *handler).collect();
        for handler in handlers {
            if handler(self, corner, mover)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Register weapon rules.
    pub fn register_weapon_rules(&mut self, rules: Q1WeaponRules) -> Result<(), Q1Error> {
        if self.weapon_rules.iter().any(|(candidate, _)| candidate == &rules.id) {
            return Err(q1_error(format!("Duplicate Q1 weapon rules: {}", rules.id)));
        }
        self.weapon_rules.push((rules.id.clone(), rules));
        Ok(())
    }

    /// Consume weapon ammunition through rules or the shared inventory.
    pub fn consume_weapon_ammo(&mut self, player: &ActorId, item: &ItemId, amount: f64) -> Result<bool, Q1Error> {
        let hook = self.weapon_rules.iter().find_map(|(_, rules)| rules.consume_ammo);
        if let Some(consume) = hook {
            return consume(self, player, item, amount);
        }
        let owned = self
            .player_owned(player)
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        Ok(self.host.inventory.consume(&owned, item, amount))
    }

    /// Run weapon pre-fire hooks.
    pub fn weapon_before_fire(&mut self, player: &ActorId) -> Result<(), Q1Error> {
        let hooks: Vec<Q1PlayerHook<Result<(), Q1Error>>> = self
            .weapon_rules
            .iter()
            .filter_map(|(_, rules)| rules.before_fire)
            .collect();
        for hook in hooks {
            hook(self, player)?;
        }
        Ok(())
    }

    /// Resolve the attack delay through rules.
    pub fn weapon_attack_delay(&mut self, player: &ActorId, delay: f64) -> Result<f64, Q1Error> {
        let hook = self.weapon_rules.iter().find_map(|(_, rules)| rules.attack_delay);
        match hook {
            Some(attack_delay) => attack_delay(self, player, delay),
            None => Ok(delay),
        }
    }

    /// Resolve the frame delay through rules.
    pub fn weapon_frame_delay(&mut self, player: &ActorId, delay: f64) -> Result<f64, Q1Error> {
        let hook = self.weapon_rules.iter().find_map(|(_, rules)| rules.frame_delay);
        match hook {
            Some(frame_delay) => frame_delay(self, player, delay),
            None => Ok(delay),
        }
    }

    /// Resolve the nail speed through rules.
    pub fn nail_speed(&mut self, player: &ActorId, speed: f64) -> Result<f64, Q1Error> {
        let hook = self.weapon_rules.iter().find_map(|(_, rules)| rules.nail_speed);
        match hook {
            Some(resolve) => resolve(self, player, speed),
            None => Ok(speed),
        }
    }

    /// Register pickup rules.
    pub fn register_pickup_rules(&mut self, rules: Q1PickupRules) -> Result<(), Q1Error> {
        if self.pickup_rules.as_ref().is_some_and(|current| current.id == rules.id) {
            return Err(q1_error(format!("Duplicate Q1 pickup rules: {}", rules.id)));
        }
        self.pickup_rules = Some(rules);
        Ok(())
    }

    /// Register a source weapon definition.
    pub fn register_weapon(&mut self, definition: Q1WeaponDefinition) -> Result<(), Q1Error> {
        if self.registered_weapons.contains_key(&definition.id) {
            return Err(q1_error(format!(
                "Duplicate Q1 weapon registration: {}",
                definition.id.as_str()
            )));
        }
        self.registered_weapons.insert(definition.id, definition);
        Ok(())
    }

    /// Replace a registered weapon definition.
    pub fn replace_weapon(&mut self, definition: Q1WeaponDefinition) -> Result<(), Q1Error> {
        if !self.registered_weapons.contains_key(&definition.id) {
            return Err(q1_error(format!(
                "Missing Q1 weapon registration: {}",
                definition.id.as_str()
            )));
        }
        self.registered_weapons.insert(definition.id, definition);
        Ok(())
    }

    /// Register a forced weapon order.
    pub fn register_weapon_order(&mut self, id: &str, order: Vec<Q1Weapon>) -> Result<(), Q1Error> {
        if self.weapon_order_owner.as_deref() == Some(id) {
            return Err(q1_error(format!("Duplicate Q1 weapon order: {id}")));
        }
        self.weapon_order_owner = Some(id.to_string());
        self.weapon_order = Some(order);
        Ok(())
    }

    /// Register a player behavior extension.
    pub fn register_player_extension(&mut self, extension: Q1PlayerExtension) -> Result<(), Q1Error> {
        if self
            .player_extensions
            .iter()
            .any(|(candidate, _)| candidate == &extension.id)
        {
            return Err(q1_error(format!("Duplicate Q1 player extension: {}", extension.id)));
        }
        self.player_extensions.push((extension.id.clone(), extension));
        Ok(())
    }

    /// Resolve the inventory capacity through extensions.
    pub fn inventory_capacity(&mut self, player: &ActorId, item: &ItemId) -> Result<Option<f64>, Q1Error> {
        let hooks: Vec<Q1PlayerItemHook<Result<Option<f64>, Q1Error>>> = self
            .player_extensions
            .iter()
            .filter_map(|(_, extension)| extension.inventory_capacity)
            .collect();
        let mut owners = 0;
        let mut capacity = None;
        for hook in hooks {
            if let Some(value) = hook(self, player, item)? {
                owners += 1;
                capacity = Some(value);
            }
        }
        if owners > 1 {
            return Err(q1_error(format!("Multiple Q1 inventory capacity owners for {item}")));
        }
        Ok(capacity)
    }

    /// Resolve the weapon view model for a player.
    pub fn weapon_model(&mut self, weapon: Q1Weapon, player: Option<&ActorId>) -> Result<String, Q1Error> {
        if let Some(definition) = self.registered_weapons.get(&weapon) {
            if let (Some(model_for), Some(player)) = (definition.model_for, player) {
                return model_for(self, player);
            }
            return Ok(definition.model.clone());
        }
        super::weapons::weapon_model(weapon)
    }

    /// Resolve the inventory item for a weapon.
    #[must_use]
    pub fn weapon_item(&self, weapon: Q1Weapon) -> ItemId {
        self.registered_weapons
            .get(&weapon)
            .and_then(|definition| definition.item.clone())
            .unwrap_or_else(|| super::types::weapon_item(weapon))
    }

    /// Resolve the ammunition item for a weapon.
    #[must_use]
    pub fn weapon_ammo(&self, weapon: Q1Weapon) -> Option<ItemId> {
        match self.registered_weapons.get(&weapon) {
            Some(definition) => definition.ammo.clone(),
            None => ammo_item(weapon),
        }
    }

    /// Whether a weapon is available to a player.
    pub fn weapon_available(
        &mut self,
        player: &ActorId,
        weapon: Q1Weapon,
        purpose: Q1WeaponPurpose,
        ammo_count: Option<&dyn Fn(&ItemId) -> f64>,
    ) -> Result<bool, Q1Error> {
        let states = self
            .player_ref(player)
            .cloned()
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        let hook = match self.registered_weapons.get(&weapon) {
            Some(definition) => match purpose {
                Q1WeaponPurpose::Best => definition.best_available.or(definition.available),
                Q1WeaponPurpose::Fire => definition.available,
            },
            None if super::types::is_q1_base_weapon(weapon) => None,
            None => return Err(q1_error("Player has no Q1 weapon state")),
        };
        if let Some(available) = hook {
            return available(self, player);
        }
        if states.weapon == weapon {
            return Ok(true);
        }
        let owned = self
            .host
            .actors
            .resolve_owned(player)
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        if self.host.inventory.count(owned.id(), &self.weapon_item(weapon)) <= 0.0 {
            return Ok(false);
        }
        let Some(ammo) = self.weapon_ammo(weapon) else {
            return Ok(true);
        };
        let count = match ammo_count {
            Some(count) => count(&ammo),
            None => self.host.inventory.count(owned.id(), &ammo),
        };
        Ok(count > 0.0)
    }

    /// Fire the player's weapon through its definition.
    pub fn fire_registered_weapon(&mut self, player: &ActorId) -> Result<bool, Q1Error> {
        let states = self
            .player_ref(player)
            .cloned()
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        let fire = self
            .registered_weapons
            .get(&states.weapon)
            .map(|definition| definition.fire)
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        fire(self, player)
    }

    /// Capture a source checkpoint.
    pub fn capture(&mut self) -> Result<Q1FoundationCheckpoint, Q1Error> {
        capture_foundation(self, self.sequence, self.next_dynamic_slot)
    }

    /// Restore source state into a fresh provider.
    pub fn restore(&mut self, checkpoint: &Q1FoundationCheckpoint, schedule_thinks: bool) -> Result<(), Q1Error> {
        restore_foundation(self, checkpoint)?;
        self.sequence = checkpoint.sequence;
        self.next_dynamic_slot = checkpoint.next_dynamic_slot;
        if schedule_thinks {
            self.resume_thinks();
        }
        Ok(())
    }

    /// Schedule restored thinks through the engine.
    pub fn resume_thinks(&mut self) {
        let ids = self.entity_ids();
        for id in ids {
            let think = self.entity_ref(&id).and_then(|entity| {
                if entity.movement == Q1MoveType::Push || entity.next_think < 0.0 {
                    None
                } else {
                    Some((entity.actor.clone(), entity.next_think))
                }
            });
            if let Some((actor, next_think)) = think {
                self.host.schedule_think(&actor, next_think);
            }
        }
    }

    /// Register a source state extension.
    pub fn register_state_extension(&mut self, extension: Box<dyn Q1StateExtension>) -> Result<(), Q1Error> {
        if self.state_extensions.contains_key(extension.id()) {
            return Err(q1_error(format!("Duplicate Q1 state extension: {}", extension.id())));
        }
        self.state_extension_order.push(extension.id().to_string());
        self.state_extensions.insert(extension.id().to_string(), extension);
        Ok(())
    }

    /// Register a named spawn handler.
    pub fn register_spawn(&mut self, classname: &str, spawn: Q1SpawnHandler) -> Result<(), Q1Error> {
        if self.spawners.contains_key(classname) {
            return Err(q1_error(format!("Q1 spawn handler already registered: {classname}")));
        }
        self.spawners.insert(classname.to_string(), spawn);
        Ok(())
    }

    /// Replace a named spawn handler.
    pub fn replace_spawn(&mut self, classname: &str, spawn: Q1SpawnHandler) -> Result<(), Q1Error> {
        if !self.spawners.contains_key(classname) {
            return Err(q1_error(format!("No registered Q1 spawn to replace: {classname}")));
        }
        self.spawners.insert(classname.to_string(), spawn);
        Ok(())
    }

    /// Create a source entity.
    pub fn create(
        &mut self,
        classname: &str,
        source: Option<&Q1Entity>,
        source_ordinal: Option<i32>,
    ) -> Result<ActorId, Q1Error> {
        let slot = match source_ordinal {
            None => {
                let slot = self.next_dynamic_slot;
                self.next_dynamic_slot += 1;
                slot
            }
            Some(0) => 0,
            Some(ordinal) => {
                let max_clients = self.options().max_clients.unwrap_or(0);
                u32::try_from(i64::from(ordinal) + i64::from(max_clients))
                    .map_err(|_| q1_error("Invalid Q1 source slot"))?
            }
        };
        let provider = self.provider();
        let owner = self
            .host
            .actors
            .allocate_at_source(&provider, slot, &format!("q1:{classname}"))?;
        let mut entity = Q1Actor::new(owner.clone(), classname, source_ordinal, source);
        entity.actor = owner.clone();
        let initial_origin = if source.is_some() {
            entity.vector("origin")
        } else {
            ZERO
        };
        let initial_angles = source.map(source_angles).unwrap_or(ZERO);
        self.host.bodies.create(
            &owner,
            &BodyState {
                origin: initial_origin,
                angles: initial_angles,
                velocity: ZERO,
                bounds: POINT,
                ground: None,
            },
        )?;
        self.host.combat.create(
            &owner,
            &CombatState {
                health: entity.max_health,
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass: 100.0,
                can_take_damage: false,
                invulnerable: false,
                no_knockback: None,
                team: None,
            },
        )?;
        let id = owner.id().clone();
        self.admit_entity(entity)?;
        Ok(id)
    }

    /// Admit an existing entity record.
    fn admit_entity(&mut self, entity: Q1Actor) -> Result<(), Q1Error> {
        self.host.actors.assert_owned(&entity.actor)?;
        if self.entities.contains_key(entity.actor.id()) {
            return Err(q1_error("Actor is already a Q1 entity"));
        }
        if self.host.bodies.read(entity.actor.id()).is_none() || self.host.combat.read(entity.actor.id()).is_none() {
            return Err(q1_error("Admit shared Q1 body and combat records before source actors"));
        }
        let id = entity.actor.id().clone();
        let species = entity.monster.as_ref().map(|monster| monster.species);
        self.entities.insert(id.clone(), entity);
        self.entity_order.push(id.clone());
        self.host.combat.bind_damage_adjustment(
            self.entities
                .get(&id)
                .map(|entity| entity.actor.clone())
                .as_ref()
                .expect("admitted"),
            Box::new(move |request: &DamageRequest| {
                foreign_shambler_damage(species, request).map(|adjusted| DamageAdjust {
                    amount: adjusted.amount,
                    knockback: adjusted.knockback,
                })
            }),
        );
        self.bind_actor_callbacks(&id)?;
        if let Some(mut register) = self.host.register_entity.take() {
            register.register(self, &id);
            self.host.register_entity = Some(register);
        }
        Ok(())
    }

    /// Attach an externally created actor as a source entity.
    pub fn attach_existing(
        &mut self,
        actor: OwnedActor,
        classname: &str,
        source: Option<&Q1Entity>,
        source_ordinal: Option<i32>,
    ) -> Result<ActorId, Q1Error> {
        let id = actor.id().clone();
        self.admit_entity(Q1Actor::new(actor, classname, source_ordinal, source))?;
        Ok(id)
    }

    /// Bind the standard actor callbacks through the host table.
    pub fn bind_actor_callbacks(&mut self, id: &ActorId) -> Result<(), Q1Error> {
        let actor = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        self.host.callbacks.bind(&actor);
        Ok(())
    }

    /// Duplicate an entity record without running a spawn function.
    pub fn clone_entity(&mut self, source: &ActorId) -> Result<ActorId, Q1Error> {
        let entity = self
            .entity_ref(source)
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let body = self.body(source)?;
        let target = self.create(&entity.classname, None, None)?;
        let state = super::checkpoint::capture_entity_source_state(&entity);
        let fields = entity.fields.clone();
        let references = entity.references.clone();
        let owner = entity.owner.clone();
        let activator = entity.activator.clone();
        let monster = entity.monster.clone();
        let move_completion = entity.move_completion.clone();
        let think = entity.think.clone().map(|name| self.named.action(&name)).transpose()?;
        let use_callback = entity
            .use_callback
            .clone()
            .map(|name| self.named.use_callback(&name))
            .transpose()?;
        let touch = entity.touch.clone().map(|name| self.named.touch(&name)).transpose()?;
        let pain = entity.pain.clone().map(|name| self.named.pain(&name)).transpose()?;
        let die = entity.die.clone().map(|name| self.named.die(&name)).transpose()?;
        let blocked = entity
            .blocked
            .clone()
            .map(|name| self.named.blocked(&name))
            .transpose()?;
        let path_end = entity
            .path_end
            .clone()
            .map(|name| self.named.action(&name))
            .transpose()?;
        let door_group = entity.door_group.clone();
        self.set_body(
            &target,
            &BodyPatch {
                origin: Some(body.origin),
                angles: Some(body.angles),
                velocity: Some(body.velocity),
                bounds: Some(body.bounds),
                ground: Some(body.ground.clone()),
            },
        )?;
        self.update_entity(&target, |entity| {
            super::checkpoint::apply_entity_source_state(entity, &state);
            entity.fields = fields;
            entity.references = references;
            entity.owner = owner;
            entity.activator = activator;
            entity.monster = monster;
            entity.move_completion = move_completion;
            entity.think = think;
            entity.use_callback = use_callback;
            entity.touch = touch;
            entity.pain = pain;
            entity.die = die;
            entity.blocked = blocked;
            entity.path_end = path_end;
            entity.door_group = door_group;
        })?;
        let extensions: Vec<String> = self.state_extension_order.clone();
        for id in extensions {
            let mut extension = self.state_extensions.remove(&id).expect("registered extension");
            extension.clone_state(self, source, &target)?;
            self.state_extensions.insert(id, extension);
        }
        self.bind_actor_callbacks(&target)?;
        let owned = self
            .entity_ref(&target)
            .map(|entity| entity.actor.clone())
            .expect("cloned");
        let combat = self.host.combat.read(owned.id());
        if let Some(mut combat) = combat {
            if let Some(source_combat) = self.host.combat.read(source) {
                combat = source_combat;
            }
            self.host.combat.set_health(&owned, combat.health)?;
            self.host.combat.set_armor(&owned, &combat.armor)?;
            self.host.combat.set_traits(
                &owned,
                CombatTraits {
                    can_take_damage: combat.can_take_damage,
                    mass: combat.mass,
                    invulnerable: combat.invulnerable,
                    team: combat.team.clone(),
                    no_knockback: combat.no_knockback,
                },
            )?;
        }
        Ok(target)
    }

    /// Read an entity record.
    #[must_use]
    pub fn entity_ref(&self, id: &ActorId) -> Option<&Q1Actor> {
        self.entities.get(id)
    }

    /// Read an entity record (donor `game.entity`).
    #[must_use]
    pub fn entity(&self, id: &ActorId) -> Option<&Q1Actor> {
        self.entity_ref(id)
    }

    /// Read a player state.
    #[must_use]
    pub fn player_ref(&self, id: &ActorId) -> Option<&super::types::Q1PlayerState> {
        let owned = self.host.actors.resolve_owned(id)?;
        self.players.get(owned.id())
    }

    /// Resolve a player's owned handle.
    #[must_use]
    pub fn player_owned(&self, id: &ActorId) -> Option<OwnedActor> {
        let owned = self.host.actors.resolve_owned(id)?;
        self.players.contains_key(owned.id()).then_some(owned)
    }

    /// Mutate an entity record.
    pub fn update_entity(&mut self, id: &ActorId, update: impl FnOnce(&mut Q1Actor)) -> Result<(), Q1Error> {
        match self.entities.get_mut(id) {
            Some(entity) => {
                update(entity);
                Ok(())
            }
            None => Err(q1_error("Missing Q1 entity")),
        }
    }

    /// Mutate a player state.
    pub fn update_player(
        &mut self,
        id: &ActorId,
        update: impl FnOnce(&mut super::types::Q1PlayerState),
    ) -> Result<(), Q1Error> {
        let owned = self
            .player_owned(id)
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        match self.players.get_mut(owned.id()) {
            Some(player) => {
                update(player);
                Ok(())
            }
            None => Err(q1_error("Player has no Q1 weapon state")),
        }
    }

    /// Entity ids in admission order.
    #[must_use]
    pub fn entity_ids(&self) -> Vec<ActorId> {
        self.entity_order.clone()
    }

    /// Player states in admission order.
    #[must_use]
    pub fn players_snapshot(&self) -> Vec<super::types::Q1PlayerState> {
        self.player_order
            .iter()
            .filter_map(|id| self.players.get(id).cloned())
            .collect()
    }

    /// Resolve an id to itself when a source entity is present.
    #[must_use]
    pub fn entity_present(&self, id: &ActorId) -> Option<ActorId> {
        self.entities.contains_key(id).then(|| id.clone())
    }

    /// Read a stored callback name.
    pub fn callback_store(&self, id: &ActorId, slot: Q1CallbackSlot) -> Result<Option<String>, Q1Error> {
        let entity = self.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
        Ok(match slot {
            Q1CallbackSlot::Think => entity.think.clone(),
            Q1CallbackSlot::Use => entity.use_callback.clone(),
            Q1CallbackSlot::Touch => entity.touch.clone(),
            Q1CallbackSlot::Pain => entity.pain.clone(),
            Q1CallbackSlot::Die => entity.die.clone(),
            Q1CallbackSlot::Blocked => entity.blocked.clone(),
            Q1CallbackSlot::PathEnd => entity.path_end.clone(),
        })
    }

    /// Whether an actor is a player.
    pub fn is_player(&mut self, actor: &ActorId) -> bool {
        if self.player_ref(actor).is_some() {
            return true;
        }
        let players = (self.host.players)();
        players.iter().any(|player| same_actor(player, actor))
    }

    /// Whether an actor is live.
    #[must_use]
    pub fn is_live(&self, actor: &ActorId) -> bool {
        self.host.actors.is_live(actor)
    }

    /// Read actor health (`0` when unbound).
    #[must_use]
    pub fn health(&self, actor: &ActorId) -> f64 {
        self.host.combat.read(actor).map(|combat| combat.health).unwrap_or(0.0)
    }

    /// Write combat health for an entity.
    pub fn set_health(&mut self, id: &ActorId, health: f64) -> Result<(), Q1Error> {
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        self.host.combat.set_health(&owned, health)
    }

    /// Read an entity body.
    pub fn body(&self, id: &ActorId) -> Result<BodyState, Q1Error> {
        self.host.bodies.read(id).ok_or_else(|| q1_error("Missing Q1 body"))
    }

    /// Patch an entity body.
    pub fn set_body(&mut self, id: &ActorId, patch: &BodyPatch) -> Result<(), Q1Error> {
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 body"))?;
        let current = self.body(id)?;
        self.host.bodies.write(&owned, &patch.apply_to(&current))
    }

    /// Link an entity body for spatial queries.
    pub fn link(&mut self, id: &ActorId) -> Result<(), Q1Error> {
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        self.host.bodies.link(&owned)
    }

    /// Move an entity origin.
    pub fn set_origin(&mut self, id: &ActorId, origin: Vec3) -> Result<(), Q1Error> {
        self.set_body(
            id,
            &BodyPatch {
                origin: Some(origin),
                ..Default::default()
            },
        )
    }

    /// Resize an entity bounds.
    pub fn set_bounds(&mut self, id: &ActorId, bounds: Bounds) -> Result<(), Q1Error> {
        self.set_body(
            id,
            &BodyPatch {
                bounds: Some(bounds),
                ..Default::default()
            },
        )
    }

    /// Whether an entity can take damage.
    #[must_use]
    pub fn is_damageable(&self, id: &ActorId) -> bool {
        self.host.combat.read(id).is_some_and(|combat| combat.can_take_damage)
    }

    /// Set whether an entity can take damage.
    pub fn set_damageable(&mut self, id: &ActorId, damageable: bool) -> Result<(), Q1Error> {
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let combat = self
            .host
            .combat
            .read(owned.id())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        self.host.combat.set_traits(
            &owned,
            CombatTraits {
                can_take_damage: damageable,
                mass: combat.mass,
                invulnerable: combat.invulnerable,
                team: combat.team,
                no_knockback: combat.no_knockback,
            },
        )
    }

    /// Remove a live entity (already-gone entities are a no-op).
    pub fn remove(&mut self, id: &ActorId) -> Result<(), Q1Error> {
        let owned = self.entity_ref(id).map(|entity| entity.actor.clone());
        if owned.as_ref().is_some_and(|owned| self.is_live(owned.id())) {
            self.release_actor(owned.as_ref().expect("live"))?;
        }
        Ok(())
    }

    /// Release a shared actor and run game-registered release hooks.
    pub fn release_actor(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
        self.host.actors.release(actor)?;
        self.host.bodies.remove(actor.id());
        self.entities.remove(actor.id());
        self.entity_order.retain(|id| id != actor.id());
        self.authored_targets.remove(actor.id());
        self.monster_missions.remove(actor.id());
        self.players.remove(actor.id());
        self.player_order.retain(|id| id != actor.id());
        self.host.cancel_think(actor);
        let hooks = std::mem::take(&mut self.release_hooks);
        for hook in &hooks {
            hook.borrow_mut().on_release(self, actor);
        }
        self.release_hooks.extend(hooks);
        Ok(())
    }

    /// Register an actor-release hook.
    pub fn register_release_hook(&mut self, hook: Rc<RefCell<dyn Q1ReleaseHook>>) {
        self.release_hooks.push(hook);
    }

    /// Schedule a named action after a delay in seconds.
    pub fn schedule(&mut self, id: &ActorId, delay: f64, callback: &str) -> Result<(), Q1Error> {
        let callback = self.named.action(callback)?;
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let due = self.time + delay;
        self.update_entity(id, |entity| {
            entity.think = Some(callback);
            entity.next_think = due;
        })?;
        self.host.schedule_think(&owned, due);
        Ok(())
    }

    /// Schedule a named action at an absolute source time.
    pub fn schedule_at(&mut self, id: &ActorId, due: f64, callback: &str) -> Result<(), Q1Error> {
        let callback = self.named.action(callback)?;
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        self.update_entity(id, |entity| {
            entity.think = Some(callback);
            entity.next_think = due;
        })?;
        self.host.schedule_think(&owned, due);
        Ok(())
    }

    /// Cancel a scheduled think (missing entities are already gone).
    pub fn cancel(&mut self, id: &ActorId) {
        let owned = self.entity_ref(id).map(|entity| entity.actor.clone());
        let cancelled = self
            .update_entity(id, |entity| {
                entity.think = None;
                entity.next_think = -1.0;
            })
            .is_ok();
        if cancelled {
            if let Some(owned) = owned {
                self.host.cancel_think(&owned);
            }
        }
    }

    /// Play a positioned sound.
    pub fn sound(
        &mut self,
        actor: &ActorId,
        path: &str,
        channel: Q1SoundChannel,
        attenuation: f64,
        volume: f64,
    ) -> Result<(), Q1Error> {
        if self.host.bodies.read(actor).is_none() {
            return Err(q1_error("Missing Q1 sound emitter body"));
        }
        self.host.emit(Q1Event::Sound {
            origin: None,
            actor: actor.clone(),
            path: path.to_string(),
            channel,
            attenuation,
            volume,
        });
        Ok(())
    }

    /// Play a voice-channel sound at unit attenuation and volume.
    pub fn sound_simple(&mut self, actor: &ActorId, path: &str) -> Result<(), Q1Error> {
        self.sound(actor, path, Q1SoundChannel::Voice, 1.0, 1.0)
    }

    /// Center/print a message to a player.
    pub fn message(&mut self, player: Option<&ActorId>, text: &str, center: bool, args: Vec<Q1MessageArg>) {
        if player.is_some_and(|player| self.is_player(player)) && !text.is_empty() {
            self.host.emit(Q1Event::Message {
                player: player.expect("player").clone(),
                text: text.to_string(),
                center,
                args: Some(args),
                parts: None,
            });
        }
    }

    /// Center a message to a player without arguments.
    pub fn message_simple(&mut self, player: Option<&ActorId>, text: &str) {
        self.message(player, text, true, Vec::new());
    }

    /// Broadcast a temp-entity effect.
    pub fn effect(&mut self, effect: Q1Effect, origin: Vec3, actor: Option<&ActorId>, amount: i32) {
        self.host.emit(Q1Event::Effect {
            effect,
            actor: actor.cloned(),
            origin,
            amount,
            muzzle: None,
        });
    }

    /// Broadcast a temp-entity effect without an actor at unit amount.
    pub fn effect_simple(&mut self, effect: Q1Effect, origin: Vec3) {
        self.effect(effect, origin, None, 1);
    }

    /// Find entities by target name in admission order.
    #[must_use]
    pub fn find(&self, targetname: &str) -> Vec<ActorId> {
        self.entity_order
            .iter()
            .filter(|id| {
                self.entities
                    .get(*id)
                    .is_some_and(|entity| entity.targetname == targetname)
            })
            .cloned()
            .collect()
    }

    /// Fire delayed use/target/killtarget/message for an entity or
    /// authored target.
    pub fn use_targets(&mut self, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
        let (delay, target, killtarget, message, actor) = match self.entity_ref(id) {
            Some(entity) => (
                entity.delay,
                entity.target.clone(),
                entity.killtarget.clone(),
                entity.message.clone(),
                entity.actor.id().clone(),
            ),
            None => match self.authored_targets.get(id) {
                Some(target) => (
                    target.delay,
                    target.target.clone(),
                    target.killtarget.clone(),
                    target.message.clone(),
                    target.actor.id().clone(),
                ),
                None => return Err(q1_error("Missing Q1 use-target entity")),
            },
        };
        if delay > 0.0 {
            let delayed = self.create("DelayedUse", None, None)?;
            let activator = activator.cloned();
            self.update_entity(&delayed, |entity| {
                entity.message = message;
                entity.target = target;
                entity.killtarget = killtarget;
                entity.activator = activator;
            })?;
            return self.schedule(&delayed, delay, "DelayThink");
        }
        if activator.is_some() && !message.is_empty() {
            self.message_simple(activator, &message);
        }
        if !killtarget.is_empty() {
            let victims = self.target_actors(&killtarget);
            for victim in victims {
                if self.is_live(&victim) {
                    let owned = self.entity_ref(&victim).map(|entity| entity.actor.clone());
                    match owned {
                        Some(owned) => self.release_actor(&owned)?,
                        None => {
                            if let Some(owned) = self.host.actors.resolve_owned(&victim) {
                                self.release_actor(&owned)?;
                            }
                        }
                    }
                }
            }
        }
        if !target.is_empty() {
            let targets = self.target_actors(&target);
            for target in targets {
                if self.is_live(&target) {
                    self.fire_use(&target, Some(&actor), activator)?;
                }
            }
        }
        Ok(())
    }

    /// Resolve live target actors by name in source-slot order.
    fn target_actors(&mut self, targetname: &str) -> Vec<ActorId> {
        let entities = &self.entities;
        let order = &self.entity_order;
        let mut actors: Vec<ActorId> = order
            .iter()
            .filter(|id| entities.get(*id).is_some_and(|entity| entity.targetname == targetname) && self.is_live(id))
            .cloned()
            .collect();
        let mut authored: Vec<ActorId> = self
            .authored_targets
            .values()
            .filter(|target| target.targetname == targetname && self.is_live(target.actor.id()))
            .map(|target| target.actor.id().clone())
            .collect();
        actors.append(&mut authored);
        actors.sort_by_key(|id| {
            self.host
                .actors
                .source_of(id)
                .map(|source| source.slot)
                .unwrap_or_else(|| id.slot())
        });
        actors
    }

    /// Apply direct damage with default parameters.
    pub fn damage_direct(
        &mut self,
        target: &ActorId,
        inflictor: Option<&ActorId>,
        attacker: Option<&ActorId>,
        amount: f64,
    ) -> DamageOutcome {
        self.damage(target, inflictor, attacker, amount, &Q1DamageParams::default())
    }

    /// Damage a target through the shared combat authority.
    pub fn damage(
        &mut self,
        target: &ActorId,
        inflictor: Option<&ActorId>,
        attacker: Option<&ActorId>,
        amount: f64,
        params: &Q1DamageParams,
    ) -> DamageOutcome {
        let sequence = self.sequence;
        self.sequence += 1;
        let multiplier = match attacker {
            Some(attacker) => match self.host.source_damage_multiplier.as_mut() {
                Some(multiplier) => multiplier(attacker),
                None => 1.0,
            },
            None => 1.0,
        };
        let scaled = f64::from((f64::from((amount) as f32) * f64::from((multiplier) as f32)) as f32);
        let mut request = DamageRequest {
            attack: super::gameplay::AttackProvenance {
                sequence,
                time: SourceTime::Seconds(self.time as f32),
                attacker: attacker.cloned(),
                inflictor: inflictor.cloned(),
                originating_projectile: None,
                weapon: params.weapon.map(|weapon| self.weapon_item(weapon)),
                weapon_provider: self.provider(),
                damage_powerup_owner: self.host.source_damage_powerup_owner.clone(),
                combat_provider: self.options().combat_provider.clone(),
                inventory_provider: self.options().inventory_provider.clone(),
                movement_provider: self.options().movement_provider.clone(),
                cause: AttackCause::Q1 {
                    death_type: params.death_type.clone(),
                    armor_effect: params.armor_effect,
                },
            },
            target: target.clone(),
            amount: scaled,
            knockback: scaled,
            direction: ZERO,
            point: ZERO,
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            delivery: params.delivery,
        };
        request = apply_source_damage_modifier(
            request,
            self.host.source_damage_modifier.as_ref(),
            &|actor: &ActorId| self.host.actors.is_live(actor),
        );
        let mut applied = request.clone();
        applied.knockback = request.amount;
        self.host.combat.apply(&applied)
    }

    /// Read a player powerup expiry in seconds.
    pub fn powerup_expires(&mut self, player: &ActorId, powerup: Q1Powerup) -> f64 {
        if let Some(expires) = self.host.powerup_expires.as_mut() {
            return expires(player, powerup);
        }
        self.player_ref(player)
            .and_then(|player| player.powerups.get(&powerup).copied())
            .unwrap_or(0.0)
    }

    /// Resolve the combat policy context for a request.
    pub fn combat_context(&mut self, request: &DamageRequest) -> Q1CombatContext {
        let teamplay = match self.options().teamplay {
            Some(teamplay) => teamplay,
            None => {
                if self.options().deathmatch == 0 {
                    0
                } else {
                    1
                }
            }
        };
        let walk = self
            .entity_ref(&request.target)
            .is_some_and(|entity| entity.movement == Q1MoveType::Step);
        let quad = match request.attack.attacker.as_ref() {
            Some(attacker) => self.powerup_expires(attacker, Q1Powerup::Quad) > self.time,
            None => false,
        };
        let momentum_direction = self
            .world
            .as_ref()
            .filter(|world| same_actor(world, &request.target))
            .and_then(|_| self.host.bodies.linked(&request.target))
            .map(|linked| linked.state.velocity);
        Q1CombatContext {
            arithmetic: Q1CombatArithmetic::Binary32,
            quad,
            teamplay,
            base_team_health: self.base_team_health,
            walk,
            momentum_direction,
        }
    }

    /// Observe source damage traits for an actor.
    pub fn source_target(&mut self, actor: &ActorId) -> Q1SourceTarget {
        if let Some(source_target) = self.host.source_target.as_mut() {
            return source_target(actor);
        }
        match self.entity_ref(actor) {
            Some(entity) => Q1SourceTarget {
                aimed_damage: entity.aimed_damage,
                push: entity.movement == Q1MoveType::Push,
                player: self.player_owned(actor).is_some(),
                slidebox: entity.solid == Q1Solid::Slidebox,
            },
            None => Q1SourceTarget {
                aimed_damage: false,
                push: false,
                player: false,
                slidebox: false,
            },
        }
    }

    /// Observe a monster target through the host hook.
    pub fn monster_target(&mut self, actor: &ActorId) -> Option<MonsterTargetObservation> {
        match self.host.monster_target.as_mut() {
            Some(observe) => observe(actor),
            None => None,
        }
    }

    /// Whether an inflictor can damage a target along a clear trace.
    pub fn can_damage(&mut self, target: &ActorId, inflictor: &ActorId) -> bool {
        let (Some(target_body), Some(inflictor_body)) =
            (self.host.bodies.read(target), self.host.bodies.read(inflictor))
        else {
            return false;
        };
        let start = vadd(
            inflictor_body.origin,
            vscale(vadd(inflictor_body.bounds.min, inflictor_body.bounds.max), 0.5),
        );
        for offset in [
            Vec3 {
                x: 15.0,
                y: 15.0,
                z: 0.0,
            },
            Vec3 {
                x: -15.0,
                y: 15.0,
                z: 0.0,
            },
            Vec3 {
                x: 15.0,
                y: -15.0,
                z: 0.0,
            },
            Vec3 {
                x: -15.0,
                y: -15.0,
                z: 0.0,
            },
        ] {
            let trace = self.host.trace(&Q1TraceRequest {
                start,
                end: vadd(
                    vadd(
                        target_body.origin,
                        vscale(vadd(target_body.bounds.min, target_body.bounds.max), 0.5),
                    ),
                    offset,
                ),
                bounds: POINT,
                ignore: Some(inflictor.clone()),
                monsters: false,
                missile: false,
            });
            if trace.fraction == 1.0 {
                return true;
            }
        }
        false
    }

    /// Damage actors within a radius, scaling by distance.
    pub fn radius_damage(
        &mut self,
        inflictor: &ActorId,
        attacker: Option<&ActorId>,
        damage: f64,
        ignore: Option<&ActorId>,
        weapon: Option<Q1Weapon>,
        death_type: &str,
    ) {
        let observations = self.host.actors.observations();
        for observation in &observations {
            let target = &observation.id;
            if !self.is_live(target) || ignore == Some(target) {
                continue;
            }
            let target_body = self.host.bodies.read(target);
            let inflictor_body = self.host.bodies.read(inflictor);
            let (Some(target_body), Some(inflictor_body)) = (target_body, inflictor_body) else {
                continue;
            };
            let target_traits = self.source_target(target);
            if self.player_owned(target).is_none() && !target_traits.slidebox && !target_traits.push {
                continue;
            }
            let mine = vadd(
                target_body.origin,
                vscale(vadd(target_body.bounds.min, target_body.bounds.max), 0.5),
            );
            let distance = f64::from(length(vsub(mine, inflictor_body.origin)));
            if distance > damage + 40.0 || !self.can_damage(target, inflictor) {
                continue;
            }
            let points = 0.5f64.mul_add(distance, damage);
            if points > 0.0 {
                self.damage(
                    target,
                    Some(inflictor),
                    attacker,
                    points,
                    &Q1DamageParams {
                        weapon,
                        delivery: DamageDelivery::Radius,
                        death_type: death_type.to_string(),
                        ..Default::default()
                    },
                );
            }
        }
    }

    /// Move an entity toward a destination, invoking a named action on
    /// arrival (`SUB_CalcMove`).
    pub fn calc_move(&mut self, id: &ActorId, destination: Vec3, speed: f64, done: &str) -> Result<(), Q1Error> {
        if speed <= 0.0 {
            return Err(q1_range("Q1 move speed must be positive"));
        }
        let done = self.named.action(done)?;
        self.cancel(id);
        let id = id.clone();
        let origin = self.body(&id).map(|body| body.origin)?;
        let delta = vsub(destination, origin);
        let travel = f64::from(length(delta)) / speed;
        if travel < 0.03 {
            self.set_origin(&id, destination)?;
            self.update_entity(&id, |entity| entity.move_completion = None)?;
            return self.invoke_action(&id, &done);
        }
        let velocity = vscale(delta, 1.0 / travel.max(0.1));
        self.set_body(
            &id,
            &BodyPatch {
                velocity: Some(velocity),
                ..Default::default()
            },
        )?;
        let ltime = self.entity_ref(&id).map(|entity| entity.number("ltime")).unwrap_or(0.0);
        self.update_entity(&id, |entity| {
            entity.move_completion = Some(Q1Move {
                destination,
                done: done.clone(),
            });
        })?;
        self.schedule_at(&id, ltime + travel.max(0.1), "SUB_CalcMoveDone")
    }

    /// Step pushers and projectiles.
    pub fn physics_step(&mut self, seconds: f64, elapsed: f64) -> Result<(), Q1Error> {
        let ids = self.entity_ids();
        for id in ids {
            let owned = self.entity_ref(&id).map(|entity| entity.actor.clone());
            if let Some(owned) = owned {
                self.physics_entity(&owned, seconds, elapsed)?;
            }
        }
        Ok(())
    }

    /// Launch a projectile behavior through the host port.
    pub fn launch_projectile_behavior(
        &mut self,
        id: &ActorId,
        shooter: &ActorId,
        weapon: Q1Weapon,
        role: crate::contract::ProjectileRole,
    ) -> Result<(), Q1Error> {
        if !self.is_player(shooter) {
            return Ok(());
        }
        let owned = self
            .entity_ref(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let body = self.body(id)?;
        let weapon_item = self.weapon_item(weapon);
        let time_seconds = self.time;
        let update = match self.host.weapon_behavior.as_mut() {
            Some(behavior) => behavior.launch(&Q1WeaponBehaviorLaunch {
                projectile: owned,
                shooter: shooter.clone(),
                weapon: weapon_item,
                role,
                time_seconds,
                body,
            }),
            None => None,
        };
        if let Some(update) = update {
            self.project_trajectory(id, &update)?;
        }
        Ok(())
    }

    /// Apply a projectile behavior step through the host port.
    pub fn apply_projectile_behavior(&mut self, actor: &OwnedActor, seconds: f64) -> Result<(), Q1Error> {
        if !self.is_live(actor.id()) {
            return Ok(());
        }
        let body = self.body(actor.id())?;
        let update = match self.host.weapon_behavior.as_mut() {
            Some(behavior) => behavior.step(actor, &body, seconds),
            None => None,
        };
        if let Some(update) = update {
            self.project_trajectory(actor.id(), &update)?;
        }
        Ok(())
    }

    /// Step one actor's physics.
    pub fn physics_entity(&mut self, actor: &OwnedActor, seconds: f64, elapsed: f64) -> Result<(), Q1Error> {
        let id = actor.id().clone();
        let movement = match self.entity_ref(&id) {
            Some(entity) => entity.movement,
            None => return Ok(()),
        };
        if movement == Q1MoveType::Push {
            self.host.step_pusher(actor.id(), elapsed);
            return Ok(());
        }
        self.apply_projectile_behavior(actor, seconds)?;
        if !self.is_live(actor.id()) {
            return Ok(());
        }
        let movement = self
            .entity_ref(&id)
            .map(|entity| entity.movement)
            .unwrap_or(Q1MoveType::None);
        if movement == Q1MoveType::Noclip {
            return Ok(());
        }
        if movement == Q1MoveType::Toss || movement == Q1MoveType::Bounce || movement == Q1MoveType::Flymissile {
            self.projectile_physics(&id, seconds, matches!(movement, Q1MoveType::Bounce))?;
        }
        Ok(())
    }

    /// Integrate projectile motion and impacts.
    fn projectile_physics(&mut self, id: &ActorId, seconds: f64, bounce: bool) -> Result<(), Q1Error> {
        let id = id.clone();
        let mut body = self.body(&id)?;
        if self.entity_ref(&id).is_some_and(|entity| entity.classname == "gib") || body.ground.is_none() {
            body.velocity = vadd(
                body.velocity,
                vscale(
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: -1.0,
                    },
                    800.0 * seconds,
                ),
            );
        }
        let origin = body.origin;
        let move_vector = vscale(body.velocity, seconds);
        let end = vadd(origin, move_vector);
        let missile = self
            .entity_ref(&id)
            .is_some_and(|entity| entity.movement == Q1MoveType::Flymissile);
        let trace = self.host.trace(&Q1TraceRequest {
            start: origin,
            end,
            bounds: POINT,
            ignore: Some(id.clone()),
            monsters: true,
            missile,
        });
        self.set_body(
            &id,
            &BodyPatch {
                origin: Some(trace.end),
                velocity: Some(body.velocity),
                ..Default::default()
            },
        )?;
        if self.entity_ref(&id).is_none() {
            return Ok(());
        }
        self.link(&id)?;
        if trace.sky {
            self.remove(&id)?;
            return Ok(());
        }
        if trace.fraction != 1.0 {
            if self.entity_ref(&id).and_then(|entity| entity.projectile).is_some() {
                super::weapons::projectile_touch(self, &id, trace.actor.as_ref(), trace.normal, None)?;
            } else if let Some(other) = trace.actor.clone() {
                // Donor `Q1Trace` never carries a surface, so the
                // `"surface" in trace` check always yields null here.
                self.invoke_touch(&id, &other, Some(trace.normal), None)?;
            }
            if self.entity_ref(&id).is_none() {
                return Ok(());
            }
            if bounce {
                let velocity = self.body(&id).map(|body| body.velocity)?;
                let backoff = f64::from(dot(velocity, trace.normal)) * 2.0;
                self.set_body(
                    &id,
                    &BodyPatch {
                        velocity: Some(vadd(velocity, vscale(trace.normal, -backoff))),
                        ..Default::default()
                    },
                )?;
            } else {
                self.set_body(
                    &id,
                    &BodyPatch {
                        velocity: Some(ZERO),
                        ..Default::default()
                    },
                )?;
            }
        }
        let ground = trace.actor.clone().filter(|_| trace.fraction < 1.0);
        self.set_body(
            &id,
            &BodyPatch {
                ground: Some(ground),
                ..Default::default()
            },
        )?;
        self.check_water_transition(&id)?;
        Ok(())
    }

    /// Run one player frame.
    pub fn player_frame(&mut self, actor: &OwnedActor, seconds: f64, water_level: Option<i32>) -> Result<(), Q1Error> {
        let id = actor.id().clone();
        if self.player_ref(&id).is_none() {
            return Ok(());
        }
        let hooks: Vec<Q1PlayerSecondsHook<Result<(), Q1Error>>> = self
            .player_extensions
            .iter()
            .filter_map(|(_, extension)| extension.frame)
            .collect();
        for hook in hooks {
            hook(self, &id, seconds)?;
        }
        let water_level = water_level.unwrap_or(0);
        self.update_player(&id, |player| player.water_level = water_level)?;
        if water_level > 1 {
            let death = if water_level == 3 {
                "player/h2odeath.wav"
            } else {
                "player/inlava.wav"
            };
            let pain = if water_level == 3 {
                "player/drown1.wav"
            } else {
                "player/lburn1.wav"
            };
            self.sound_simple(&id, death)?;
            let drown_at = self.player_ref(&id).map(|player| player.drown_at).unwrap_or(0.0);
            let drown_damage = self.player_ref(&id).map(|player| player.drown_damage).unwrap_or(0.0);
            if drown_at < seconds {
                self.sound_simple(&id, pain)?;
                let mut damage = drown_damage + 2.0;
                if damage > 15.0 {
                    damage = 10.0;
                }
                self.update_player(&id, |player| {
                    player.drown_damage = damage;
                    player.drown_at = seconds + 1.0;
                })?;
                self.damage_direct(&id, None, None, damage);
            }
        } else {
            self.update_player(&id, |player| {
                player.drown_at = seconds + 2.0;
                player.drown_damage = 2.0;
            })?;
        }
        let states = self.player_ref(&id).cloned().expect("player");
        if states.hostile_until != 0.0 && self.time >= states.hostile_until {
            self.update_player(&id, |player| player.hostile_until = 0.0)?;
        }
        if states.weapon_animation_at >= 0.0 && self.time >= states.weapon_animation_at {
            self.update_player(&id, |player| player.weapon_animation_at = -1.0)?;
            self.weapon_frame(actor, seconds)?;
        }
        let quad = self.powerup_expires(&id, Q1Powerup::Quad);
        let invisibility = self.powerup_expires(&id, Q1Powerup::Invisibility);
        let mut effects = self.entity_ref(&id).map(|entity| entity.effects).unwrap_or(0);
        if quad > self.time {
            effects |= 16;
        } else {
            effects &= !16;
        }
        if invisibility > self.time {
            effects |= 128;
        } else {
            effects &= !128;
        }
        self.update_entity(&id, |entity| entity.effects = effects)?;
        let expired: Vec<Q1Powerup> = self
            .player_ref(&id)
            .map(|player| {
                player
                    .powerups
                    .keys()
                    .filter(|powerup| !matches!(powerup, Q1Powerup::Quad))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for powerup in expired {
            let expires = self.powerup_expires(&id, powerup);
            if expires <= self.time {
                self.update_player(&id, |player| {
                    player.powerups.remove(&powerup);
                })?;
                self.host.emit(Q1Event::Powerup {
                    player: id.clone(),
                    powerup,
                    expires: 0.0,
                });
            }
        }
        Ok(())
    }

    /// Apply view punch to a player.
    pub fn weapon_punch(&mut self, player: &ActorId, pitch: f64) -> Result<(), Q1Error> {
        if self.player_owned(player).is_none() {
            return Ok(());
        }
        let mut punch = match self.host.punch_angles.as_mut() {
            Some(punch_angles) => punch_angles.read(player),
            None => self
                .player_ref(player)
                .map(|player| player.punch_angles)
                .unwrap_or(ZERO),
        };
        punch.x = (f64::from(punch.x) + pitch) as f32;
        if let Some(punch_angles) = self.host.punch_angles.as_mut() {
            punch_angles.write(player, punch);
        } else {
            self.update_player(player, |player| player.punch_angles = punch)?;
        }
        Ok(())
    }

    /// Decay view punch for an actor.
    pub fn advance_punch(&mut self, actor: &ActorId, elapsed: f64, numeric: &NumericOps) -> Option<Vec3> {
        let punch = match self.host.punch_angles.as_mut() {
            Some(punch_angles) => punch_angles.read(actor),
            None => self.player_ref(actor).map(|player| player.punch_angles)?,
        };
        Some(drop_q1_punch(punch, elapsed, numeric))
    }

    /// Present the player's weapon frame.
    pub fn weapon_frame(&mut self, actor: &OwnedActor, seconds: f64) -> Result<(), Q1Error> {
        let id = actor.id().clone();
        let states = match self.player_ref(&id).cloned() {
            Some(states) => states,
            None => return Ok(()),
        };
        if states.continuous_firing {
            let fired = self.fire_registered_weapon(&id)?;
            let next = if fired {
                seconds + self.weapon_frame_delay(&id, 0.1)?
            } else {
                seconds + 0.1
            };
            self.update_player(&id, |player| player.next_weapon_frame = next)?;
            return Ok(());
        }
        let frame = states.weapon_frame;
        let mut states = states;
        match states.weapon {
            Q1Weapon::Nailgun | Q1Weapon::Supernailgun => {
                if frame == 1 {
                    self.update_player(&id, |player| player.weapon_frame = 2)?;
                    states.weapon_frame = 2;
                } else {
                    self.update_player(&id, |player| player.weapon_frame = 1)?;
                    states.weapon_frame = 1;
                }
            }
            Q1Weapon::Grenadelauncher | Q1Weapon::Rocketlauncher => {
                let next = (frame % 5) + 1;
                self.update_player(&id, |player| player.weapon_frame = next)?;
                states.weapon_frame = next;
            }
            Q1Weapon::Lightning => {
                let next = (frame % 3) + 1;
                self.update_player(&id, |player| player.weapon_frame = next)?;
                states.weapon_frame = next;
                if states.lightning_sound_at < seconds {
                    self.sound(&id, "weapons/lstart.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
                    self.update_player(&id, |player| player.lightning_sound_at = seconds + 0.6)?;
                }
            }
            _ => {}
        }
        let view_model = self.weapon_model(states.weapon, Some(&id))?;
        let frame = states.weapon_frame;
        self.host.emit(Q1Event::Weapon {
            player: id.clone(),
            weapon: states.weapon,
            view_model,
            frame,
            punch: 0,
            attack: None,
        });
        Ok(())
    }

    /// Run post-physics player hooks.
    pub fn player_after_physics(&mut self, actor: &OwnedActor, seconds: f64) -> Result<(), Q1Error> {
        let id = actor.id().clone();
        if self.player_ref(&id).is_none() {
            return Ok(());
        }
        let hooks: Vec<Q1PlayerSecondsHook<Result<(), Q1Error>>> = self
            .player_extensions
            .iter()
            .filter_map(|(_, extension)| extension.after_physics)
            .collect();
        for hook in hooks {
            hook(self, &id, seconds)?;
        }
        Ok(())
    }

    /// Travel to a map.
    pub fn travel(&mut self, map: &str, cause: Option<&ActorId>) {
        self.host.transition(TransitionIntent::CampaignLevel {
            campaign: self.options().campaign.clone(),
            map: map.to_string(),
            spawn_point: String::from("start"),
            gates: Vec::new(),
            cause: cause.cloned(),
        });
    }

    /// Begin an intermission camera.
    pub fn begin_intermission(&mut self, map: &str, cause: Option<&ActorId>) -> Result<(), Q1Error> {
        let spots = self.find("info_intermission");
        let spot = spots
            .first()
            .ok_or_else(|| q1_error("FindIntermission: no spot"))?
            .clone();
        let body = self.body(&spot)?;
        let mut angles = body.angles;
        angles.x = 0.0;
        angles.z = 0.0;
        let players: Vec<ActorId> = self.player_order.clone();
        for player in &players {
            let punch = match self.host.punch_angles.as_mut() {
                Some(punch_angles) => {
                    punch_angles.write(player, ZERO);
                    ZERO
                }
                None => {
                    self.update_player(player, |player| player.punch_angles = ZERO)?;
                    ZERO
                }
            };
            let _ = punch;
            self.set_body(
                player,
                &BodyPatch {
                    origin: Some(body.origin),
                    angles: Some(angles),
                    velocity: Some(ZERO),
                    ..Default::default()
                },
            )?;
            self.update_entity(player, |entity| {
                entity.model = String::new();
                entity.think = None;
            })?;
            self.host.emit(Q1Event::Camera {
                player: player.clone(),
                origin: body.origin,
                angles,
                view_offset: Some(ZERO),
            });
        }
        let exit_after = self.time + 1.0;
        self.intermission = Some(Q1Intermission {
            map: map.to_string(),
            cause: cause.cloned(),
            exit_after,
        });
        self.host.emit(Q1Event::Intermission {
            origin: body.origin,
            angles,
            map: map.to_string(),
            exit_after,
            track: 3,
        });
        Ok(())
    }

    /// Request an intermission exit.
    pub fn request_intermission_exit(&mut self, seconds: f64, pressed: bool) -> bool {
        match self.intermission.clone() {
            Some(intermission) if pressed && seconds > intermission.exit_after => {
                self.travel(&intermission.map.clone(), intermission.cause.as_ref());
                self.intermission = None;
                true
            }
            _ => false,
        }
    }

    /// Save and return the angle basis.
    pub fn make_vectors(&mut self, angles: Vec3) -> Q1Basis {
        self.basis = vectors(angles);
        self.basis
    }

    /// Begin a source frame.
    pub fn begin_frame(&mut self, seconds: f64, elapsed: f64) {
        self.time = seconds;
        self.frame_seconds = elapsed;
    }

    /// Record player input.
    pub fn player_input(&mut self, actor: &OwnedActor, input: &Q1PlayerInput) {
        let id = actor.id().clone();
        let teleport_until = input.teleport_until;
        let _ = self.update_player(&id, |player| {
            player.attack_held = input.attack;
            player.jump_held = input.jump;
            if let Some(teleport_until) = teleport_until {
                player.teleport_until = teleport_until;
            }
        });
    }

    /// Run the attack button for a player.
    pub fn attack(
        &mut self,
        actor: &OwnedActor,
        view_angles: Vec3,
        seconds: f64,
        water_level: i32,
    ) -> Result<bool, Q1Error> {
        let id = actor.id().clone();
        let states = self
            .player_ref(&id)
            .cloned()
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        if states.attack_finished > seconds || states.teleport_until > seconds {
            return Ok(false);
        }
        self.update_player(&id, |player| player.view_angles = view_angles)?;
        self.weapon_before_fire(&id)?;
        let (fire, ammo, per_shot) = {
            let definition = self
                .registered_weapons
                .get(&states.weapon)
                .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
            (
                definition.fire,
                definition.ammo.clone(),
                definition.ammo_per_shot.unwrap_or(1.0),
            )
        };
        if let Some(ammo) = ammo {
            if !self.consume_weapon_ammo(&id, &ammo, per_shot)? {
                return Ok(false);
            }
        }
        let elapsed = (seconds - self.time).max(0.0);
        if !fire(self, &id)? {
            return Ok(false);
        }
        if water_level == 3 {
            self.sound(&id, "player/inlava.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
        }
        let (attack_finished, weapon_animation_at, weapon_animation_base) = {
            let states = self.player_ref(&id).cloned().expect("player");
            let attack_finished = match states.weapon {
                Q1Weapon::Axe => seconds + self.weapon_attack_delay(&id, 0.5)?,
                Q1Weapon::Shotgun => seconds + self.weapon_attack_delay(&id, 0.5)?,
                Q1Weapon::Supershotgun => seconds + self.weapon_attack_delay(&id, 0.7)?,
                Q1Weapon::Nailgun => seconds + self.weapon_attack_delay(&id, 0.1)?,
                Q1Weapon::Supernailgun => seconds + self.weapon_attack_delay(&id, 0.1)?,
                Q1Weapon::Grenadelauncher => seconds + self.weapon_attack_delay(&id, 0.6)?,
                Q1Weapon::Rocketlauncher => seconds + self.weapon_attack_delay(&id, 0.8)?,
                Q1Weapon::Lightning => seconds + self.weapon_attack_delay(&id, 0.1)?,
                _ => seconds + self.weapon_attack_delay(&id, 0.5)?,
            };
            let weapon_animation_at = self.time + elapsed;
            let weapon_animation_base = match states.weapon {
                Q1Weapon::Axe => 1,
                Q1Weapon::Shotgun => 3,
                Q1Weapon::Supershotgun => 2,
                Q1Weapon::Nailgun => 1,
                Q1Weapon::Supernailgun => 3,
                Q1Weapon::Grenadelauncher => 2,
                Q1Weapon::Rocketlauncher => 1,
                Q1Weapon::Lightning => 2,
                _ => 1,
            };
            (attack_finished, weapon_animation_at, weapon_animation_base)
        };
        self.update_player(&id, |player| {
            player.attack_finished = attack_finished;
            player.weapon_animation_at = weapon_animation_at;
            player.weapon_animation_base = weapon_animation_base;
        })?;
        Ok(true)
    }

    /// Run weapon selection input for a player.
    pub fn weapon_input(
        &mut self,
        actor: &OwnedActor,
        pressed: Option<Q1Weapon>,
        view_angles: Vec3,
        seconds: f64,
        water_level: i32,
    ) -> Result<bool, Q1Error> {
        let id = actor.id().clone();
        if self.player_ref(&id).is_none() {
            return Err(q1_error("Player has no Q1 weapon state"));
        }
        self.update_player(&id, |player| player.view_angles = view_angles)?;
        let mut fired = false;
        if let Some(weapon) = pressed {
            self.select_weapon(actor, weapon)?;
        }
        let states = self.player_ref(&id).cloned().expect("player");
        if states.attack_held {
            fired = self.attack(actor, view_angles, seconds, water_level)?;
        }
        if states.continuous_firing && seconds >= states.next_weapon_frame {
            let _ = fired;
            self.weapon_frame(actor, seconds)?;
            return Ok(true);
        }
        Ok(fired)
    }

    /// Select a player's weapon.
    pub fn select_weapon(&mut self, actor: &OwnedActor, weapon: Q1Weapon) -> Result<bool, Q1Error> {
        let id = actor.id().clone();
        if self.player_ref(&id).is_none() || self.host.inventory.count(&id, &self.weapon_item(weapon)) == 0.0 {
            return Ok(false);
        }
        if !super::types::is_q1_base_weapon(weapon) && !self.registered_weapons.contains_key(&weapon) {
            return Ok(false);
        }
        let hook = self
            .registered_weapons
            .get(&weapon)
            .and_then(|definition| definition.available);
        if let Some(available) = hook {
            if !available(self, &id)? {
                return Ok(false);
            }
        }
        self.update_player(&id, |player| {
            player.weapon = weapon;
            player.primary_holstered = false;
            player.weapon_frame = 0;
            player.continuous_firing = false;
            player.weapon_animation_at = -1.0;
        })?;
        let view_model = self.weapon_model(weapon, Some(&id))?;
        self.host.emit(Q1Event::Weapon {
            player: id.clone(),
            weapon,
            view_model,
            frame: 0,
            punch: 0,
            attack: None,
        });
        Ok(true)
    }

    /// Bind the primary weapon handoff for a player.
    pub fn primary_weapon_handoff(&mut self, actor: &OwnedActor) -> Result<SourceWeaponHandoff, Q1Error> {
        if self.player_owned(actor.id()).is_none() {
            return Err(q1_error("Player has no Q1 weapon state"));
        }
        Ok(SourceWeaponHandoff::Immediate(Box::new(Q1PrimaryHandoff {
            actor: actor.clone(),
            provider: self.provider(),
            game: self as *mut Q1EntityServices,
        })))
    }

    /// Resolve the best available weapon for a player.
    pub fn choose_best(
        &mut self,
        actor: &OwnedActor,
        ammo_count: Option<&dyn Fn(&ItemId) -> f64>,
    ) -> Result<Q1Weapon, Q1Error> {
        super::weapons::best_weapon(self, actor, ammo_count)
    }

    /// Give a timed powerup to a player.
    pub fn give_powerup(&mut self, player: &ActorId, powerup: Q1Powerup, duration: f64) -> Result<(), Q1Error> {
        let owned = self
            .player_owned(player)
            .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
        let expires = f64::from((self.time + duration) as f32);
        self.update_player(player, |player| {
            player.powerups.insert(powerup, expires);
        })?;
        self.host.powerup(&owned, powerup, expires);
        self.host.emit(Q1Event::Powerup {
            player: player.clone(),
            powerup,
            expires,
        });
        Ok(())
    }

    /// Drop a shells backpack that fades after two minutes.
    pub fn drop_shells(&mut self, origin: Vec3) -> Result<(), Q1Error> {
        let id = self.create("item_backpack", None, None)?;
        self.set_origin(
            &id,
            vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -24.0,
                },
            ),
        )?;
        self.update_entity(&id, |entity| {
            entity.fields.insert(String::from("shells"), String::from("5"));
        })?;
        super::pickups::spawn_pickup(self, &id)?;
        self.schedule(&id, 120.0, "SUB_Remove")?;
        Ok(())
    }

    /// Snapshot presented entities in admission order.
    #[must_use]
    pub fn presentations(&self) -> Vec<Q1Presentation> {
        self.entity_order
            .iter()
            .filter_map(|id| {
                self.entities.get(id).map(|entity| Q1Presentation {
                    actor: entity.actor.id().clone(),
                    classname: entity.classname.clone(),
                    model: entity.model.clone(),
                    frame: entity.frame,
                    skin: entity.skin,
                    effects: entity.effects,
                    solid: entity.solid,
                    movement: entity.movement,
                    targetname: entity.targetname.clone(),
                    source_ordinal: entity.source_ordinal,
                })
            })
            .collect()
    }

    /// Check a water transition for an entity.
    pub fn check_water_transition(&mut self, id: &ActorId) -> Result<(), Q1Error> {
        let point = self
            .body(id)
            .map(|body| vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 8.0 }))?;
        let value = match self.host.contents(point) {
            Q1Contents::Water => -3,
            Q1Contents::Slime => -4,
            Q1Contents::Lava => -5,
            Q1Contents::Sky => -6,
            Q1Contents::Solid => -2,
            Q1Contents::Empty => -1,
        };
        let previous = self.entity_ref(id).map(|entity| entity.water_type).unwrap_or(0);
        let transition = q1_water_transition(previous, value);
        self.update_entity(id, |entity| {
            entity.water_type = transition.water_type;
            entity.water_level = transition.water_level;
        })?;
        if transition.splash && self.is_player(id) {
            self.sound(id, "misc/outwater.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
        }
        Ok(())
    }

    /// Run a spawn function for an entity.
    pub fn spawn_entity(&mut self, id: &ActorId, deathmatch: Option<i32>) -> Result<(), Q1Error> {
        let previous = self.spawn_options.clone();
        if let Some(deathmatch) = deathmatch {
            let mut options = self.configured_options.clone();
            options.deathmatch = deathmatch;
            self.spawn_options = Some(options);
        }
        let classname = self
            .entity_ref(id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        let spawn = self.spawners.get(&classname).copied();
        let result = (|| -> Result<(), Q1Error> {
            match spawn {
                Some(spawn) => spawn(self, id)?,
                None => super::spawns::spawn_map_actor(self, id)?,
            }
            if self.is_live(id) {
                self.link(id)?;
            }
            Ok(())
        })();
        self.spawn_options = previous;
        result
    }

    /// Initialize the weapon inventory for a player.
    pub fn initialize_weapon_inventory(&mut self, actor: &OwnedActor) -> Result<(), Q1Error> {
        let id = actor.id().clone();
        let mut entries = vec![
            InventoryEntry {
                item: super::types::weapon_item(Q1Weapon::Axe),
                count: 1.0,
                capacity: 1.0,
                count_policy: None,
            },
            InventoryEntry {
                item: String::from("q1:ammo/shells"),
                count: 0.0,
                capacity: 100.0,
                count_policy: None,
            },
            InventoryEntry {
                item: String::from("q1:ammo/nails"),
                count: 0.0,
                capacity: 200.0,
                count_policy: None,
            },
            InventoryEntry {
                item: String::from("q1:ammo/rockets"),
                count: 0.0,
                capacity: 100.0,
                count_policy: None,
            },
            InventoryEntry {
                item: String::from("q1:ammo/cells"),
                count: 0.0,
                capacity: 100.0,
                count_policy: None,
            },
        ];
        for weapon in WEAPONS.iter().skip(1) {
            entries.push(InventoryEntry {
                item: super::types::weapon_item(Q1Weapon::from(*weapon)),
                count: 0.0,
                capacity: 1.0,
                count_policy: None,
            });
        }
        self.host.inventory.create(actor, &entries)?;
        for item in ["q1:ammo/shells", "q1:ammo/nails", "q1:ammo/rockets", "q1:ammo/cells"] {
            let item = item.to_string();
            let count = self.host.inventory.count(actor.id(), &item);
            let capacity = self.inventory_capacity(&id, &item)?.unwrap_or(match item.as_str() {
                "q1:ammo/nails" => 200.0,
                _ => 100.0,
            });
            self.host.inventory.configure(
                actor,
                &InventoryEntry {
                    item,
                    count,
                    capacity,
                    count_policy: None,
                },
            )?;
        }
        Ok(())
    }

    /// Admit a player arsenal state.
    pub fn attach_player(&mut self, actor: &OwnedActor, options: &Q1AttachOptions) -> Result<ActorId, Q1Error> {
        let id = actor.id().clone();
        if self.players.contains_key(&id) {
            return Ok(id);
        }
        if options.initialize_inventory {
            self.initialize_weapon_inventory(actor)?;
        }
        let max_health = options.max_health.unwrap_or(100.0);
        let hostile_until = if self.options().deathmatch == 3 {
            self.time + 1.0
        } else {
            0.0
        };
        self.players.insert(
            id.clone(),
            super::types::Q1PlayerState {
                alpha: 0.0,
                scale: 0.0,
                actor: actor.clone(),
                weapon: options.weapon.unwrap_or(Q1Weapon::Axe),
                primary_holstered: false,
                attack_finished: 0.0,
                attack_held: false,
                jump_held: false,
                teleport_until: 0.0,
                weapon_frame: 0,
                weapon_animation_at: -1.0,
                weapon_animation_base: 0,
                continuous_firing: false,
                next_weapon_frame: 0.0,
                lightning_sound_at: 0.0,
                punch_angles: ZERO,
                nail_side: 1.0,
                max_health,
                mega_rot_at: -1.0,
                hostile_until,
                view_angles: ZERO,
                water_level: 0,
                air_finished: 12.0,
                drown_damage: 2.0,
                drown_at: 0.0,
                hazard_at: 0.0,
                auto_switch: Q1AutoSwitch::Always,
                powerups: HashMap::new(),
            },
        );
        self.player_order.push(id.clone());
        let hooks: Vec<Q1PlayerHook<Result<(), Q1Error>>> = self
            .player_extensions
            .iter()
            .filter_map(|(_, extension)| extension.attach)
            .collect();
        for hook in hooks {
            hook(self, &id)?;
        }
        if self.options().deathmatch == 3 {
            self.give_powerup(&id, Q1Powerup::Invulnerability, 3.0)?;
        }
        Ok(id)
    }

    /// Invoke a stored action by name.
    pub fn invoke_action(&mut self, id: &ActorId, name: &str) -> Result<(), Q1Error> {
        let handler = self.named.action_handler(name)?;
        handler(self, id)
    }

    /// Invoke a stored use callback by name.
    pub fn invoke_use(
        &mut self,
        id: &ActorId,
        name: &str,
        other: Option<&ActorId>,
        activator: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        let handler = self.named.use_handler(name)?;
        handler(self, id, other, activator)
    }

    /// Invoke a stored touch callback by name.
    pub fn invoke_touch(
        &mut self,
        id: &ActorId,
        other: &ActorId,
        normal: Option<Vec3>,
        surface: Option<&super::gameplay::TouchSurface>,
    ) -> Result<(), Q1Error> {
        let name = self.callback_store(id, Q1CallbackSlot::Touch)?;
        if let Some(name) = name {
            let handler = self.named.touch_handler(&name)?;
            let other = other.clone();
            let surface = surface.cloned();
            return handler(self, id, &other, normal, surface.as_ref());
        }
        Ok(())
    }

    /// Invoke a stored pain callback by name.
    pub fn invoke_pain(&mut self, id: &ActorId, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
        let name = self.callback_store(id, Q1CallbackSlot::Pain)?;
        if let Some(name) = name {
            let handler = self.named.pain_handler(&name)?;
            let attacker = attacker.cloned();
            return handler(self, id, attacker.as_ref(), damage);
        }
        Ok(())
    }

    /// Invoke a stored death callback by name.
    pub fn invoke_die(&mut self, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
        let name = self.callback_store(id, Q1CallbackSlot::Die)?;
        if let Some(name) = name {
            let handler = self.named.die_handler(&name)?;
            let attacker = attacker.cloned();
            return handler(self, id, attacker.as_ref());
        }
        Ok(())
    }

    /// Invoke a stored blocked callback by name.
    pub fn invoke_blocked(&mut self, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
        let name = self.callback_store(id, Q1CallbackSlot::Blocked)?;
        if let Some(name) = name {
            let handler = self.named.blocked_handler(&name)?;
            let other = other.clone();
            return handler(self, id, &other);
        }
        Ok(())
    }

    /// Invoke the stored path-end action.
    pub fn invoke_path_end(&mut self, id: &ActorId) -> Result<(), Q1Error> {
        let name = self.callback_store(id, Q1CallbackSlot::PathEnd)?;
        if let Some(name) = name {
            return self.invoke_action(id, &name);
        }
        Ok(())
    }

    /// Project a trajectory update through the entity's touch
    /// callback (`projectTrajectory`).
    pub fn project_trajectory(&mut self, id: &ActorId, update: &Q1TrajectoryUpdate) -> Result<(), Q1Error> {
        let name = self.callback_store(id, Q1CallbackSlot::Touch)?;
        if let Some(name) = name {
            let handler = self.named.trajectory_handler(&name)?;
            if let Some(trajectory) = handler {
                return trajectory(self, id, update);
            }
        }
        Ok(())
    }

    /// Drive a scheduled think (engine entry point).
    pub fn fire_think(&mut self, id: &ActorId, frame: &Q1ThinkFrame) -> Result<(), Q1Error> {
        self.time = frame.time.as_seconds_f64();
        self.frame_seconds = frame.elapsed.as_seconds_f64();
        let name = self.callback_store(id, Q1CallbackSlot::Think)?;
        self.update_entity(id, |entity| {
            entity.think = None;
            entity.next_think = -1.0;
        })?;
        if let Some(name) = name {
            return self.invoke_action(id, &name);
        }
        Ok(())
    }

    /// Drive a touch contact (engine entry point).
    pub fn fire_touch(&mut self, contact: &TouchContact) -> Result<(), Q1Error> {
        let id = contact.self_actor.id().clone();
        let other = contact.other.clone();
        let normal = contact.plane.as_ref().map(|plane| plane.normal);
        let surface = contact.surface.clone();
        self.invoke_touch(&id, &other, normal, surface.as_ref())
    }

    /// Drive a use invocation (engine and target entry point).
    pub fn fire_use(
        &mut self,
        target: &ActorId,
        other: Option<&ActorId>,
        activator: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        let name = self.callback_store(target, Q1CallbackSlot::Use)?;
        if let Some(name) = name {
            let handler = self.named.use_handler(&name)?;
            let other = other.cloned();
            let activator = activator.cloned();
            return handler(self, target, other.as_ref(), activator.as_ref());
        }
        Ok(())
    }

    /// Drive a pain reaction (engine entry point).
    pub fn fire_pain(&mut self, reaction: &PainReaction) -> Result<(), Q1Error> {
        let id = reaction.self_actor.id().clone();
        let attacker = reaction.attacker.clone();
        let damage = reaction.damage;
        self.invoke_pain(&id, attacker.as_ref(), damage)
    }

    /// Drive a death reaction (engine entry point).
    pub fn fire_die(&mut self, reaction: &DeathReaction) -> Result<(), Q1Error> {
        let id = reaction.self_actor.id().clone();
        let attacker = reaction.attacker.clone();
        let damage = reaction.damage;
        let health = self.health(&id);
        let owned = self.entity_ref(&id).map(|entity| entity.actor.clone());
        if let Some(owned) = owned {
            self.host.combat.set_health(&owned, damage.min(health))?;
        }
        let attacker_id = attacker.clone();
        self.update_entity(&id, |entity| {
            if let Some(monster) = entity.monster.as_mut() {
                monster.enemy = attacker_id;
            }
        })?;
        self.invoke_die(&id, attacker.as_ref())
    }
}

/// Primary weapon handoff bound to a live game.
///
/// The donor's handoff closures capture the game by garbage-collected
/// reference. The shared [`SourceWeaponHandoff`] API takes `&self`,
/// so the handoff holds a raw game pointer instead. Safety contract
/// for the engine: the handoff must not outlive the game, and handoff
/// methods must only run while the game is not otherwise borrowed
/// (sequential engine calls satisfy this; the derived borrows end
/// before each method returns).
pub struct Q1PrimaryHandoff {
    actor: OwnedActor,
    provider: ProviderId,
    game: *mut Q1EntityServices,
}

impl Q1PrimaryHandoff {
    fn game(&self) -> &Q1EntityServices {
        // SAFETY: engine upholds the documented lifetime and
        // sequential-use contract.
        unsafe { &*self.game }
    }

    #[allow(clippy::mut_from_ref)]
    fn game_mut(&self) -> &mut Q1EntityServices {
        // SAFETY: engine upholds the documented lifetime and
        // sequential-use contract.
        unsafe { &mut *self.game }
    }

    fn resolve(&self, item: &ItemId) -> Option<Q1Weapon> {
        let game = self.game_mut();
        let weapon = super::types::WEAPONS
            .iter()
            .map(|weapon| Q1Weapon::from(*weapon))
            .chain(game.registered_weapons.keys().copied())
            .find(|weapon| &game.weapon_item(*weapon) == item);
        match weapon {
            Some(weapon) => {
                let available = game
                    .weapon_available(self.actor.id(), weapon, Q1WeaponPurpose::Best, None)
                    .unwrap_or(false);
                available.then_some(weapon)
            }
            None => None,
        }
    }
}

impl crate::contract::SourceWeaponHandoffBase for Q1PrimaryHandoff {
    fn provider(&self) -> &ProviderId {
        &self.provider
    }

    fn accepts(&self, item: &ItemId) -> bool {
        self.resolve(item).is_some()
    }

    fn select(&self, item: &ItemId) -> bool {
        match self.resolve(item) {
            Some(weapon) => {
                let actor = self.actor.clone();
                self.game_mut().select_weapon(&actor, weapon).unwrap_or(false)
            }
            None => false,
        }
    }

    fn holster(&self) {
        let game = self.game_mut();
        let holstered = game
            .player_ref(self.actor.id())
            .is_some_and(|player| player.primary_holstered);
        if !holstered {
            let _ = game.update_player(self.actor.id(), |player| {
                player.primary_holstered = true;
                player.weapon_frame = 0;
                player.continuous_firing = false;
                player.weapon_animation_at = -1.0;
            });
        }
    }

    fn is_holstered(&self) -> bool {
        self.game()
            .player_ref(self.actor.id())
            .is_some_and(|player| player.primary_holstered)
    }
}

impl crate::contract::ImmediateWeaponHandoff for Q1PrimaryHandoff {
    fn resume(&self, item: Option<&ItemId>) -> bool {
        let game = self.game_mut();
        let id = self.actor.id().clone();
        let owned = self.actor.clone();
        let current = match game.player_ref(&id).map(|player| player.weapon) {
            Some(weapon) => weapon,
            None => return false,
        };
        let requested = match item {
            None => Some(current),
            Some(item) => {
                let weapon = super::types::WEAPONS
                    .iter()
                    .map(|weapon| Q1Weapon::from(*weapon))
                    .chain(game.registered_weapons.keys().copied())
                    .find(|weapon| &game.weapon_item(*weapon) == item);
                match weapon {
                    Some(weapon) => {
                        let available = game
                            .weapon_available(&id, weapon, Q1WeaponPurpose::Best, None)
                            .unwrap_or(false);
                        available.then_some(weapon)
                    }
                    None => None,
                }
            }
        };
        let weapon = match requested {
            Some(requested) => {
                let available = game
                    .weapon_available(&id, requested, Q1WeaponPurpose::Best, None)
                    .unwrap_or(false);
                if available {
                    requested
                } else {
                    match game.choose_best(&owned, None) {
                        Ok(weapon) => weapon,
                        Err(_) => return false,
                    }
                }
            }
            None => match game.choose_best(&owned, None) {
                Ok(weapon) => weapon,
                Err(_) => return false,
            },
        };
        let _ = game.select_weapon(&owned, weapon);
        let _ = game.update_player(&id, |player| player.primary_holstered = false);
        match item {
            None => true,
            Some(item) => game
                .player_ref(&id)
                .is_some_and(|player| &game.weapon_item(player.weapon) == item),
        }
    }
}

/// Ammunition item for a weapon (`ammoItem`).
#[must_use]
pub fn ammo_item(weapon: Q1Weapon) -> Option<ItemId> {
    match weapon {
        Q1Weapon::Shotgun => Some(String::from("q1:ammo/shells")),
        Q1Weapon::Supershotgun => Some(String::from("q1:ammo/shells")),
        Q1Weapon::Nailgun => Some(String::from("q1:ammo/nails")),
        Q1Weapon::Supernailgun => Some(String::from("q1:ammo/nails")),
        Q1Weapon::Grenadelauncher => Some(String::from("q1:ammo/rockets")),
        Q1Weapon::Rocketlauncher => Some(String::from("q1:ammo/rockets")),
        Q1Weapon::Lightning => Some(String::from("q1:ammo/cells")),
        _ => None,
    }
}

/// Normalize with donor mutable-vector-math semantics, returning the
/// magnitude. Zero-length input is preserved (`"preserve"`).
fn donor_normalize(direction: &mut Vec3, numeric: &NumericOps) -> f64 {
    let dot = numeric.add(
        numeric.add(
            numeric.mul(f64::from(direction.x), f64::from(direction.x)),
            numeric.mul(f64::from(direction.y), f64::from(direction.y)),
        ),
        numeric.mul(f64::from(direction.z), f64::from(direction.z)),
    );
    let length = numeric.sqrt(dot);
    let nonzero = match numeric.profile.arithmetic {
        Arithmetic::DonorBinary64(DonorSource::Q1 | DonorSource::Q2) => length != 0.0 && !length.is_nan(),
        _ => length != 0.0,
    };
    if nonzero {
        let scale = numeric.div(1.0, length);
        direction.x = numeric.store(numeric.mul(f64::from(direction.x), scale));
        direction.y = numeric.store(numeric.mul(f64::from(direction.y), scale));
        direction.z = numeric.store(numeric.mul(f64::from(direction.z), scale));
    }
    length
}

/// Decay view punch with donor `VectorNormalize`/`VectorScale`
/// semantics (`dropQ1Punch`).
#[must_use]
pub fn drop_q1_punch(angles: Vec3, elapsed: f64, numeric: &NumericOps) -> Vec3 {
    let mut direction = angles;
    let magnitude = donor_normalize(&mut direction, numeric);
    let remaining = numeric.sub(magnitude, numeric.mul(10.0, elapsed)).max(0.0);
    Vec3 {
        x: numeric.store(numeric.mul(f64::from(direction.x), remaining)),
        y: numeric.store(numeric.mul(f64::from(direction.y), remaining)),
        z: numeric.store(numeric.mul(f64::from(direction.z), remaining)),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::numeric::Q1_DONOR_PROFILE;

    use super::super::host::mock::mock_host;
    use super::super::types::{Q1Edition, Q1PrecacheProgram};
    use super::*;

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

    fn game() -> (
        Q1EntityServices,
        std::rc::Rc<std::cell::RefCell<super::super::host::mock::MockEvents>>,
    ) {
        let (host, events) = mock_host();
        (Q1EntityServices::new(host, options()).expect("game"), events)
    }

    #[test]
    fn creates_and_schedules_entities() {
        let (mut game, _) = game();
        let id = game.create("light", None, Some(7)).expect("create");
        assert_eq!(
            game.entity_ref(&id).map(|entity| entity.classname.as_str()),
            Some("light")
        );
        assert_eq!(game.find("missing"), Vec::new());
        game.update_entity(&id, |entity| entity.targetname = String::from("spot"))
            .expect("update");
        assert_eq!(game.find("spot"), vec![id.clone()]);
        game.schedule(&id, 0.5, "SUB_Remove").expect("schedule");
        assert_eq!(
            game.entity_ref(&id).and_then(|entity| entity.think.clone()),
            Some(String::from("SUB_Remove"))
        );
        game.cancel(&id);
        assert_eq!(game.entity_ref(&id).and_then(|entity| entity.think.clone()), None);
        assert!(game.schedule(&id, 0.1, "missing-action").is_err());
    }

    #[test]
    fn admits_players_and_fires_callbacks() {
        let (mut game, events) = game();
        let player = game.create("player", None, None).expect("player entity");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        assert!(game.is_player(&player));
        assert_eq!(
            game.player_ref(&player).map(|player| player.weapon),
            Some(Q1Weapon::Axe)
        );
        game.message_simple(Some(&player), "hello");
        assert_eq!(events.borrow().events.len(), 1);
        game.remove(&player).expect("remove");
        assert!(game.entity_ref(&player).is_none());
        assert!(game.player_ref(&player).is_none());
    }

    #[test]
    fn punch_decay_matches_donor_math() {
        let numeric = NumericOps::select(Q1_DONOR_PROFILE).expect("numeric");
        let decayed = drop_q1_punch(
            Vec3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
            0.5,
            &numeric,
        );
        assert!((f64::from(decayed.x) - 5.0).abs() < 1e-6);
        assert_eq!(drop_q1_punch(ZERO, 1.0, &numeric), ZERO);
    }

    #[test]
    fn primary_handoff_selects_owned_weapons() {
        let (mut game, _) = game();
        let player = game.create("player", None, None).expect("player entity");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        let handoff = game.primary_weapon_handoff(&owned).expect("handoff");
        let SourceWeaponHandoff::Immediate(handoff) = handoff else {
            panic!("immediate handoff");
        };
        assert!(handoff.accepts(&String::from("q1:weapon/axe")));
        assert!(!handoff.accepts(&String::from("q1:weapon/shotgun")));
        assert!(handoff.select(&String::from("q1:weapon/axe")));
        handoff.holster();
        assert!(handoff.is_holstered());
    }
}
