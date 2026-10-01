//! Q1 selected-arsenal bot weapon knowledge.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-q1-knowledge.ts`.
//!
//! Missing siblings: `SharedSimulation` (`runtime.ts`, runtime partition).
//! The [`Q1BotKnowledgeSimulation`] seam exposes exactly the donor's
//! `Pick<SharedSimulation, "inventory" | "combat" | "bodies" |
//! "q1WeaponSource">` surface; the runtime partition implements it
//! post-merge over `qa_content::q1` and `qa_world`.

use std::cell::RefCell;
use std::collections::HashMap;

use qa_bots::behavior::library::character::BotCharacterLibrary;
use qa_bots::behavior::library::weapons::{ProjectileInfo, WeaponInfo, DAMAGE_TYPE_IMPACT, DAMAGE_TYPE_RADIAL};
use qa_bots::behavior::q3::ai_definitions::BotInventory;
use qa_bots::behavior::q3::ai_state::BotState;
use qa_bots::behavior::q3::game_host::{BotArsenalKnowledge, BotObservedPickup, BotWeaponTactics};
use qa_content::contract::ItemId;
use qa_content::q1::foundation::types::{Q1BaseWeapon, Q1Weapon, WEAPONS};
use qa_content::q3::base::shared::definitions::WeaponState;
use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;
use qa_world::save::value::{SaveJson, SaveReader};

use super::bot_arsenal::{
    arsenal_activation_weapon, arsenal_aggression, arsenal_choose_weapon, arsenal_pickup_utility, arsenal_tactics,
    clear_decision_inventory, write_decision_inventory, BotArsenalBinding, BotArsenalError, BotWeaponCandidate,
    BotWeaponCandidateSupply,
};
use super::bot_knowledge_checkpoint::{BotKnowledgeSource, BotKnowledgeStore};

/// Combat projection used by the knowledge (donor `combat.read` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1BotCombat {
    /// Health.
    pub health: i32,
    /// Regular armor points (0 when armor kind is none).
    pub armor_points: i32,
}

/// Body bounds projection used by the knowledge (donor `bodies.read` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1BotBounds {
    /// Bounds minimum.
    pub min: Vec3,
    /// Bounds maximum.
    pub max: Vec3,
}

/// Q1 player projection used by the knowledge (donor `Q1PlayerState` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1BotPlayer {
    /// Nail side multiplier.
    pub nail_side: f64,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Selected weapon.
    pub weapon: Q1Weapon,
}

/// Simulation surface for Q1 bot knowledge.
pub trait Q1BotKnowledgeSimulation {
    /// Donor `simulation.q1WeaponSource() !== null`.
    fn has_q1_weapon_source(&self) -> bool;
    /// Donor `game.time`.
    fn q1_time(&self) -> f64;
    /// Donor `[...WEAPONS, ...game.registeredWeapons.keys()]` deduplicated.
    fn q1_weapons_in_order(&self) -> Vec<Q1Weapon>;
    /// Donor `game.registeredWeapons.has(weapon)`.
    fn q1_is_registered_weapon(&self, weapon: Q1Weapon) -> bool;
    /// Donor `game.weaponItem(weapon)`.
    fn q1_weapon_item(&self, weapon: Q1BaseWeapon) -> ItemId;
    /// Donor `game.weaponAmmo(weapon)`.
    fn q1_weapon_ammo(&self, weapon: Q1BaseWeapon) -> Option<ItemId>;
    /// Donor `game.weaponModel(weapon)`.
    fn q1_weapon_model(&self, weapon: Q1BaseWeapon) -> String;
    /// Donor `game.player(actor)`.
    fn q1_player(&self, actor: &ActorId) -> Option<Q1BotPlayer>;
    /// Donor `game.weaponAvailable(player, weapon)`.
    fn q1_weapon_available(&self, actor: &ActorId, weapon: Q1BaseWeapon) -> bool;
    /// Donor `game.nailSpeed(player, base)`.
    fn q1_nail_speed(&self, actor: &ActorId, base: f64) -> f64;
    /// Donor `game.powerupExpires(actor, "quad")`.
    fn q1_quad_expires(&self, actor: &ActorId) -> f64;
    /// Donor `simulation.inventory.count(actor, item)`.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32;
    /// Donor `simulation.combat.read(actor)`.
    fn combat_read(&self, actor: &ActorId) -> Option<Q1BotCombat>;
    /// Donor `simulation.bodies.read(actor)` bounds.
    fn body_bounds(&self, actor: &ActorId) -> Option<Q1BotBounds>;
}

impl<S: Q1BotKnowledgeSimulation> Q1BotKnowledgeSimulation for &S {
    fn has_q1_weapon_source(&self) -> bool {
        (*self).has_q1_weapon_source()
    }
    fn q1_time(&self) -> f64 {
        (*self).q1_time()
    }
    fn q1_weapons_in_order(&self) -> Vec<Q1Weapon> {
        (*self).q1_weapons_in_order()
    }
    fn q1_is_registered_weapon(&self, weapon: Q1Weapon) -> bool {
        (*self).q1_is_registered_weapon(weapon)
    }
    fn q1_weapon_item(&self, weapon: Q1BaseWeapon) -> ItemId {
        (*self).q1_weapon_item(weapon)
    }
    fn q1_weapon_ammo(&self, weapon: Q1BaseWeapon) -> Option<ItemId> {
        (*self).q1_weapon_ammo(weapon)
    }
    fn q1_weapon_model(&self, weapon: Q1BaseWeapon) -> String {
        (*self).q1_weapon_model(weapon)
    }
    fn q1_player(&self, actor: &ActorId) -> Option<Q1BotPlayer> {
        (*self).q1_player(actor)
    }
    fn q1_weapon_available(&self, actor: &ActorId, weapon: Q1BaseWeapon) -> bool {
        (*self).q1_weapon_available(actor, weapon)
    }
    fn q1_nail_speed(&self, actor: &ActorId, base: f64) -> f64 {
        (*self).q1_nail_speed(actor, base)
    }
    fn q1_quad_expires(&self, actor: &ActorId) -> f64 {
        (*self).q1_quad_expires(actor)
    }
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
        (*self).inventory_count(actor, item)
    }
    fn combat_read(&self, actor: &ActorId) -> Option<Q1BotCombat> {
        (*self).combat_read(actor)
    }
    fn body_bounds(&self, actor: &ActorId) -> Option<Q1BotBounds> {
        (*self).body_bounds(actor)
    }
}

fn base_weapon(weapon: Q1Weapon) -> Option<Q1BaseWeapon> {
    match weapon {
        Q1Weapon::Axe => Some(Q1BaseWeapon::Axe),
        Q1Weapon::Shotgun => Some(Q1BaseWeapon::Shotgun),
        Q1Weapon::Supershotgun => Some(Q1BaseWeapon::Supershotgun),
        Q1Weapon::Nailgun => Some(Q1BaseWeapon::Nailgun),
        Q1Weapon::Supernailgun => Some(Q1BaseWeapon::Supernailgun),
        Q1Weapon::Grenadelauncher => Some(Q1BaseWeapon::Grenadelauncher),
        Q1Weapon::Rocketlauncher => Some(Q1BaseWeapon::Rocketlauncher),
        Q1Weapon::Lightning => Some(Q1BaseWeapon::Lightning),
        _ => None,
    }
}

/// One shot row (donor `Shot`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Shot {
    damage: f64,
    count: i32,
    cycle: f64,
    ammo: i32,
    speed: f64,
    range: f64,
    radius: f64,
    spread_x: f64,
    spread_y: f64,
    forward: f64,
    side: f64,
}

/// progs106 weapons.qc/player.qc; continuous frames fire every 0.1 seconds.
fn shot(
    weapon: Q1BaseWeapon,
    nails: i32,
    shells: i32,
    nail_speed: f64,
    nail_side: f64,
) -> Result<Shot, BotArsenalError> {
    let row = |damage: f64,
               count: i32,
               cycle: f64,
               ammo: i32,
               speed: f64,
               range: f64,
               radius: f64,
               spread_x: f64,
               spread_y: f64,
               forward: f64,
               side: f64| Shot {
        damage,
        count,
        cycle,
        ammo,
        speed,
        range,
        radius,
        spread_x,
        spread_y,
        forward,
        side,
    };
    match weapon {
        Q1BaseWeapon::Axe => Ok(row(20.0, 1, 0.5, 0, 0.0, 64.0, 0.0, 0.0, 0.0, 0.0, 0.0)),
        Q1BaseWeapon::Shotgun => Ok(row(4.0, 6, 0.5, 1, 0.0, 2048.0, 0.0, 0.04, 0.04, 10.0, 0.0)),
        Q1BaseWeapon::Supershotgun => {
            if shells > 1 {
                Ok(row(4.0, 14, 0.7, 2, 0.0, 2048.0, 0.0, 0.14, 0.08, 10.0, 0.0))
            } else {
                Ok(row(4.0, 6, 0.7, 1, 0.0, 2048.0, 0.0, 0.04, 0.04, 10.0, 0.0))
            }
        }
        Q1BaseWeapon::Nailgun => Ok(row(
            9.0,
            1,
            0.1,
            1,
            nail_speed,
            nail_speed * 6.0,
            0.0,
            0.0,
            0.0,
            0.0,
            nail_side * 4.0,
        )),
        Q1BaseWeapon::Supernailgun => {
            if nails >= 2 {
                Ok(row(
                    18.0,
                    1,
                    0.1,
                    2,
                    nail_speed,
                    nail_speed * 6.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ))
            } else {
                Ok(row(
                    9.0,
                    1,
                    0.1,
                    1,
                    nail_speed,
                    nail_speed * 6.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    nail_side * 4.0,
                ))
            }
        }
        Q1BaseWeapon::Rocketlauncher => Ok(row(110.0, 1, 0.8, 1, 1000.0, 5000.0, 160.0, 0.0, 0.0, 8.0, 0.0)),
        Q1BaseWeapon::Lightning => Ok(row(30.0, 1, 0.1, 1, 0.0, 600.0, 0.0, 0.0, 0.0, 0.0, 0.0)),
        Q1BaseWeapon::Grenadelauncher => Err(BotArsenalError::GrenadeTrajectories),
    }
}

/// One weapon entry (donor `Entry`).
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    weapon: Q1BaseWeapon,
    slot: i32,
    item: ItemId,
    ammo: Option<ItemId>,
}

/// Q1 bot weapon knowledge (donor `createQ1BotKnowledge` result).
pub struct Q1BotKnowledge<S, F> {
    simulation: S,
    actor_for_client: F,
    entries: Vec<Entry>,
    uncovered_weapons: Vec<String>,
    actors: RefCell<HashMap<u32, ActorId>>,
    candidates: RefCell<Vec<BotWeaponCandidate>>,
}

impl<S: Q1BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> Q1BotKnowledge<S, F> {
    /// Create Q1 bot knowledge (donor `createQ1BotKnowledge`).
    pub fn new(simulation: S, actor_for_client: F) -> Result<Self, BotArsenalError> {
        if !simulation.has_q1_weapon_source() {
            return Err(BotArsenalError::MissingQ1Source);
        }
        let mut weapons = simulation.q1_weapons_in_order();
        if weapons.is_empty() {
            weapons = WEAPONS.iter().map(|weapon| Q1Weapon::from(*weapon)).collect();
        }
        let mut seen = std::collections::HashSet::new();
        let mut entries = Vec::new();
        let mut uncovered_weapons = Vec::new();
        for (index, weapon) in weapons.iter().enumerate() {
            if !seen.insert(*weapon) {
                continue;
            }
            let slot = index as i32 + 1;
            match base_weapon(*weapon) {
                Some(base) if !simulation.q1_is_registered_weapon(*weapon) && base != Q1BaseWeapon::Grenadelauncher => {
                    entries.push(Entry {
                        weapon: base,
                        slot,
                        item: simulation.q1_weapon_item(base),
                        ammo: simulation.q1_weapon_ammo(base),
                    });
                }
                _ => uncovered_weapons.push(weapon.as_str().to_string()),
            }
        }
        Ok(Self {
            simulation,
            actor_for_client,
            entries,
            uncovered_weapons,
            actors: RefCell::new(HashMap::new()),
            candidates: RefCell::new(Vec::new()),
        })
    }

    fn actor_for_handle(&self, handle: i32) -> Option<ActorId> {
        u32::try_from(handle)
            .ok()
            .and_then(|handle| self.actors.borrow().get(&handle).cloned())
    }

    fn usable(&self, actor: &ActorId, entry: &Entry) -> bool {
        self.simulation.q1_player(actor).is_some() && self.simulation.q1_weapon_available(actor, entry.weapon)
    }

    fn refresh(&self, handle: i32) -> Vec<BotWeaponCandidate> {
        let actor = self.actor_for_handle(handle);
        let player = actor.as_ref().and_then(|actor| self.simulation.q1_player(actor));
        let body = actor.as_ref().and_then(|actor| self.simulation.body_bounds(actor));
        let mut candidates = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let (nails, shells, nail_speed, nail_side) = match (actor.as_ref(), player.as_ref()) {
                (Some(actor), Some(player)) => (
                    self.simulation.inventory_count(actor, "q1:ammo/nails"),
                    self.simulation.inventory_count(actor, "q1:ammo/shells"),
                    self.simulation.q1_nail_speed(actor, 1000.0),
                    player.nail_side,
                ),
                _ => (2, 2, 1000.0, 1.0),
            };
            let Ok(values) = shot(entry.weapon, nails, shells, nail_speed, nail_side) else {
                continue;
            };
            let bullets = matches!(entry.weapon, Q1BaseWeapon::Shotgun | Q1BaseWeapon::Supershotgun);
            let muzzle_height = match (bullets, body) {
                (true, Some(body)) => body.min.z + (body.max.z - body.min.z) * 0.7,
                (true, None) => 15.2,
                (false, _) => 16.0,
            };
            let name = entry.weapon.as_str().to_string();
            let info = WeaponInfo {
                valid: true,
                number: entry.slot,
                name: name.clone(),
                model: self.simulation.q1_weapon_model(entry.weapon),
                level: 0,
                weapon_inventory_index: 64 + entry.slot,
                flags: 0,
                projectile: name.clone(),
                projectile_count: values.count,
                horizontal_spread: (values.spread_x.atan() * 180.0 / std::f64::consts::PI / 6.0) as f32,
                vertical_spread: (values.spread_y.atan() * 180.0 / std::f64::consts::PI / 6.0) as f32,
                speed: values.speed as f32,
                acceleration: 0.0,
                recoil: Vec3::default(),
                offset: Vec3 {
                    x: values.forward as f32,
                    y: values.side as f32,
                    z: muzzle_height - 22.0,
                },
                angle_offset: Vec3::default(),
                extra_z_velocity: 0.0,
                ammo_amount: values.ammo,
                ammo_inventory_index: 96 + entry.slot,
                activate: 0.0,
                reload: values.cycle as f32,
                spin_up: 0.0,
                spin_down: 0.0,
                projectile_info: ProjectileInfo {
                    name,
                    model: String::new(),
                    flags: 0,
                    gravity: 0.0,
                    damage: values.damage as f32,
                    radius: values.radius as f32,
                    visible_damage: 0.0,
                    damage_type: DAMAGE_TYPE_IMPACT | if values.radius > 0.0 { DAMAGE_TYPE_RADIAL } else { 0 },
                    health_increase: 0.0,
                    push: 0.0,
                    detonation: 0.0,
                    bounce: 0.0,
                    bounce_friction: 0.0,
                    bounce_stop: 0.0,
                },
            };
            candidates.push(BotWeaponCandidate {
                info,
                maximum_range: Some(values.range),
                melee: entry.weapon == Q1BaseWeapon::Axe,
                personality_role: None,
                supply: Some(BotWeaponCandidateSupply {
                    weapon: entry.item.clone(),
                    owned: actor
                        .as_ref()
                        .is_some_and(|actor| self.simulation.inventory_count(actor, &entry.item) > 0),
                    ammo: entry.ammo.as_ref().map(|ammo| (ammo.clone(), f64::from(values.ammo))),
                }),
            });
        }
        *self.candidates.borrow_mut() = candidates.clone();
        candidates
    }

    fn update_inventory_state(&self, state: &mut BotState) {
        let actor = (self.actor_for_client)(state.client);
        let handle = u32::try_from(state.ws).unwrap_or(0);
        match actor.clone() {
            None => {
                self.actors.borrow_mut().remove(&handle);
            }
            Some(actor) => {
                self.actors.borrow_mut().insert(handle, actor);
            }
        }
        clear_decision_inventory(state);
        let combat = actor.as_ref().and_then(|actor| self.simulation.combat_read(actor));
        write_decision_inventory(state, BotInventory::HEALTH, combat.map_or(0, |combat| combat.health));
        write_decision_inventory(
            state,
            BotInventory::ARMOR,
            combat.map_or(0, |combat| combat.armor_points),
        );
        let quad = actor
            .as_ref()
            .is_some_and(|actor| self.simulation.q1_quad_expires(actor) > self.simulation.q1_time());
        write_decision_inventory(state, BotInventory::QUAD, i32::from(quad));
        for entry in &self.entries {
            let usable = actor.as_ref().is_some_and(|actor| self.usable(actor, entry));
            write_decision_inventory(state, (64 + entry.slot) as usize, i32::from(usable));
            let ammo = match (actor.as_ref(), entry.ammo.as_ref()) {
                (Some(actor), Some(ammo)) => self.simulation.inventory_count(actor, ammo),
                _ => 0,
            };
            write_decision_inventory(state, (96 + entry.slot) as usize, ammo);
        }
    }
}

impl<S: Q1BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> BotArsenalKnowledge for Q1BotKnowledge<S, F> {
    fn pickup_utility(&self, _characters: &BotCharacterLibrary, state: &BotState, pickup: &BotObservedPickup) -> f32 {
        let candidates = self.refresh(state.ws);
        arsenal_pickup_utility(&candidates, &pickup.name, pickup.eligible)
    }

    fn choose_weapon(&self, _characters: &BotCharacterLibrary, state: &BotState) -> i32 {
        let candidates = self.refresh(state.ws);
        arsenal_choose_weapon(&candidates, state)
    }

    fn activation_weapon(&self, _characters: &BotCharacterLibrary, state: &BotState) -> i32 {
        let candidates = self.refresh(state.ws);
        arsenal_activation_weapon(&candidates, state)
    }

    fn tactics(&self, weapon: i32) -> BotWeaponTactics {
        arsenal_tactics(&self.candidates.borrow(), weapon)
    }

    fn aggression(&self, state: &BotState) -> f32 {
        arsenal_aggression(&self.candidates.borrow(), state)
    }

    fn update_inventory(&mut self, state: &mut BotState) {
        self.update_inventory_state(state);
    }
}

impl<S: Q1BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> BotArsenalBinding for Q1BotKnowledge<S, F> {
    fn uncovered_weapons(&self) -> &[String] {
        &self.uncovered_weapons
    }

    fn resolve_weapon(&self, client: i32, decision_slot: i32) -> Option<ItemId> {
        let actor = (self.actor_for_client)(client)?;
        let entry = self.entries.iter().find(|entry| entry.slot == decision_slot)?;
        self.usable(&actor, entry).then(|| entry.item.clone())
    }

    fn source_weapon(&self, client: i32) -> Result<i32, BotArsenalError> {
        let current = (self.actor_for_client)(client)
            .and_then(|actor| self.simulation.q1_player(&actor))
            .map(|player| player.weapon);
        Ok(self
            .entries
            .iter()
            .find(|entry| Some(entry.weapon) == current.and_then(base_weapon))
            .map_or(0, |entry| entry.slot))
    }

    fn source_weapon_state(&self, client: i32) -> Result<WeaponState, BotArsenalError> {
        let firing = (self.actor_for_client)(client).is_some_and(|actor| {
            self.simulation
                .q1_player(&actor)
                .is_some_and(|player| player.attack_finished > self.simulation.q1_time())
        });
        Ok(if firing {
            WeaponState::WeaponFiring
        } else {
            WeaponState::WeaponReady
        })
    }

    fn checkpoint_binding(&self) -> SaveJson {
        BotKnowledgeStore::new(BotKnowledgeSource::Q1, self.actors.borrow().clone()).checkpoint()
    }

    fn restore_binding(
        &mut self,
        reader: SaveReader<'_>,
        actor: &dyn Fn(SavedActorId) -> ActorId,
        weapon_handle: &dyn Fn(u32) -> i64,
    ) -> Result<(), BotArsenalError> {
        let mut store = BotKnowledgeStore::new(BotKnowledgeSource::Q1, HashMap::new());
        store.restore(reader, actor, weapon_handle)?;
        *self.actors.borrow_mut() = store.actors().clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FakeSimulation {
        time: f64,
        registered: Vec<Q1Weapon>,
        players: HashMap<ActorId, Q1BotPlayer>,
        counts: HashMap<(ActorId, String), i32>,
        combat: HashMap<ActorId, Q1BotCombat>,
        bounds: HashMap<ActorId, Q1BotBounds>,
    }

    impl Q1BotKnowledgeSimulation for FakeSimulation {
        fn has_q1_weapon_source(&self) -> bool {
            true
        }
        fn q1_time(&self) -> f64 {
            self.time
        }
        fn q1_weapons_in_order(&self) -> Vec<Q1Weapon> {
            let mut weapons: Vec<Q1Weapon> = WEAPONS.iter().map(|weapon| Q1Weapon::from(*weapon)).collect();
            weapons.extend(self.registered.iter().copied());
            weapons
        }
        fn q1_is_registered_weapon(&self, weapon: Q1Weapon) -> bool {
            self.registered.contains(&weapon)
        }
        fn q1_weapon_item(&self, weapon: Q1BaseWeapon) -> ItemId {
            format!("q1:weapon/{}", weapon.as_str())
        }
        fn q1_weapon_ammo(&self, weapon: Q1BaseWeapon) -> Option<ItemId> {
            match weapon {
                Q1BaseWeapon::Axe => None,
                Q1BaseWeapon::Shotgun | Q1BaseWeapon::Supershotgun => Some("q1:ammo/shells".to_string()),
                Q1BaseWeapon::Nailgun | Q1BaseWeapon::Supernailgun => Some("q1:ammo/nails".to_string()),
                Q1BaseWeapon::Grenadelauncher | Q1BaseWeapon::Rocketlauncher => Some("q1:ammo/rockets".to_string()),
                Q1BaseWeapon::Lightning => Some("q1:ammo/cells".to_string()),
            }
        }
        fn q1_weapon_model(&self, weapon: Q1BaseWeapon) -> String {
            format!("progs/v_{}.mdl", weapon.as_str())
        }
        fn q1_player(&self, actor: &ActorId) -> Option<Q1BotPlayer> {
            self.players.get(actor).copied()
        }
        fn q1_weapon_available(&self, actor: &ActorId, weapon: Q1BaseWeapon) -> bool {
            self.players.contains_key(actor) && weapon != Q1BaseWeapon::Lightning
        }
        fn q1_nail_speed(&self, _actor: &ActorId, base: f64) -> f64 {
            base
        }
        fn q1_quad_expires(&self, _actor: &ActorId) -> f64 {
            0.0
        }
        fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
            self.counts
                .get(&(actor.clone(), item.to_string()))
                .copied()
                .unwrap_or(0)
        }
        fn combat_read(&self, actor: &ActorId) -> Option<Q1BotCombat> {
            self.combat.get(actor).copied()
        }
        fn body_bounds(&self, actor: &ActorId) -> Option<Q1BotBounds> {
            self.bounds.get(actor).copied()
        }
    }

    fn actor_fn(actor: ActorId) -> impl Fn(i32) -> Option<ActorId> {
        move |_| Some(actor.clone())
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("test").unwrap()
    }

    fn fixture() -> (IdentityOwner, ActorId, FakeSimulation) {
        let owner = owner();
        let actor = owner.actor(1, 1);
        let mut counts = HashMap::new();
        counts.insert((actor.clone(), "q1:ammo/shells".to_string()), 8);
        counts.insert((actor.clone(), "q1:ammo/nails".to_string()), 4);
        counts.insert((actor.clone(), "q1:weapon/shotgun".to_string()), 1);
        let mut players = HashMap::new();
        players.insert(
            actor.clone(),
            Q1BotPlayer {
                nail_side: 1.0,
                attack_finished: 0.0,
                weapon: Q1Weapon::Shotgun,
            },
        );
        let mut combat = HashMap::new();
        combat.insert(
            actor.clone(),
            Q1BotCombat {
                health: 100,
                armor_points: 25,
            },
        );
        let mut bounds = HashMap::new();
        bounds.insert(
            actor.clone(),
            Q1BotBounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            },
        );
        (
            owner,
            actor,
            FakeSimulation {
                time: 10.0,
                registered: Vec::new(),
                players,
                counts,
                combat,
                bounds,
            },
        )
    }

    #[test]
    fn entries_skip_grenades_and_cover_base_roster() {
        let (_owner, actor, sim) = fixture();
        let knowledge = Q1BotKnowledge::new(sim, actor_fn(actor.clone())).unwrap();
        assert_eq!(knowledge.entries.len(), 7);
        assert_eq!(knowledge.uncovered_weapons(), ["grenadelauncher"]);
        assert_eq!(knowledge.entries[1].slot, 2);
        assert_eq!(knowledge.entries[1].weapon, Q1BaseWeapon::Shotgun);
    }

    #[test]
    fn update_inventory_maps_health_armor_and_ammo() {
        let (_owner, actor, sim) = fixture();
        let mut knowledge = Q1BotKnowledge::new(sim, actor_fn(actor.clone())).unwrap();
        let mut state = BotState::new(0);
        knowledge.update_inventory(&mut state);
        assert_eq!(state.inventory[BotInventory::HEALTH], 100);
        assert_eq!(state.inventory[BotInventory::ARMOR], 25);
        assert_eq!(state.inventory[BotInventory::QUAD], 0);
        assert_eq!(state.inventory[64 + 2], 1);
        assert_eq!(state.inventory[96 + 2], 8);
        assert_eq!(state.inventory[64 + 8], 0);
    }

    #[test]
    fn resolve_weapon_requires_usability() {
        let (_owner, actor, sim) = fixture();
        let knowledge = Q1BotKnowledge::new(sim, actor_fn(actor.clone())).unwrap();
        assert_eq!(knowledge.resolve_weapon(0, 2).as_deref(), Some("q1:weapon/shotgun"));
        assert_eq!(knowledge.resolve_weapon(0, 8), None);
        assert_eq!(knowledge.resolve_weapon(0, 99), None);
    }

    #[test]
    fn source_weapon_reports_current_slot() {
        let (_owner, actor, sim) = fixture();
        let knowledge = Q1BotKnowledge::new(sim, actor_fn(actor.clone())).unwrap();
        assert_eq!(knowledge.source_weapon(0).unwrap(), 2);
        assert_eq!(knowledge.source_weapon_state(0).unwrap(), WeaponState::WeaponReady);
    }

    #[test]
    fn checkpoint_round_trips_handles() {
        let (_owner, actor, sim) = fixture();
        let mut knowledge = Q1BotKnowledge::new(sim, actor_fn(actor.clone())).unwrap();
        let mut state = BotState::new(0);
        state.ws = 3;
        knowledge.update_inventory(&mut state);
        let checkpoint = knowledge.checkpoint_binding();
        let (_owner2, actor2, sim2) = fixture();
        let mut restored = Q1BotKnowledge::new(sim2, actor_fn(actor2.clone())).unwrap();
        restored
            .restore_binding(
                SaveReader::at(&checkpoint, "botKnowledge"),
                &|_| actor2.clone(),
                &|handle| i64::from(handle),
            )
            .unwrap();
        assert_eq!(restored.checkpoint_binding(), checkpoint);
    }

    #[test]
    fn shot_table_matches_donor_rows() {
        let axe = shot(Q1BaseWeapon::Axe, 2, 2, 1000.0, 1.0).unwrap();
        assert_eq!((axe.damage, axe.range, axe.cycle), (20.0, 64.0, 0.5));
        let double = shot(Q1BaseWeapon::Supershotgun, 2, 8, 1000.0, 1.0).unwrap();
        assert_eq!((double.count, double.ammo), (14, 2));
        let single = shot(Q1BaseWeapon::Supershotgun, 2, 1, 1000.0, 1.0).unwrap();
        assert_eq!((single.count, single.ammo), (6, 1));
        assert!(shot(Q1BaseWeapon::Grenadelauncher, 2, 2, 1000.0, 1.0).is_err());
    }
}
