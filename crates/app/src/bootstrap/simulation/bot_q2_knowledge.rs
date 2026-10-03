//! Q2 selected-arsenal bot weapon knowledge.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bot-q2-knowledge.ts`.
//!
//! Sibling homes: [`SharedSimulation`](super::runtime::SharedSimulation)
//! (`simulation/runtime.ts` port) is the live shared simulation. The
//! [`Q2BotKnowledgeSimulation`] seam stays as the narrow interface this
//! module needs (the donor `Pick<SharedSimulation, "q2WeaponSource" |
//! "inventory" | "combat">` surface); only test doubles implement it.

use std::cell::RefCell;
use std::collections::HashMap;

use qa_bots::behavior::library::character::BotCharacterLibrary;
use qa_bots::behavior::library::weapons::{ProjectileInfo, WeaponInfo, DAMAGE_TYPE_IMPACT, DAMAGE_TYPE_RADIAL};
use qa_bots::behavior::q3::ai_definitions::BotInventory;
use qa_bots::behavior::q3::ai_state::BotState;
use qa_bots::behavior::q3::game_host::{BotArsenalKnowledge, BotObservedPickup, BotWeaponTactics};
use qa_content::contract::ItemId;
use qa_content::q2::foundation::weapons::types::{Q2WeaponDefinition, Q2WeaponPhase};
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
pub struct Q2BotCombat {
    /// Health.
    pub health: i32,
    /// Regular armor points (0 when armor kind is none).
    pub armor_points: i32,
}

/// Weapon animation projection (donor `source.weapons.states` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BotWeaponState {
    /// Animation phase.
    pub phase: Q2WeaponPhase,
    /// Current weapon name.
    pub weapon: Option<String>,
}

/// Simulation surface for Q2 bot knowledge.
pub trait Q2BotKnowledgeSimulation {
    /// Donor `simulation.q2WeaponSource() !== null`.
    fn has_q2_weapon_source(&self) -> bool;
    /// Donor `source.weapons.registeredDefinitions()`.
    fn q2_weapon_definitions(&self) -> Vec<Q2WeaponDefinition>;
    /// Donor `source.game.options.edition === "rerelease"`.
    fn q2_edition_is_rerelease(&self) -> bool;
    /// Donor `source.game.options.mode === "deathmatch"`.
    fn q2_mode_is_deathmatch(&self) -> bool;
    /// Donor `source.weapons.states.get(actor)`.
    fn q2_weapon_state(&self, actor: &ActorId) -> Option<Q2BotWeaponState>;
    /// Donor `simulation.inventory.count(actor, item)`.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32;
    /// Donor `simulation.combat.read(actor)`.
    fn combat_read(&self, actor: &ActorId) -> Option<Q2BotCombat>;
}

impl<S: Q2BotKnowledgeSimulation> Q2BotKnowledgeSimulation for &S {
    fn has_q2_weapon_source(&self) -> bool {
        (*self).has_q2_weapon_source()
    }
    fn q2_weapon_definitions(&self) -> Vec<Q2WeaponDefinition> {
        (*self).q2_weapon_definitions()
    }
    fn q2_edition_is_rerelease(&self) -> bool {
        (*self).q2_edition_is_rerelease()
    }
    fn q2_mode_is_deathmatch(&self) -> bool {
        (*self).q2_mode_is_deathmatch()
    }
    fn q2_weapon_state(&self, actor: &ActorId) -> Option<Q2BotWeaponState> {
        (*self).q2_weapon_state(actor)
    }
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
        (*self).inventory_count(actor, item)
    }
    fn combat_read(&self, actor: &ActorId) -> Option<Q2BotCombat> {
        (*self).combat_read(actor)
    }
}

/// Ballistics row (donor `Ballistics`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ballistics {
    damage: f64,
    speed: f64,
    range: f64,
    radius: f64,
    horizontal: f64,
    vertical: f64,
    count: i32,
    cycle: f64,
    gravity: f64,
    bounce: f64,
    detonation: f64,
    lift: f64,
}

/// p_weapon callbacks provide these unpowered shot values; random damage uses
/// its arithmetic mean.
fn ballistics(name: &str, rerelease: bool, deathmatch: bool) -> Option<Ballistics> {
    let shot =
        |damage: f64, speed: f64, range: f64, radius: f64, horizontal: f64, vertical: f64, count: i32, cycle: f64| {
            Ballistics {
                damage,
                speed,
                range,
                radius,
                horizontal,
                vertical,
                count,
                cycle,
                gravity: 0.0,
                bounce: 0.0,
                detonation: 0.0,
                lift: 0.0,
            }
        };
    match name {
        "blaster" => Some(shot(
            if rerelease || deathmatch { 15.0 } else { 10.0 },
            if rerelease { 1500.0 } else { 1000.0 },
            if rerelease { 3000.0 } else { 2000.0 },
            0.0,
            0.0,
            0.0,
            1,
            0.5,
        )),
        "shotgun" => Some(shot(4.0, 0.0, 8192.0, 0.0, 500.0, 500.0, 12, 1.2)),
        "supershotgun" => Some(shot(6.0, 0.0, 8192.0, 0.0, 1000.0, 500.0, 20, 1.2)),
        "machinegun" => Some(shot(8.0, 0.0, 8192.0, 0.0, 300.0, 500.0, 1, 0.1)),
        "chaingun" => Some(shot(
            if deathmatch { 6.0 } else { 8.0 },
            0.0,
            8192.0,
            0.0,
            300.0,
            500.0,
            3,
            0.1,
        )),
        "grenadelauncher" => Some(Ballistics {
            gravity: 1.0,
            bounce: 1.5,
            detonation: 2.5,
            lift: 200.0,
            ..shot(120.0, 600.0, 1500.0, 160.0, 0.0, 0.0, 1, 1.2)
        }),
        "rocketlauncher" => Some(shot(109.5, 650.0, 8000.0, 120.0, 0.0, 0.0, 1, 0.9)),
        "hyperblaster" => Some(shot(
            if deathmatch { 15.0 } else { 20.0 },
            1000.0,
            2000.0,
            0.0,
            0.0,
            0.0,
            1,
            0.1,
        )),
        "railgun" => Some(shot(
            if deathmatch {
                100.0
            } else if rerelease {
                125.0
            } else {
                150.0
            },
            0.0,
            8192.0,
            0.0,
            0.0,
            0.0,
            1,
            1.6,
        )),
        "ionripper" => Some(shot(
            if deathmatch { 30.0 } else { 50.0 },
            500.0,
            1500.0,
            0.0,
            (std::f64::consts::PI / 180.0).tan() * 8192.0,
            0.0,
            1,
            0.3,
        )),
        "phalanx" => Some(shot(
            74.5,
            725.0,
            8000.0,
            120.0,
            (1.5 * std::f64::consts::PI / 180.0).tan() * 8192.0,
            0.0,
            2,
            1.6,
        )),
        "etf_rifle" => Some(shot(
            10.0,
            if rerelease { 1150.0 } else { 750.0 },
            8000.0,
            0.0,
            0.0,
            0.0,
            1,
            0.1,
        )),
        "heatbeam" => Some(shot(15.0, 0.0, 8192.0, 0.0, 0.0, 0.0, 1, 0.1)),
        _ => None,
    }
}

fn weapon_info(definition: &Q2WeaponDefinition, slot: i32, shot: Ballistics, rerelease: bool) -> WeaponInfo {
    let name = definition.name.as_str();
    let offset = if name == "blaster" || name == "hyperblaster" {
        Vec3 {
            x: 24.0,
            y: 8.0,
            z: -8.0,
        }
    } else if name == "ionripper" {
        Vec3 {
            x: 16.0,
            y: 7.0,
            z: -8.0,
        }
    } else if name == "etf_rifle" {
        Vec3 {
            x: 15.0,
            y: 8.0,
            z: -8.0,
        }
    } else if name == "heatbeam" {
        Vec3 {
            x: 7.0,
            y: 2.0,
            z: -3.0,
        }
    } else if name == "railgun" {
        Vec3 {
            x: 0.0,
            y: 7.0,
            z: -8.0,
        }
    } else if name == "rocketlauncher" {
        Vec3 {
            x: 8.0,
            y: 8.0,
            z: -8.0,
        }
    } else if name == "grenadelauncher" {
        Vec3 {
            x: 8.0,
            y: if rerelease { 0.0 } else { 8.0 },
            z: -8.0,
        }
    } else {
        Vec3 {
            x: 0.0,
            y: if name == "phalanx" {
                8.0
            } else if rerelease {
                0.0
            } else {
                8.0
            },
            z: -8.0,
        }
    };
    WeaponInfo {
        valid: true,
        number: slot,
        name: definition.name.clone(),
        model: definition.view_model.clone(),
        level: 0,
        weapon_inventory_index: 64 + slot,
        flags: 0,
        projectile: definition.name.clone(),
        projectile_count: shot.count,
        horizontal_spread: ((shot.horizontal / 8192.0).atan() * 180.0 / std::f64::consts::PI / 6.0) as f32,
        vertical_spread: ((shot.vertical / 8192.0).atan() * 180.0 / std::f64::consts::PI / 6.0) as f32,
        speed: shot.speed as f32,
        acceleration: 0.0,
        recoil: Vec3::default(),
        offset,
        angle_offset: Vec3::default(),
        extra_z_velocity: shot.lift as f32,
        ammo_amount: definition.quantity,
        ammo_inventory_index: 96 + slot,
        activate: definition.activate_last as f32 * 0.1,
        reload: shot.cycle as f32,
        spin_up: if name == "chaingun" { 1.0 } else { 0.0 },
        spin_down: 0.0,
        projectile_info: ProjectileInfo {
            name: definition.name.clone(),
            model: String::new(),
            flags: 0,
            gravity: shot.gravity as f32,
            damage: shot.damage as f32,
            radius: shot.radius as f32,
            visible_damage: 0.0,
            damage_type: DAMAGE_TYPE_IMPACT | if shot.radius > 0.0 { DAMAGE_TYPE_RADIAL } else { 0 },
            health_increase: 0.0,
            push: 0.0,
            detonation: shot.detonation as f32,
            bounce: shot.bounce as f32,
            bounce_friction: 0.0,
            bounce_stop: 0.0,
        },
    }
}

/// One weapon entry (donor `Entry`).
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    slot: i32,
    definition: Q2WeaponDefinition,
    ballistics: Ballistics,
    info: WeaponInfo,
}

/// Q2 bot weapon knowledge (donor `createQ2BotKnowledge` result).
pub struct Q2BotKnowledge<S, F> {
    simulation: S,
    actor_for_client: F,
    entries: Vec<Entry>,
    uncovered_weapons: Vec<String>,
    actors: RefCell<HashMap<u32, ActorId>>,
    candidates: RefCell<Vec<BotWeaponCandidate>>,
}

impl<S: Q2BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> Q2BotKnowledge<S, F> {
    /// Create Q2 bot knowledge (donor `createQ2BotKnowledge`).
    pub fn new(simulation: S, actor_for_client: F) -> Result<Self, BotArsenalError> {
        if !simulation.has_q2_weapon_source() {
            return Err(BotArsenalError::MissingQ2Source);
        }
        let rerelease = simulation.q2_edition_is_rerelease();
        let deathmatch = simulation.q2_mode_is_deathmatch();
        let mut entries = Vec::new();
        let mut uncovered_weapons = Vec::new();
        for (index, definition) in simulation.q2_weapon_definitions().iter().enumerate() {
            let shot = ballistics(&definition.name, rerelease, deathmatch);
            match shot {
                Some(shot) if shot.gravity <= 0.0 => {
                    let slot = index as i32 + 1;
                    entries.push(Entry {
                        slot,
                        definition: definition.clone(),
                        ballistics: shot,
                        info: weapon_info(definition, slot, shot, rerelease),
                    });
                }
                _ => uncovered_weapons.push(definition.name.clone()),
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

    fn can_use(&self, actor: &ActorId, entry: &Entry) -> bool {
        self.simulation.inventory_count(actor, &entry.definition.item) > 0
            && entry
                .definition
                .ammo
                .as_ref()
                .is_none_or(|ammo| self.simulation.inventory_count(actor, ammo) >= entry.definition.quantity)
    }

    fn refresh(&self, handle: i32) -> Vec<BotWeaponCandidate> {
        let actor = self.actor_for_handle(handle);
        let candidates = self
            .entries
            .iter()
            .map(|entry| BotWeaponCandidate {
                info: entry.info.clone(),
                maximum_range: Some(entry.ballistics.range),
                melee: false,
                personality_role: None,
                supply: Some(BotWeaponCandidateSupply {
                    weapon: entry.definition.item.clone(),
                    owned: actor
                        .as_ref()
                        .is_some_and(|actor| self.simulation.inventory_count(actor, &entry.definition.item) > 0),
                    ammo: entry
                        .definition
                        .ammo
                        .as_ref()
                        .map(|ammo| (ammo.clone(), f64::from(entry.definition.quantity))),
                }),
            })
            .collect::<Vec<_>>();
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
        for entry in &self.entries {
            let owned = actor
                .as_ref()
                .is_some_and(|actor| self.simulation.inventory_count(actor, &entry.definition.item) > 0);
            write_decision_inventory(state, entry.info.weapon_inventory_index as usize, i32::from(owned));
            let ammo = match (actor.as_ref(), entry.definition.ammo.as_ref()) {
                (Some(actor), Some(ammo)) => self.simulation.inventory_count(actor, ammo),
                _ => 0,
            };
            write_decision_inventory(state, entry.info.ammo_inventory_index as usize, ammo);
        }
    }
}

impl<S: Q2BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> BotArsenalKnowledge for Q2BotKnowledge<S, F> {
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

impl<S: Q2BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> BotArsenalBinding for Q2BotKnowledge<S, F> {
    fn uncovered_weapons(&self) -> &[String] {
        &self.uncovered_weapons
    }

    fn resolve_weapon(&self, client: i32, decision_slot: i32) -> Option<ItemId> {
        let actor = (self.actor_for_client)(client)?;
        let entry = self.entries.iter().find(|entry| entry.slot == decision_slot)?;
        self.can_use(&actor, entry).then(|| entry.definition.item.clone())
    }

    fn source_weapon(&self, client: i32) -> Result<i32, BotArsenalError> {
        let current = (self.actor_for_client)(client)
            .and_then(|actor| self.simulation.q2_weapon_state(&actor))
            .and_then(|state| state.weapon);
        Ok(self
            .entries
            .iter()
            .find(|entry| Some(entry.definition.name.clone()) == current)
            .map_or(0, |entry| entry.slot))
    }

    fn source_weapon_state(&self, client: i32) -> Result<WeaponState, BotArsenalError> {
        let phase = (self.actor_for_client)(client)
            .and_then(|actor| self.simulation.q2_weapon_state(&actor))
            .map(|state| state.phase);
        Ok(match phase {
            Some(Q2WeaponPhase::Activating) => WeaponState::WeaponRaising,
            Some(Q2WeaponPhase::Dropping) => WeaponState::WeaponDropping,
            Some(Q2WeaponPhase::Firing) => WeaponState::WeaponFiring,
            _ => WeaponState::WeaponReady,
        })
    }

    fn checkpoint_binding(&self) -> SaveJson {
        BotKnowledgeStore::new(BotKnowledgeSource::Q2, self.actors.borrow().clone()).checkpoint()
    }

    fn restore_binding(
        &mut self,
        reader: SaveReader<'_>,
        actor: &dyn Fn(SavedActorId) -> ActorId,
        weapon_handle: &dyn Fn(u32) -> i64,
    ) -> Result<(), BotArsenalError> {
        let mut store = BotKnowledgeStore::new(BotKnowledgeSource::Q2, HashMap::new());
        store.restore(reader, actor, weapon_handle)?;
        *self.actors.borrow_mut() = store.actors().clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    fn definition(name: &str, item: &str, ammo: Option<&str>, quantity: i32) -> Q2WeaponDefinition {
        Q2WeaponDefinition {
            name: name.to_string(),
            item: item.to_string(),
            classname: format!("weapon_{name}"),
            ammo: ammo.map(str::to_string),
            quantity,
            warning: 0,
            view_model: format!("models/weapons/v_{name}/tris.md2"),
            world_model: String::new(),
            player_model: 0,
            activate_last: 4,
            fire_last: 8,
            idle_last: 10,
            deactivate_last: 2,
            pauses: Vec::new(),
            fires: Vec::new(),
            repeating: false,
        }
    }

    struct FakeSimulation {
        definitions: Vec<Q2WeaponDefinition>,
        rerelease: bool,
        deathmatch: bool,
        states: HashMap<ActorId, Q2BotWeaponState>,
        counts: HashMap<(ActorId, String), i32>,
        combat: HashMap<ActorId, Q2BotCombat>,
    }

    impl Q2BotKnowledgeSimulation for FakeSimulation {
        fn has_q2_weapon_source(&self) -> bool {
            true
        }
        fn q2_weapon_definitions(&self) -> Vec<Q2WeaponDefinition> {
            self.definitions.clone()
        }
        fn q2_edition_is_rerelease(&self) -> bool {
            self.rerelease
        }
        fn q2_mode_is_deathmatch(&self) -> bool {
            self.deathmatch
        }
        fn q2_weapon_state(&self, actor: &ActorId) -> Option<Q2BotWeaponState> {
            self.states.get(actor).cloned()
        }
        fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
            self.counts
                .get(&(actor.clone(), item.to_string()))
                .copied()
                .unwrap_or(0)
        }
        fn combat_read(&self, actor: &ActorId) -> Option<Q2BotCombat> {
            self.combat.get(actor).copied()
        }
    }

    fn actor_fn(actor: ActorId) -> impl Fn(i32) -> Option<ActorId> {
        move |_| Some(actor.clone())
    }

    fn fixture() -> (IdentityOwner, ActorId, FakeSimulation) {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut counts = HashMap::new();
        counts.insert((actor.clone(), "q2:weapon/shotgun".to_string()), 1);
        counts.insert((actor.clone(), "q2:ammo/shells".to_string()), 12);
        let mut states = HashMap::new();
        states.insert(
            actor.clone(),
            Q2BotWeaponState {
                phase: Q2WeaponPhase::Firing,
                weapon: Some("shotgun".to_string()),
            },
        );
        let mut combat = HashMap::new();
        combat.insert(
            actor.clone(),
            Q2BotCombat {
                health: 90,
                armor_points: 0,
            },
        );
        (
            owner,
            actor,
            FakeSimulation {
                definitions: vec![
                    definition("blaster", "q2:weapon/blaster", None, 0),
                    definition("shotgun", "q2:weapon/shotgun", Some("q2:ammo/shells"), 1),
                    definition("grenadelauncher", "q2:weapon/grenades", Some("q2:ammo/grenades"), 1),
                    definition("mystery", "q2:weapon/mystery", None, 0),
                ],
                rerelease: false,
                deathmatch: true,
                states,
                counts,
                combat,
            },
        )
    }

    #[test]
    fn entries_cover_hitscan_and_skip_gravity_and_unknown() {
        let (_owner, actor, sim) = fixture();
        let knowledge = Q2BotKnowledge::new(sim, actor_fn(actor)).unwrap();
        assert_eq!(knowledge.entries.len(), 2);
        assert_eq!(knowledge.uncovered_weapons(), ["grenadelauncher", "mystery"]);
        assert_eq!(knowledge.entries[1].slot, 2);
    }

    #[test]
    fn ballistics_match_donor_deathmatch_rows() {
        let blaster = ballistics("blaster", false, true).unwrap();
        assert_eq!((blaster.damage, blaster.speed, blaster.range), (15.0, 1000.0, 2000.0));
        let blaster = ballistics("blaster", true, false).unwrap();
        assert_eq!((blaster.damage, blaster.speed, blaster.range), (15.0, 1500.0, 3000.0));
        let railgun = ballistics("railgun", false, true).unwrap();
        assert_eq!(railgun.damage, 100.0);
        let grenades = ballistics("grenadelauncher", false, false).unwrap();
        assert_eq!((grenades.gravity, grenades.lift), (1.0, 200.0));
        assert_eq!(ballistics("mystery", false, false), None);
    }

    #[test]
    fn update_inventory_maps_health_and_shells() {
        let (_owner, actor, sim) = fixture();
        let mut knowledge = Q2BotKnowledge::new(sim, actor_fn(actor)).unwrap();
        let mut state = BotState::new(0);
        knowledge.update_inventory(&mut state);
        assert_eq!(state.inventory[BotInventory::HEALTH], 90);
        assert_eq!(state.inventory[65], 0);
        assert_eq!(state.inventory[66], 1);
        assert_eq!(state.inventory[98], 12);
    }

    #[test]
    fn source_state_maps_firing_phase() {
        let (_owner, actor, sim) = fixture();
        let knowledge = Q2BotKnowledge::new(sim, actor_fn(actor)).unwrap();
        assert_eq!(knowledge.source_weapon(0).unwrap(), 2);
        assert_eq!(knowledge.source_weapon_state(0).unwrap(), WeaponState::WeaponFiring);
        assert_eq!(knowledge.resolve_weapon(0, 2).as_deref(), Some("q2:weapon/shotgun"));
        assert_eq!(knowledge.resolve_weapon(0, 1), None);
    }
}
