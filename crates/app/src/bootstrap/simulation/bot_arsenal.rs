//! Selected-arsenal bot weapon knowledge binding.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-arsenal.ts`.
//!
//! Missing siblings: `SharedSimulation` (`runtime.ts`, runtime partition).
//! The factory below is generic over the three dialect simulation seams so
//! the runtime partition can wire the shared simulation post-merge.
//!
//! The decision shell ([`BotWeaponCandidate`] plus the `arsenal_*` helpers)
//! mirrors `createBotArsenalKnowledge` from donor
//! `src/bots/behavior/q3/arsenal-knowledge.ts` (canonical home:
//! `qa_bots::behavior::q3::arsenal_knowledge`); unify post-merge. Two
//! adaptations are forced by the simplified Rust botlib surface:
//!
//! * Fuzzy fight weights (`evaluateFightWeapon`) have no Rust equivalent, so
//!   [`arsenal_choose_weapon`] ranks surviving candidates with a uniform
//!   positive weight and keeps the donor's same-role damage-rate tiebreak.
//! * [`BotObservedPickup`](qa_bots::behavior::q3::game_host::BotObservedPickup)
//!   carries a name instead of supply receipts, so [`arsenal_pickup_utility`]
//!   matches the pickup name against candidate supply items with the donor's
//!   constants (100 per new weapon, unit quantity over per-shot cost).
//!
//! Like the donor, [`arsenal_tactics`] and [`arsenal_aggression`] read the
//! last refreshed candidate list; the dialect structs refresh on every
//! pickup/choose/activation call.

use qa_bots::behavior::library::weapons::{WeaponAi, WeaponInfo};
use qa_bots::behavior::q3::ai_definitions::BotInventory;
use qa_bots::behavior::q3::ai_state::BotState;
use qa_bots::behavior::q3::arsenal_knowledge::{personality_role, Weapon as WeaponRole};
use qa_bots::behavior::q3::game_host::{BotArsenalKnowledge, BotWeaponTactics};
use qa_content::contract::ItemId;
use qa_content::q3::base::shared::definitions::WeaponState;
use qa_core::identity::{ActorId, SavedActorId};
use qa_world::save::value::{SaveJson, SaveReader};
use thiserror::Error;

use super::bot_knowledge_checkpoint::BotKnowledgeError;
use super::bot_q1_knowledge::{Q1BotKnowledge, Q1BotKnowledgeSimulation};
use super::bot_q2_knowledge::{Q2BotKnowledge, Q2BotKnowledgeSimulation};
use super::bot_q3_knowledge::{Q3BotKnowledge, Q3BotKnowledgeSimulation};

/// Mirror of `BotCharacteristic` aim IDs from donor
/// `src/bots/behavior/q3/ai-definitions.ts` (canonical home: `qa_bots`);
/// unify post-merge.
mod characteristic {
    pub const AIM_ACCURACY_MACHINEGUN: i32 = 8;
    pub const AIM_ACCURACY_SHOTGUN: i32 = 9;
    pub const AIM_ACCURACY_ROCKETLAUNCHER: i32 = 10;
    pub const AIM_ACCURACY_GRENADELAUNCHER: i32 = 11;
    pub const AIM_ACCURACY_LIGHTNING: i32 = 12;
    pub const AIM_ACCURACY_PLASMAGUN: i32 = 13;
    pub const AIM_ACCURACY_RAILGUN: i32 = 14;
    pub const AIM_ACCURACY_BFG10K: i32 = 15;
    pub const AIM_SKILL_ROCKETLAUNCHER: i32 = 17;
    pub const AIM_SKILL_GRENADELAUNCHER: i32 = 18;
    pub const AIM_SKILL_PLASMAGUN: i32 = 19;
    pub const AIM_SKILL_BFG10K: i32 = 20;
}

/// Selected-arsenal binding failures.
#[derive(Debug, Error)]
pub enum BotArsenalError {
    /// Q1 bot weapon knowledge requires a selected Q1 arsenal.
    #[error("Q1 bot weapon knowledge requires a selected Q1 arsenal")]
    MissingQ1Source,
    /// Q2 bot weapon knowledge requires a selected Q2 arsenal.
    #[error("Q2 bot weapon knowledge requires a selected Q2 arsenal")]
    MissingQ2Source,
    /// Q3 bot weapon knowledge requires an actual selected Q3 arsenal.
    #[error("Q3 bot weapon knowledge requires an actual selected Q3 arsenal")]
    MissingQ3Source,
    /// Selected Q3 source returned a foreign arsenal.
    #[error("Selected Q3 source returned a foreign arsenal")]
    ForeignQ3Arsenal,
    /// Selected Q3 source has an invalid weapon phase.
    #[error("Selected Q3 source has an invalid weapon phase")]
    InvalidQ3WeaponPhase,
    /// Q1 grenade trajectories are not admitted by bot weapon knowledge.
    #[error("Q1 grenade trajectories are not admitted by bot weapon knowledge")]
    GrenadeTrajectories,
    /// Weapon-handle checkpoint failure.
    #[error(transparent)]
    Checkpoint(#[from] BotKnowledgeError),
}

/// Weapon supply behind one candidate (mirror of the donor `supply` field).
#[derive(Debug, Clone, PartialEq)]
pub struct BotWeaponCandidateSupply {
    /// Weapon item.
    pub weapon: ItemId,
    /// Whether the candidate actor owns it.
    pub owned: bool,
    /// Ammo item and per-shot cost.
    pub ammo: Option<(ItemId, f64)>,
}

/// One weapon candidate (mirror of donor `BotWeaponKnowledge`).
#[derive(Debug, Clone, PartialEq)]
pub struct BotWeaponCandidate {
    /// Ballistics.
    pub info: WeaponInfo,
    /// Maximum range, or `None` for unlimited.
    pub maximum_range: Option<f64>,
    /// Melee weapon.
    pub melee: bool,
    /// Personality role override, or `None` to derive from ballistics.
    pub personality_role: Option<i32>,
    /// Supply backing, or `None` when the candidate has no supply.
    pub supply: Option<BotWeaponCandidateSupply>,
}

/// Personality role for a candidate (donor `personalityRole`).
#[must_use]
pub fn candidate_role(candidate: &BotWeaponCandidate) -> i32 {
    if let Some(role) = candidate.personality_role {
        return role;
    }
    let info = &candidate.info;
    personality_role(
        candidate.melee,
        info.projectile_info.gravity,
        info.speed,
        info.projectile_info.damage_type,
        info.projectile_count,
        info.horizontal_spread > 0.0 || info.vertical_spread > 0.0,
        info.reload,
    )
}

/// Role inventory projection (donor `profileInventory`).
fn profile_inventory(role: i32) -> Option<(usize, Option<usize>)> {
    match role {
        WeaponRole::GAUNTLET => Some((BotInventory::GAUNTLET, None)),
        WeaponRole::MACHINEGUN => Some((BotInventory::MACHINEGUN, Some(BotInventory::BULLETS))),
        WeaponRole::SHOTGUN => Some((BotInventory::SHOTGUN, Some(BotInventory::SHELLS))),
        WeaponRole::GRENADE_LAUNCHER => Some((BotInventory::GRENADELAUNCHER, Some(BotInventory::GRENADES))),
        WeaponRole::ROCKET_LAUNCHER => Some((BotInventory::ROCKETLAUNCHER, Some(BotInventory::ROCKETS))),
        WeaponRole::LIGHTNING => Some((BotInventory::LIGHTNING, Some(BotInventory::LIGHTNINGAMMO))),
        WeaponRole::RAILGUN => Some((BotInventory::RAILGUN, Some(BotInventory::SLUGS))),
        WeaponRole::PLASMAGUN => Some((BotInventory::PLASMAGUN, Some(BotInventory::CELLS))),
        WeaponRole::BFG => Some((BotInventory::BFG10K, Some(BotInventory::BFGAMMO))),
        WeaponRole::GRAPPLING_HOOK => Some((BotInventory::GRAPPLINGHOOK, None)),
        WeaponRole::NAILGUN => Some((BotInventory::NAILGUN, Some(BotInventory::NAILS))),
        WeaponRole::PROX_LAUNCHER => Some((BotInventory::PROXLAUNCHER, Some(BotInventory::MINES))),
        WeaponRole::CHAINGUN => Some((BotInventory::CHAINGUN, Some(BotInventory::BELT))),
        _ => None,
    }
}

fn inventory_at(state: &BotState, index: usize) -> i32 {
    state.inventory.get(index).copied().unwrap_or(0)
}

/// Whether the bot owns a candidate (donor `owned`).
fn candidate_owned(state: &BotState, candidate: &BotWeaponCandidate) -> bool {
    let index = match candidate.personality_role {
        None => Some(candidate.info.weapon_inventory_index),
        Some(role) => profile_inventory(role).map(|(weapon, _)| weapon as i32),
    };
    index.is_some_and(|index| usize::try_from(index).is_ok_and(|slot| inventory_at(state, slot) > 0))
}

/// Ammunition available for a candidate (donor `ammunition`).
fn candidate_ammunition(state: &BotState, candidate: &BotWeaponCandidate) -> i32 {
    if candidate.personality_role.is_none() {
        return if candidate.info.ammo_amount == 0 {
            999
        } else {
            usize::try_from(candidate.info.ammo_inventory_index).map_or(0, |slot| inventory_at(state, slot))
        };
    }
    let role = candidate.personality_role.unwrap_or(WeaponRole::NONE);
    match profile_inventory(role).map(|(_, ammo)| ammo) {
        None => 0,
        Some(None) => 999,
        Some(Some(slot)) => inventory_at(state, slot),
    }
}

/// Tactics for one weapon number (donor `tactics`).
#[must_use]
pub fn arsenal_tactics(candidates: &[BotWeaponCandidate], weapon: i32) -> BotWeaponTactics {
    let candidate = candidates.iter().find(|candidate| candidate.info.number == weapon);
    let role = candidate.map_or(WeaponRole::NONE, candidate_role);
    let accuracy = match role {
        WeaponRole::MACHINEGUN => Some(characteristic::AIM_ACCURACY_MACHINEGUN),
        WeaponRole::SHOTGUN => Some(characteristic::AIM_ACCURACY_SHOTGUN),
        WeaponRole::GRENADE_LAUNCHER => Some(characteristic::AIM_ACCURACY_GRENADELAUNCHER),
        WeaponRole::ROCKET_LAUNCHER => Some(characteristic::AIM_ACCURACY_ROCKETLAUNCHER),
        WeaponRole::LIGHTNING => Some(characteristic::AIM_ACCURACY_LIGHTNING),
        WeaponRole::RAILGUN => Some(characteristic::AIM_ACCURACY_RAILGUN),
        WeaponRole::PLASMAGUN => Some(characteristic::AIM_ACCURACY_PLASMAGUN),
        WeaponRole::BFG => Some(characteristic::AIM_ACCURACY_BFG10K),
        _ => None,
    };
    let skill = match role {
        WeaponRole::GRENADE_LAUNCHER => Some(characteristic::AIM_SKILL_GRENADELAUNCHER),
        WeaponRole::ROCKET_LAUNCHER => Some(characteristic::AIM_SKILL_ROCKETLAUNCHER),
        WeaponRole::PLASMAGUN => Some(characteristic::AIM_SKILL_PLASMAGUN),
        WeaponRole::BFG => Some(characteristic::AIM_SKILL_BFG10K),
        _ => None,
    };
    BotWeaponTactics {
        melee: candidate.is_some_and(|candidate| candidate.melee),
        maximum_range: candidate.and_then(|candidate| candidate.maximum_range.map(|range| range as f32)),
        aim_accuracy: accuracy.map(|value| value as f32),
        aim_skill: skill.map(|value| value as f32),
        weakness: if role == WeaponRole::MACHINEGUN { 90.0 } else { 0.0 },
        predict_occluded_splash: matches!(
            role,
            WeaponRole::BFG | WeaponRole::ROCKET_LAUNCHER | WeaponRole::GRENADE_LAUNCHER
        ),
    }
}

/// Aggression 0-100 for a bot (donor `aggression`).
#[must_use]
pub fn arsenal_aggression(candidates: &[BotWeaponCandidate], state: &BotState) -> f32 {
    let current = candidates
        .iter()
        .find(|candidate| candidate.info.number == state.weapon_num);
    let enemy_distance = f64::from(inventory_at(state, BotInventory::ENEMY_HORIZONTAL_DIST));
    if inventory_at(state, BotInventory::QUAD) != 0
        && (current.is_none_or(|candidate| !candidate.melee) || enemy_distance < 80.0)
    {
        return 70.0;
    }
    let health = inventory_at(state, BotInventory::HEALTH);
    let armor = inventory_at(state, BotInventory::ARMOR);
    if inventory_at(state, BotInventory::ENEMY_HEIGHT) > 200 || health < 60 || (health < 80 && armor < 40) {
        return 0.0;
    }
    const PROFILES: [(i32, i32, f32); 7] = [
        (WeaponRole::BFG, 7, 100.0),
        (WeaponRole::RAILGUN, 5, 95.0),
        (WeaponRole::LIGHTNING, 50, 90.0),
        (WeaponRole::ROCKET_LAUNCHER, 5, 90.0),
        (WeaponRole::PLASMAGUN, 40, 85.0),
        (WeaponRole::GRENADE_LAUNCHER, 10, 80.0),
        (WeaponRole::SHOTGUN, 10, 50.0),
    ];
    for (role, minimum, value) in PROFILES {
        if candidates.iter().any(|candidate| {
            candidate_role(candidate) == role
                && candidate_owned(state, candidate)
                && candidate_ammunition(state, candidate) > minimum
        }) {
            return value;
        }
    }
    0.0
}

/// Activation weapon for shootable goals (donor `activationWeapon`).
///
/// The donor extends the role order for missionpack products; the Rust
/// [`BotState`] carries no product, and missionpack roles are unreachable
/// from the Q1/Q2/Q3 dialect candidates, so the base order applies.
#[must_use]
pub fn arsenal_activation_weapon(candidates: &[BotWeaponCandidate], state: &BotState) -> i32 {
    let available: Vec<&BotWeaponCandidate> = candidates
        .iter()
        .filter(|candidate| {
            if !candidate.info.valid
                || candidate.melee
                || candidate.info.projectile_info.gravity != 0.0
                || !candidate_owned(state, candidate)
            {
                return false;
            }
            if candidate.personality_role.is_none() {
                candidate_ammunition(state, candidate) >= candidate.info.ammo_amount
            } else {
                candidate_ammunition(state, candidate) > 0
            }
        })
        .collect();
    const ORDER: [i32; 7] = [
        WeaponRole::MACHINEGUN,
        WeaponRole::SHOTGUN,
        WeaponRole::PLASMAGUN,
        WeaponRole::LIGHTNING,
        WeaponRole::RAILGUN,
        WeaponRole::ROCKET_LAUNCHER,
        WeaponRole::BFG,
    ];
    for role in ORDER {
        if let Some(candidate) = available.iter().find(|candidate| candidate_role(candidate) == role) {
            return candidate.info.number;
        }
    }
    -1
}

/// Choose a weapon number for a bot (donor `chooseWeapon`).
#[must_use]
pub fn arsenal_choose_weapon(candidates: &[BotWeaponCandidate], state: &BotState) -> i32 {
    let mut best_weight = 0.0f32;
    let mut best_weapon = 0;
    let mut best_role: Option<i32> = None;
    let mut best_rate = 0.0f64;
    for candidate in candidates {
        let role = candidate_role(candidate);
        if candidate.personality_role.is_some() {
            consider_candidate(
                candidate,
                role,
                &mut best_weight,
                &mut best_weapon,
                &mut best_role,
                &mut best_rate,
            );
            continue;
        }
        if !candidate_owned(state, candidate) || candidate_ammunition(state, candidate) < candidate.info.ammo_amount {
            continue;
        }
        let distance = f64::hypot(
            f64::from(inventory_at(state, BotInventory::ENEMY_HORIZONTAL_DIST)),
            f64::from(inventory_at(state, BotInventory::ENEMY_HEIGHT)),
        );
        if candidate.maximum_range.is_some_and(|range| distance > range) {
            continue;
        }
        if profile_inventory(role).is_none() {
            continue;
        }
        consider_candidate(
            candidate,
            role,
            &mut best_weight,
            &mut best_weapon,
            &mut best_role,
            &mut best_rate,
        );
    }
    best_weapon
}

#[allow(clippy::too_many_arguments)]
fn consider_candidate(
    candidate: &BotWeaponCandidate,
    role: i32,
    best_weight: &mut f32,
    best_weapon: &mut i32,
    best_role: &mut Option<i32>,
    best_rate: &mut f64,
) {
    // Uniform positive weight: fuzzy fight weights have no Rust equivalent.
    let weight = 1.0f32;
    let info = &candidate.info;
    let rate = if info.reload > 0.0 {
        f64::from(info.projectile_info.damage) * f64::from(info.projectile_count) / f64::from(info.reload)
    } else {
        0.0
    };
    if weight > *best_weight
        || (weight > 0.0 && weight == *best_weight && Some(role) == *best_role && rate > *best_rate)
    {
        *best_weight = weight;
        *best_weapon = info.number;
        *best_role = Some(role);
        *best_rate = rate;
    }
}

fn normalized_item(name: &str) -> String {
    let name = name.rsplit(':').next().unwrap_or(name);
    let name = name.rsplit("weapon/").next().unwrap_or(name);
    let name = name.rsplit("weapon_").next().unwrap_or(name);
    name.replace(['_', '/'], "")
}

/// Pickup utility for a bot (donor `pickupUtility`, name-matched).
#[must_use]
pub fn arsenal_pickup_utility(candidates: &[BotWeaponCandidate], pickup_name: &str, eligible: bool) -> f32 {
    if !eligible {
        return 0.0;
    }
    let name = normalized_item(pickup_name);
    for candidate in candidates {
        let Some(supply) = candidate.supply.as_ref() else {
            continue;
        };
        if !candidate.info.valid {
            continue;
        }
        if normalized_item(&supply.weapon) == name && !supply.owned {
            return 100.0;
        }
        if let Some((item, per_shot)) = supply.ammo.as_ref() {
            if normalized_item(item) == name && *per_shot > 0.0 {
                return (1.0 / *per_shot) as f32;
            }
        }
    }
    0.0
}

/// Zero the decision inventory prefix (donor `for` loop shared by dialects).
pub fn clear_decision_inventory(state: &mut BotState) {
    if state.inventory.len() < 200 {
        state.inventory.resize(200, 0);
    }
    state.inventory[..200].fill(0);
}

/// Write one decision inventory slot, growing the inventory like the donor.
pub fn write_decision_inventory(state: &mut BotState, index: usize, value: i32) {
    if state.inventory.len() <= index {
        state.inventory.resize(index + 1, 0);
    }
    state.inventory[index] = value;
}

/// Read one decision inventory slot (donor `?? 0`).
#[must_use]
pub fn read_decision_inventory(state: &BotState, index: usize) -> i32 {
    inventory_at(state, index)
}

/// Selected-arsenal binding (donor `BotArsenalBinding`).
pub trait BotArsenalBinding: BotArsenalKnowledge {
    /// Weapons the dialect tables cannot describe.
    fn uncovered_weapons(&self) -> &[String];
    /// Resolve a decision slot to a weapon item (donor `resolveWeapon`).
    fn resolve_weapon(&self, client: i32, decision_slot: i32) -> Option<ItemId>;
    /// Current source weapon number (donor `sourceWeapon`).
    fn source_weapon(&self, client: i32) -> Result<i32, BotArsenalError>;
    /// Current source weapon state (donor `sourceWeaponState`).
    fn source_weapon_state(&self, client: i32) -> Result<WeaponState, BotArsenalError>;
    /// Checkpoint the weapon-handle map.
    fn checkpoint_binding(&self) -> SaveJson;
    /// Restore the weapon-handle map.
    fn restore_binding(
        &mut self,
        reader: SaveReader<'_>,
        actor: &dyn Fn(SavedActorId) -> ActorId,
        weapon_handle: &dyn Fn(u32) -> i64,
    ) -> Result<(), BotArsenalError>;
}

/// Create the binding for the selected arsenal (donor `createBotArsenalBinding`).
///
/// `weapons` resolves the selected Q3 weapon config; it is ignored unless a
/// Q3 arsenal is selected.
pub fn create_bot_arsenal_binding<S, F>(
    simulation: S,
    actor_for_client: F,
    weapons: &WeaponAi<'_>,
) -> Option<Box<dyn BotArsenalBinding>>
where
    S: Q1BotKnowledgeSimulation + Q2BotKnowledgeSimulation + Q3BotKnowledgeSimulation + 'static,
    F: Fn(i32) -> Option<ActorId> + Clone + 'static,
{
    if simulation.has_q2_weapon_source() {
        let knowledge = Q2BotKnowledge::new(simulation, actor_for_client).ok()?;
        return Some(Box::new(knowledge));
    }
    if simulation.has_q1_weapon_source() {
        let knowledge = Q1BotKnowledge::new(simulation, actor_for_client).ok()?;
        return Some(Box::new(knowledge));
    }
    if simulation.has_selected_q3_weapon_source() {
        let knowledge = Q3BotKnowledge::new(simulation, actor_for_client, weapons).ok()?;
        return Some(Box::new(knowledge));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::library::weapons::{ProjectileInfo, DAMAGE_TYPE_IMPACT};
    use qa_core::math::Vec3;

    pub fn test_candidate(number: i32) -> BotWeaponCandidate {
        BotWeaponCandidate {
            info: WeaponInfo {
                valid: true,
                number,
                name: format!("weapon{number}"),
                model: String::new(),
                level: 0,
                weapon_inventory_index: 64 + number,
                flags: 0,
                projectile: format!("weapon{number}"),
                projectile_count: 1,
                horizontal_spread: 0.0,
                vertical_spread: 0.0,
                speed: 0.0,
                acceleration: 0.0,
                recoil: Vec3::default(),
                offset: Vec3::default(),
                angle_offset: Vec3::default(),
                extra_z_velocity: 0.0,
                ammo_amount: 1,
                ammo_inventory_index: 96 + number,
                activate: 0.0,
                reload: 0.5,
                spin_up: 0.0,
                spin_down: 0.0,
                projectile_info: ProjectileInfo {
                    name: format!("weapon{number}"),
                    model: String::new(),
                    flags: 0,
                    gravity: 0.0,
                    damage: 10.0,
                    radius: 0.0,
                    visible_damage: 0.0,
                    damage_type: DAMAGE_TYPE_IMPACT,
                    health_increase: 0.0,
                    push: 0.0,
                    detonation: 0.0,
                    bounce: 0.0,
                    bounce_friction: 0.0,
                    bounce_stop: 0.0,
                },
            },
            maximum_range: None,
            melee: false,
            personality_role: None,
            supply: Some(BotWeaponCandidateSupply {
                weapon: format!("q1:weapon/test{number}"),
                owned: true,
                ammo: Some((format!("q1:ammo/test{number}"), 1.0)),
            }),
        }
    }

    fn test_state() -> BotState {
        BotState::new(0)
    }

    #[test]
    fn tactics_derive_machinegun_role_from_spread() {
        let mut candidate = test_candidate(2);
        candidate.info.horizontal_spread = 0.5;
        let tactics = arsenal_tactics(&[candidate], 2);
        assert_eq!(tactics.weakness, 90.0);
        assert_eq!(tactics.aim_accuracy, Some(8.0));
        assert!(!tactics.predict_occluded_splash);
    }

    #[test]
    fn aggression_returns_zero_when_weak() {
        let candidate = test_candidate(2);
        let mut state = test_state();
        state.inventory[BotInventory::HEALTH] = 50;
        assert_eq!(arsenal_aggression(&[candidate], &state), 0.0);
    }

    #[test]
    fn choose_weapon_prefers_same_role_damage_rate() {
        let mut slow = test_candidate(2);
        slow.info.horizontal_spread = 0.5;
        slow.info.projectile_info.damage = 4.0;
        let mut fast = test_candidate(3);
        fast.info.horizontal_spread = 0.5;
        fast.info.projectile_info.damage = 40.0;
        let mut state = test_state();
        state.inventory[66] = 1;
        state.inventory[67] = 1;
        state.inventory[98] = 10;
        state.inventory[99] = 10;
        assert_eq!(arsenal_choose_weapon(&[slow, fast], &state), 3);
    }

    #[test]
    fn pickup_utility_prices_new_weapons() {
        let mut candidate = test_candidate(2);
        candidate.supply.as_mut().unwrap().owned = false;
        assert_eq!(arsenal_pickup_utility(&[candidate], "weapon/test2", true), 100.0);
    }

    #[test]
    fn ineligible_pickups_have_no_utility() {
        let candidate = test_candidate(2);
        assert_eq!(arsenal_pickup_utility(&[candidate], "weapon/test2", false), 0.0);
    }

    #[test]
    fn activation_prefers_machinegun_role() {
        let mut machinegun = test_candidate(2);
        machinegun.info.horizontal_spread = 0.5;
        let mut shotgun = test_candidate(3);
        shotgun.info.projectile_count = 6;
        let mut state = test_state();
        state.inventory[66] = 1;
        state.inventory[67] = 1;
        state.inventory[98] = 10;
        state.inventory[99] = 10;
        assert_eq!(arsenal_activation_weapon(&[shotgun, machinegun], &state), 2);
    }
}
