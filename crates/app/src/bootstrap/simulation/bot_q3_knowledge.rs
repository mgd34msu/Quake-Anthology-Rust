//! Q3 selected-arsenal bot weapon knowledge.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bot-q3-knowledge.ts`.
//!
//! Missing siblings: `SharedSimulation` (`runtime.ts`, runtime partition).
//! The [`Q3BotKnowledgeSimulation`] seam exposes exactly the donor's
//! `Pick<SharedSimulation, "selectedQ3WeaponSource" | "inventory" |
//! "combat">` surface; the runtime partition implements it post-merge over
//! the selected Q3 source and `qa_world`.
//!
//! The donor resolves weapon infos through the bot library on every
//! candidates call. The Rust [`WeaponAi`] owns a single selected config, so
//! the valid infos are resolved once at construction; per-handle configs
//! unify post-merge when `qa_bots` ports them.
//!
//! `update_q3_bot_weapon_inventory` below mirrors
//! `updateQ3BotWeaponInventory` from donor
//! Port of Quake-Anthology-TS `src/bots/behavior/q3/ai-combat.ts` (canonical home:
//! `qa_bots::behavior::q3::ai_combat`); unify post-merge. The donor skips
//! missionpack slots for baseq3 products; the dialect zeroes the inventory
//! first, which makes the skip unobservable, and the Rust [`BotState`]
//! carries no product.

use std::cell::RefCell;
use std::collections::HashMap;

use qa_bots::behavior::library::character::BotCharacterLibrary;
use qa_bots::behavior::library::weapons::{WeaponAi, WeaponInfo};
use qa_bots::behavior::q3::ai_definitions::BotInventory;
use qa_bots::behavior::q3::ai_state::BotState;
use qa_bots::behavior::q3::arsenal_knowledge::Weapon as WeaponRole;
use qa_bots::behavior::q3::game_host::{BotArsenalKnowledge, BotObservedPickup, BotWeaponTactics};
use qa_content::contract::ItemId;
use qa_content::q3::base::shared::definitions::{Weapon as ContentWeapon, WeaponState};
use qa_content::q3::foundation::arsenal::{q3_weapon_item, Q3_WEAPON_ITEMS};
use qa_core::identity::{ActorId, SavedActorId};
use qa_world::movement::q3::constants::weapon_state as weapon_phase;
use qa_world::save::value::{SaveJson, SaveReader};

use super::bot_arsenal::{
    arsenal_activation_weapon, arsenal_aggression, arsenal_choose_weapon, arsenal_pickup_utility, arsenal_tactics,
    clear_decision_inventory, write_decision_inventory, BotArsenalBinding, BotArsenalError, BotWeaponCandidate,
    BotWeaponCandidateSupply,
};
use super::bot_knowledge_checkpoint::{BotKnowledgeSource, BotKnowledgeStore};

/// Combat projection used by the knowledge (donor `combat.read` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BotCombat {
    /// Health.
    pub health: i32,
    /// Regular armor points (0 when armor kind is none).
    pub armor_points: i32,
}

/// Selected Q3 arsenal projection (donor `source.read(actor).state` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BotArsenalView {
    /// Donor `arsenal.state.kind === "q3"`.
    pub is_q3: bool,
    /// Donor `state.sourceWeapon`.
    pub source_weapon: i32,
    /// Donor `state.state` phase number.
    pub phase: i32,
}

/// Simulation surface for Q3 bot knowledge.
pub trait Q3BotKnowledgeSimulation {
    /// Donor `simulation.selectedQ3WeaponSource() !== null`.
    fn has_selected_q3_weapon_source(&self) -> bool;
    /// Donor `source.has(actor)`.
    fn q3_source_has(&self, actor: &ActorId) -> bool;
    /// Donor `source.read(actor)` state projection.
    fn q3_arsenal(&self, actor: &ActorId) -> Option<Q3BotArsenalView>;
    /// Donor `simulation.inventory.count(actor, item)`.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32;
    /// Donor `simulation.combat.read(actor)`.
    fn combat_read(&self, actor: &ActorId) -> Option<Q3BotCombat>;
}

impl<S: Q3BotKnowledgeSimulation> Q3BotKnowledgeSimulation for &S {
    fn has_selected_q3_weapon_source(&self) -> bool {
        (*self).has_selected_q3_weapon_source()
    }
    fn q3_source_has(&self, actor: &ActorId) -> bool {
        (*self).q3_source_has(actor)
    }
    fn q3_arsenal(&self, actor: &ActorId) -> Option<Q3BotArsenalView> {
        (*self).q3_arsenal(actor)
    }
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
        (*self).inventory_count(actor, item)
    }
    fn combat_read(&self, actor: &ActorId) -> Option<Q3BotCombat> {
        (*self).combat_read(actor)
    }
}

/// Mirror of donor `updateQ3BotWeaponInventory` (see module docs).
fn update_q3_bot_weapon_inventory(state: &mut BotState, owned: &dyn Fn(i32) -> bool, ammo: &dyn Fn(i32) -> i32) {
    const WEAPON_SLOTS: [(usize, i32); 13] = [
        (BotInventory::GAUNTLET, WeaponRole::GAUNTLET),
        (BotInventory::SHOTGUN, WeaponRole::SHOTGUN),
        (BotInventory::MACHINEGUN, WeaponRole::MACHINEGUN),
        (BotInventory::GRENADELAUNCHER, WeaponRole::GRENADE_LAUNCHER),
        (BotInventory::ROCKETLAUNCHER, WeaponRole::ROCKET_LAUNCHER),
        (BotInventory::LIGHTNING, WeaponRole::LIGHTNING),
        (BotInventory::RAILGUN, WeaponRole::RAILGUN),
        (BotInventory::PLASMAGUN, WeaponRole::PLASMAGUN),
        (BotInventory::BFG10K, WeaponRole::BFG),
        (BotInventory::GRAPPLINGHOOK, WeaponRole::GRAPPLING_HOOK),
        (BotInventory::NAILGUN, WeaponRole::NAILGUN),
        (BotInventory::PROXLAUNCHER, WeaponRole::PROX_LAUNCHER),
        (BotInventory::CHAINGUN, WeaponRole::CHAINGUN),
    ];
    const AMMO_SLOTS: [(usize, i32); 11] = [
        (BotInventory::SHELLS, WeaponRole::SHOTGUN),
        (BotInventory::BULLETS, WeaponRole::MACHINEGUN),
        (BotInventory::GRENADES, WeaponRole::GRENADE_LAUNCHER),
        (BotInventory::CELLS, WeaponRole::PLASMAGUN),
        (BotInventory::LIGHTNINGAMMO, WeaponRole::LIGHTNING),
        (BotInventory::ROCKETS, WeaponRole::ROCKET_LAUNCHER),
        (BotInventory::SLUGS, WeaponRole::RAILGUN),
        (BotInventory::BFGAMMO, WeaponRole::BFG),
        (BotInventory::NAILS, WeaponRole::NAILGUN),
        (BotInventory::MINES, WeaponRole::PROX_LAUNCHER),
        (BotInventory::BELT, WeaponRole::CHAINGUN),
    ];
    for (index, weapon) in WEAPON_SLOTS {
        write_decision_inventory(state, index, i32::from(owned(weapon)));
    }
    for (index, weapon) in AMMO_SLOTS {
        write_decision_inventory(state, index, ammo(weapon));
    }
}

/// One selected weapon entry.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    weapon: i32,
    item: ItemId,
    ammo: Option<ItemId>,
    info: Option<WeaponInfo>,
}

/// Q3 bot weapon knowledge (donor `createQ3BotKnowledge` result).
pub struct Q3BotKnowledge<S, F> {
    simulation: S,
    actor_for_client: F,
    entries: Vec<Entry>,
    actors: RefCell<HashMap<u32, ActorId>>,
    candidates: RefCell<Vec<BotWeaponCandidate>>,
}

impl<S: Q3BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> Q3BotKnowledge<S, F> {
    /// Create Q3 bot knowledge (donor `createQ3BotKnowledge`).
    pub fn new(simulation: S, actor_for_client: F, weapons: &WeaponAi<'_>) -> Result<Self, BotArsenalError> {
        if !simulation.has_selected_q3_weapon_source() {
            return Err(BotArsenalError::MissingQ3Source);
        }
        let hook = ContentWeapon::WpGrapplingHook as i32;
        let entries = Q3_WEAPON_ITEMS
            .iter()
            .filter(|entry| entry.weapon as i32 <= hook)
            .map(|entry| {
                let weapon = entry.weapon as i32;
                let info = weapons.get_weapon_info(weapon).filter(|info| info.valid).cloned();
                Entry {
                    weapon,
                    item: entry.item.clone(),
                    ammo: entry.ammo.clone(),
                    info,
                }
            })
            .collect();
        Ok(Self {
            simulation,
            actor_for_client,
            entries,
            actors: RefCell::new(HashMap::new()),
            candidates: RefCell::new(Vec::new()),
        })
    }

    fn actor_for_client_guarded(&self, client: i32) -> Option<ActorId> {
        let actor = (self.actor_for_client)(client)?;
        self.simulation.q3_source_has(&actor).then_some(actor)
    }

    fn actor_for_handle(&self, handle: i32) -> Option<ActorId> {
        u32::try_from(handle)
            .ok()
            .and_then(|handle| self.actors.borrow().get(&handle).cloned())
    }

    fn source_state(&self, client: i32) -> Result<Option<Q3BotArsenalView>, BotArsenalError> {
        let Some(actor) = self.actor_for_client_guarded(client) else {
            return Ok(None);
        };
        let Some(view) = self.simulation.q3_arsenal(&actor) else {
            return Err(BotArsenalError::ForeignQ3Arsenal);
        };
        if !view.is_q3 {
            return Err(BotArsenalError::ForeignQ3Arsenal);
        }
        Ok(Some(view))
    }

    fn refresh(&self, handle: i32) -> Vec<BotWeaponCandidate> {
        let actor = self.actor_for_handle(handle);
        let candidates = self
            .entries
            .iter()
            .filter_map(|entry| {
                let info = entry.info.clone()?;
                Some(BotWeaponCandidate {
                    info,
                    maximum_range: if entry.weapon == WeaponRole::GAUNTLET {
                        Some(60.0)
                    } else {
                        None
                    },
                    melee: entry.weapon == WeaponRole::GAUNTLET,
                    personality_role: Some(entry.weapon),
                    supply: Some(BotWeaponCandidateSupply {
                        weapon: entry.item.clone(),
                        owned: actor
                            .as_ref()
                            .is_some_and(|actor| self.simulation.inventory_count(actor, &entry.item) > 0),
                        ammo: entry.ammo.as_ref().map(|ammo| (ammo.clone(), 1.0)),
                    }),
                })
            })
            .collect::<Vec<_>>();
        *self.candidates.borrow_mut() = candidates.clone();
        candidates
    }

    fn update_inventory_state(&self, state: &mut BotState) {
        let actor = self.actor_for_client_guarded(state.client);
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
        let owned = |weapon: i32| {
            let entry = q3_weapon_item(weapon);
            actor
                .as_ref()
                .is_some_and(|actor| entry.is_some_and(|entry| self.simulation.inventory_count(actor, &entry.item) > 0))
        };
        let ammo = |weapon: i32| {
            let entry = q3_weapon_item(weapon);
            match (actor.as_ref(), entry.and_then(|entry| entry.ammo.as_ref())) {
                (Some(actor), Some(ammo)) => self.simulation.inventory_count(actor, ammo),
                _ => 0,
            }
        };
        update_q3_bot_weapon_inventory(state, &owned, &ammo);
    }
}

impl<S: Q3BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> BotArsenalKnowledge for Q3BotKnowledge<S, F> {
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

impl<S: Q3BotKnowledgeSimulation, F: Fn(i32) -> Option<ActorId>> BotArsenalBinding for Q3BotKnowledge<S, F> {
    fn uncovered_weapons(&self) -> &[String] {
        &[]
    }

    fn resolve_weapon(&self, client: i32, decision_slot: i32) -> Option<ItemId> {
        let actor = self.actor_for_client_guarded(client)?;
        let entry = self.entries.iter().find(|entry| entry.weapon == decision_slot)?;
        if self.simulation.inventory_count(&actor, &entry.item) <= 0 {
            return None;
        }
        if entry
            .ammo
            .as_ref()
            .is_some_and(|ammo| self.simulation.inventory_count(&actor, ammo) == 0)
        {
            return None;
        }
        Some(entry.item.clone())
    }

    fn source_weapon(&self, client: i32) -> Result<i32, BotArsenalError> {
        Ok(self
            .source_state(client)?
            .map_or(ContentWeapon::WpNone as i32, |view| view.source_weapon))
    }

    fn source_weapon_state(&self, client: i32) -> Result<WeaponState, BotArsenalError> {
        let phase = self
            .source_state(client)?
            .map_or(weapon_phase::READY, |view| view.phase);
        match phase {
            weapon_phase::READY => Ok(WeaponState::WeaponReady),
            weapon_phase::RAISING => Ok(WeaponState::WeaponRaising),
            weapon_phase::DROPPING => Ok(WeaponState::WeaponDropping),
            weapon_phase::FIRING => Ok(WeaponState::WeaponFiring),
            _ => Err(BotArsenalError::InvalidQ3WeaponPhase),
        }
    }

    fn checkpoint_binding(&self) -> SaveJson {
        BotKnowledgeStore::new(BotKnowledgeSource::Q3, self.actors.borrow().clone()).checkpoint()
    }

    fn restore_binding(
        &mut self,
        reader: SaveReader<'_>,
        actor: &dyn Fn(SavedActorId) -> ActorId,
        weapon_handle: &dyn Fn(u32) -> i64,
    ) -> Result<(), BotArsenalError> {
        let mut store = BotKnowledgeStore::new(BotKnowledgeSource::Q3, HashMap::new());
        store.restore(reader, actor, weapon_handle)?;
        *self.actors.borrow_mut() = store.actors().clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::assets::BotAssetFiles;
    use qa_core::identity::IdentityOwner;

    struct FakeSimulation {
        views: HashMap<ActorId, Q3BotArsenalView>,
        counts: HashMap<(ActorId, String), i32>,
        combat: HashMap<ActorId, Q3BotCombat>,
    }

    impl Q3BotKnowledgeSimulation for FakeSimulation {
        fn has_selected_q3_weapon_source(&self) -> bool {
            true
        }
        fn q3_source_has(&self, actor: &ActorId) -> bool {
            self.views.contains_key(actor)
        }
        fn q3_arsenal(&self, actor: &ActorId) -> Option<Q3BotArsenalView> {
            self.views.get(actor).copied()
        }
        fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
            self.counts
                .get(&(actor.clone(), item.to_string()))
                .copied()
                .unwrap_or(0)
        }
        fn combat_read(&self, actor: &ActorId) -> Option<Q3BotCombat> {
            self.combat.get(actor).copied()
        }
    }

    fn actor_fn(actor: ActorId) -> impl Fn(i32) -> Option<ActorId> {
        move |_| Some(actor.clone())
    }

    fn weapons_c() -> String {
        [
            "projectileinfo { name gauntlet_hit; damage 50; radius 0; gravity 0; }",
            "weaponinfo { number 1; name gauntlet; projectile gauntlet_hit; numprojectiles 1; speed 0; reload 0.4; ammoamount 0; }",
        ]
        .join("\n")
    }

    fn fixture() -> (IdentityOwner, ActorId, FakeSimulation, BotAssetFiles) {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut views = HashMap::new();
        views.insert(
            actor.clone(),
            Q3BotArsenalView {
                is_q3: true,
                source_weapon: 1,
                phase: weapon_phase::FIRING,
            },
        );
        let mut counts = HashMap::new();
        counts.insert((actor.clone(), "q3:weapon/gauntlet".to_string()), 1);
        let mut combat = HashMap::new();
        combat.insert(
            actor.clone(),
            Q3BotCombat {
                health: 120,
                armor_points: 50,
            },
        );
        let mut files = BotAssetFiles::new();
        files.add("weapons.c", weapons_c().as_bytes());
        (owner, actor, FakeSimulation { views, counts, combat }, files)
    }

    #[test]
    fn entries_cover_selected_roster() {
        let (_owner, actor, sim, files) = fixture();
        let mut library = WeaponAi::new(&files);
        library.load_weapons("weapons.c");
        let knowledge = Q3BotKnowledge::new(sim, actor_fn(actor), &library).unwrap();
        assert_eq!(knowledge.entries.len(), 10);
        assert!(knowledge.uncovered_weapons().is_empty());
    }

    #[test]
    fn update_inventory_maps_gauntlet_and_health() {
        let (_owner, actor, sim, files) = fixture();
        let mut library = WeaponAi::new(&files);
        library.load_weapons("weapons.c");
        let mut knowledge = Q3BotKnowledge::new(sim, actor_fn(actor), &library).unwrap();
        let mut state = BotState::new(0);
        knowledge.update_inventory(&mut state);
        assert_eq!(state.inventory[BotInventory::HEALTH], 120);
        assert_eq!(state.inventory[BotInventory::GAUNTLET], 1);
        assert_eq!(state.inventory[BotInventory::MACHINEGUN], 0);
    }

    #[test]
    fn source_state_maps_firing_and_rejects_foreign() {
        let (_owner, actor, mut sim, files) = fixture();
        sim.views.get_mut(&actor).unwrap().is_q3 = false;
        let mut library = WeaponAi::new(&files);
        library.load_weapons("weapons.c");
        let knowledge = Q3BotKnowledge::new(sim, actor_fn(actor), &library).unwrap();
        assert!(matches!(
            knowledge.source_weapon(0),
            Err(BotArsenalError::ForeignQ3Arsenal)
        ));
    }

    #[test]
    fn resolve_weapon_requires_owned_item() {
        let (_owner, actor, sim, files) = fixture();
        let mut library = WeaponAi::new(&files);
        library.load_weapons("weapons.c");
        let knowledge = Q3BotKnowledge::new(sim, actor_fn(actor), &library).unwrap();
        assert_eq!(knowledge.resolve_weapon(0, 1).as_deref(), Some("q3:weapon/gauntlet"));
        assert_eq!(knowledge.resolve_weapon(0, 2), None);
        assert_eq!(knowledge.source_weapon(0).unwrap(), 1);
        assert_eq!(knowledge.source_weapon_state(0).unwrap(), WeaponState::WeaponFiring);
    }
}
