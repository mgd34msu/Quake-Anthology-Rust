//! Q3 source arsenal adapter: the `PM_Weapon` stage and `ClientSpawn`
//! loadout (`src/content/q3/foundation/arsenal.ts`, from id Software `bg_pmove.c`,
//! GPL-2.0-or-later).
//!
//! The world weapon step reports post-state rather than the donor's
//! setter calls, so per-set weapon effects collapse into one net per-step
//! effect and buffered event/torso effects follow the state diffs. Net
//! state is exact; effect granularity is documented at
//! [`step_q3_arsenal`]. Movement ammo entries carry no capacity (the world
//! movement mirror drops it), so the loadout's capacities are unrepresentable
//! here; counts are exact.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use qa_core::identity::{OwnedActor, ProviderId};
use qa_core::time::{FrameContext, SourceTime};
use qa_world::movement::q3::animation::{run_q3_torso_operation, Q3AnimationContext};
use qa_world::movement::q3::constants::{command_buttons, move_flags, player_animation, weapon, weapon_state};
use qa_world::movement::q3::weapon::{
    run_q3_weapon_step, Q3ExternalWeaponSlot, Q3SourceWeaponOptions, Q3SourceWeaponState, Q3WeaponCommand,
};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, InventoryEntry, ItemId, MovementEffect, MovementEnvironment,
    PredictableMovementEvent, UserCommand, WeaponState as FamilyWeaponState,
};
use thiserror::Error;

use super::super::base::shared::definitions::{Product, Weapon};

/// Arsenal adapter failure (donor `TypeError`/`RangeError`/`Error` throws).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ArsenalError {
    /// Non-Q3 weapon state (donor `TypeError`).
    #[error("Q3 arsenal adapter requires Q3 weapon state")]
    NotQ3Arsenal,
    /// Bad step clock (donor `RangeError`).
    #[error("Q3 weapon step clock must be finite and nonnegative")]
    BadClock,
    /// Weapon outside the product table.
    #[error("Requested weapon does not belong to the Q3 product")]
    BadWeaponRequest,
    /// Resume before the source drop finished.
    #[error("Q3 primary must finish its source drop before resuming")]
    ResumeBeforeDrop,
    /// Ammo write for a weapon with no consumable slot.
    #[error("Q3 weapon has no consumable ammo slot: {0}")]
    NoAmmoSlot(i32),
    /// Ammo write for a missing inventory entry.
    #[error("Missing Q3 ammo inventory entry: {0}")]
    MissingAmmoEntry(String),
    /// Torso operation failure (unreachable: calls are Q3-gated).
    #[error("Q3 torso operation failed: {0}")]
    Torso(String),
}

/// Q3 weapon item row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3WeaponItem {
    /// Source weapon number.
    pub weapon: Weapon,
    /// Weapon item.
    pub item: ItemId,
    /// Ammo item, if any.
    pub ammo: Option<ItemId>,
}

/// Q3 weapon/ammo item rows.
pub static Q3_WEAPON_ITEMS: LazyLock<Vec<Q3WeaponItem>> = LazyLock::new(|| {
    const ROWS: [(Weapon, &str, Option<&str>); 13] = [
        (Weapon::WpGauntlet, "q3:weapon/gauntlet", None),
        (Weapon::WpMachinegun, "q3:weapon/machinegun", Some("q3:ammo/machinegun")),
        (Weapon::WpShotgun, "q3:weapon/shotgun", Some("q3:ammo/shotgun")),
        (
            Weapon::WpGrenadeLauncher,
            "q3:weapon/grenadelauncher",
            Some("q3:ammo/grenadelauncher"),
        ),
        (
            Weapon::WpRocketLauncher,
            "q3:weapon/rocketlauncher",
            Some("q3:ammo/rocketlauncher"),
        ),
        (Weapon::WpLightning, "q3:weapon/lightning", Some("q3:ammo/lightning")),
        (Weapon::WpRailgun, "q3:weapon/railgun", Some("q3:ammo/railgun")),
        (Weapon::WpPlasmagun, "q3:weapon/plasmagun", Some("q3:ammo/plasmagun")),
        (Weapon::WpBfg, "q3:weapon/bfg", Some("q3:ammo/bfg")),
        (Weapon::WpGrapplingHook, "q3:weapon/grapple", None),
        (Weapon::WpNailgun, "q3:weapon/nailgun", Some("q3:ammo/nailgun")),
        (
            Weapon::WpProxLauncher,
            "q3:weapon/proxlauncher",
            Some("q3:ammo/proxlauncher"),
        ),
        (Weapon::WpChaingun, "q3:weapon/chaingun", Some("q3:ammo/chaingun")),
    ];
    ROWS.into_iter()
        .map(|(weapon, item, ammo)| Q3WeaponItem {
            weapon,
            item: item.to_string(),
            ammo: ammo.map(str::to_string),
        })
        .collect()
});

/// Weapon item row for a source weapon number.
#[must_use]
pub fn q3_weapon_item(weapon: i32) -> Option<&'static Q3WeaponItem> {
    Q3_WEAPON_ITEMS.iter().find(|entry| entry.weapon as i32 == weapon)
}

/// Q3 arsenal controls for one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ArsenalControls {
    /// Attack held.
    pub attack: bool,
    /// Use-holdable held.
    pub use_holdable: bool,
    /// Requested weapon number.
    pub requested_weapon: i32,
}

/// Q3 arsenal runtime state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalRuntimeState {
    /// Product.
    pub product: Product,
    /// Maximum health.
    pub max_health: f64,
    /// Spectator.
    pub spectator: bool,
    /// Persistent powerup tag.
    pub persistent_powerup_tag: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Holdable tag.
    pub holdable_tag: i32,
    /// Respawned flag.
    pub respawned: bool,
    /// Use-item held flag.
    pub use_item_held: bool,
    /// Event sequence.
    pub event_sequence: i32,
    /// Fractional milliseconds carried to the next step.
    pub fractional_milliseconds: f64,
    /// External weapon slot phase.
    pub external_slot: Q3ExternalWeaponSlot,
    /// Requested weapon override.
    pub requested_weapon: Option<i32>,
}

/// Request a weapon on the runtime.
pub fn q3_request_weapon(runtime: &Q3ArsenalRuntimeState, weapon: i32) -> Result<Q3ArsenalRuntimeState, ArsenalError> {
    if !Q3_WEAPON_ITEMS
        .iter()
        .any(|entry| entry.weapon as i32 == weapon && (runtime.product == Product::Missionpack || weapon <= 10))
    {
        return Err(ArsenalError::BadWeaponRequest);
    }
    Ok(Q3ArsenalRuntimeState {
        requested_weapon: Some(weapon),
        ..runtime.clone()
    })
}

/// Request the external slot holstered.
#[must_use]
pub fn q3_request_weapon_holster(runtime: &Q3ArsenalRuntimeState) -> Q3ArsenalRuntimeState {
    if runtime.external_slot == Q3ExternalWeaponSlot::ResumeRequested {
        return Q3ArsenalRuntimeState {
            external_slot: Q3ExternalWeaponSlot::Holstered,
            ..runtime.clone()
        };
    }
    if runtime.external_slot == Q3ExternalWeaponSlot::Active {
        return Q3ArsenalRuntimeState {
            external_slot: Q3ExternalWeaponSlot::HolsterRequested,
            ..runtime.clone()
        };
    }
    runtime.clone()
}

/// Request the external slot resumed.
pub fn q3_request_weapon_resume(runtime: &Q3ArsenalRuntimeState) -> Result<Q3ArsenalRuntimeState, ArsenalError> {
    if runtime.external_slot == Q3ExternalWeaponSlot::Active
        || runtime.external_slot == Q3ExternalWeaponSlot::ResumeRequested
    {
        return Ok(runtime.clone());
    }
    if runtime.external_slot != Q3ExternalWeaponSlot::Holstered {
        return Err(ArsenalError::ResumeBeforeDrop);
    }
    Ok(Q3ArsenalRuntimeState {
        external_slot: Q3ExternalWeaponSlot::ResumeRequested,
        ..runtime.clone()
    })
}

/// Weapon step input (donor `WeaponStepInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStepInput {
    /// Acting actor.
    pub actor: OwnedActor,
    /// User command.
    pub command: UserCommand,
    /// Frame context.
    pub frame: FrameContext,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
    /// Movement environment.
    pub environment: MovementEnvironment,
    /// Gauntlet hit flag.
    pub gauntlet_hit: bool,
}

/// Q3 weapon step result: donor `WeaponStepResult` plus the runtime and the
/// torso requests a foreign character consumes through its own pose adapter.
/// Q3 steps never continue or remove, so there is no continuation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalStep {
    /// Updated arsenal.
    pub arsenal: ArsenalState,
    /// Updated animation.
    pub animation: ActorAnimationState,
    /// Emitted effects.
    pub effects: Vec<MovementEffect>,
    /// Updated runtime.
    pub runtime: Q3ArsenalRuntimeState,
    /// Torso animations requested, in order.
    pub torso_animations: Vec<i32>,
}

/// Step the Q3 arsenal.
///
/// The selected input adapter supplies Q3 actions; foreign button words are
/// never reinterpreted. The donor records one effect per source setter call;
/// the world step reports post-state, so setter-level effects collapse into
/// net per-step effects here: at most one weapon effect, one selection
/// effect, and one ammo effect per changed counter, followed by the buffered
/// event and torso effects in callback order.
pub fn step_q3_arsenal(
    input: &WeaponStepInput,
    runtime: &Q3ArsenalRuntimeState,
    controls: &Q3ArsenalControls,
    firing_delay: Option<&dyn Fn(i32) -> i32>,
) -> Result<Q3ArsenalStep, ArsenalError> {
    let FamilyWeaponState::Q3 {
        source_weapon,
        state,
        time_milliseconds,
    } = input.arsenal.state.clone()
    else {
        return Err(ArsenalError::NotQ3Arsenal);
    };
    let elapsed = match input.frame.elapsed {
        SourceTime::Milliseconds(value) => value as f64,
        SourceTime::Seconds(value) => f64::from(value) * 1000.0,
    };
    let clock = elapsed + runtime.fractional_milliseconds;
    let msec = clock.trunc() as i32;
    if !clock.is_finite() || msec < 0 {
        return Err(ArsenalError::BadClock);
    }
    let mut order: Vec<&InventoryEntry> = Vec::with_capacity(input.arsenal.ammo.len());
    let mut counts: HashMap<&str, f64> = HashMap::with_capacity(input.arsenal.ammo.len());
    for entry in &input.arsenal.ammo {
        order.push(entry);
        counts.insert(entry.item.as_str(), entry.count);
    }
    let mut owned_weapons = 0;
    for weapon in Q3_WEAPON_ITEMS.iter() {
        if counts.get(weapon.item.as_str()).copied().unwrap_or(0.0) > 0.0 {
            owned_weapons |= 1 << (weapon.weapon as i32);
        }
    }
    let before = (source_weapon, state, time_milliseconds);
    let mut ammo_before: HashMap<i32, i32> = HashMap::with_capacity(Q3_WEAPON_ITEMS.len());
    for weapon in Q3_WEAPON_ITEMS.iter() {
        let count = match weapon.ammo.as_deref() {
            None => -1,
            Some(ammo) => counts.get(ammo).copied().unwrap_or(0.0) as i32,
        };
        ammo_before.insert(weapon.weapon as i32, count);
    }
    let mut world_state = Q3SourceWeaponState {
        product: runtime.product.into(),
        pm_flags: (if runtime.respawned { move_flags::RESPAWNED } else { 0 })
            | (if runtime.use_item_held {
                move_flags::USE_ITEM_HELD
            } else {
                0
            }),
        weapon: source_weapon,
        weapon_state: state,
        weapon_time: time_milliseconds,
        owned_weapons,
        health: input.environment.health,
        max_health: runtime.max_health,
        spectator: runtime.spectator,
        haste: input.environment.haste,
        persistent_powerup_tag: runtime.persistent_powerup_tag,
        holdable_item: runtime.holdable_item,
        holdable_tag: runtime.holdable_tag,
        ammo: ammo_before.clone(),
    };
    // PMF_RESPAWNED clears when both source actions are released, even under
    // foreign movement.
    if input.environment.health > 0.0 && !controls.attack && !controls.use_holdable {
        world_state.pm_flags &= !move_flags::RESPAWNED;
    }
    let provider = input.arsenal.provider.clone();
    let animation = Rc::new(RefCell::new(input.animation.clone()));
    let sequence = Rc::new(RefCell::new(runtime.event_sequence));
    let torso_animations = Rc::new(RefCell::new(Vec::new()));
    let buffered_events = Rc::new(RefCell::new(Vec::new()));
    let buffered_torso = Rc::new(RefCell::new(Vec::new()));
    let torso_error = Rc::new(RefCell::new(None));
    let mut options = Q3SourceWeaponOptions {
        msec,
        gauntlet_hit: input.gauntlet_hit,
        external_slot: Some(runtime.external_slot),
        firing_delay: firing_delay
            .map(|delay| Box::new(move |milliseconds: i32| delay(milliseconds)) as Box<dyn Fn(i32) -> i32>),
        event: {
            let provider = provider.clone();
            let sequence = Rc::clone(&sequence);
            let buffered_events = Rc::clone(&buffered_events);
            Box::new(move |event: i32| {
                let next = *sequence.borrow();
                *sequence.borrow_mut() = next + 1;
                buffered_events
                    .borrow_mut()
                    .push(MovementEffect::Event(PredictableMovementEvent {
                        provider: provider.clone(),
                        sequence: next,
                        event,
                        parameter: 0,
                    }));
            })
        },
        start_torso: {
            let animation = Rc::clone(&animation);
            let sequence = Rc::clone(&sequence);
            let torso_animations = Rc::clone(&torso_animations);
            let buffered_torso = Rc::clone(&buffered_torso);
            let torso_error = Rc::clone(&torso_error);
            let health = input.environment.health;
            let product = runtime.product;
            Box::new(move |torso: i32| {
                torso_animations.borrow_mut().push(torso);
                // A foreign character consumes these semantic source requests
                // through its own pose adapter.
                if !matches!(animation.borrow().state, AnimationState::Q3 { .. }) || health <= 0.0 {
                    return;
                }
                let context = Q3AnimationContext {
                    animation: animation.borrow().clone(),
                    dead: false,
                    elapsed_milliseconds: msec,
                    buttons: 0,
                    product: product.into(),
                    event_sequence: *sequence.borrow(),
                };
                match run_q3_torso_operation(torso, &context, false) {
                    Ok(result) => {
                        buffered_torso.borrow_mut().extend(result.effects);
                        *animation.borrow_mut() = result.animation;
                    }
                    Err(error) => *torso_error.borrow_mut() = Some(error.to_string()),
                }
            })
        },
    };
    run_q3_weapon_step(
        &mut world_state,
        Q3WeaponCommand {
            buttons: (if controls.attack { command_buttons::ATTACK } else { 0 })
                | (if controls.use_holdable {
                    command_buttons::USE_HOLDABLE
                } else {
                    0
                }),
            weapon: runtime.requested_weapon.unwrap_or(controls.requested_weapon),
        },
        &mut options,
    );
    if let Some(error) = torso_error.borrow().clone() {
        return Err(ArsenalError::Torso(error));
    }
    let after = (world_state.weapon, world_state.weapon_state, world_state.weapon_time);
    let mut effects = Vec::new();
    if after != before {
        effects.push(MovementEffect::Weapon {
            provider: provider.clone(),
            before: FamilyWeaponState::Q3 {
                source_weapon: before.0,
                state: before.1,
                time_milliseconds: before.2,
            },
            after: FamilyWeaponState::Q3 {
                source_weapon: after.0,
                state: after.1,
                time_milliseconds: after.2,
            },
        });
    }
    if after.0 != before.0 {
        effects.push(MovementEffect::WeaponSelection {
            provider: provider.clone(),
            before: q3_weapon_item(before.0).map(|weapon| weapon.item.clone()),
            after: q3_weapon_item(after.0).map(|weapon| weapon.item.clone()),
        });
    }
    for key in world_state.ammo.keys() {
        if !ammo_before.contains_key(key) {
            return Err(ArsenalError::NoAmmoSlot(*key));
        }
    }
    let mut ammo: Vec<InventoryEntry> = Vec::with_capacity(order.len());
    for entry in order {
        let mut entry = entry.clone();
        if let Some(weapon) = Q3_WEAPON_ITEMS
            .iter()
            .find(|weapon| weapon.ammo.as_deref() == Some(entry.item.as_str()))
        {
            let number = weapon.weapon as i32;
            let before_count = ammo_before[&number];
            let after_count = world_state.ammo.get(&number).copied().unwrap_or(before_count);
            if after_count != before_count {
                effects.push(MovementEffect::Ammo {
                    item: entry.item.clone(),
                    before: entry.count,
                    after: after_count as f64,
                });
                entry.count = after_count as f64;
            }
        }
        ammo.push(entry);
    }
    for weapon in Q3_WEAPON_ITEMS.iter() {
        if weapon.ammo.is_none()
            && world_state.ammo.get(&(weapon.weapon as i32)).copied().unwrap_or(-1)
                != ammo_before[&(weapon.weapon as i32)]
        {
            return Err(ArsenalError::NoAmmoSlot(weapon.weapon as i32));
        }
    }
    effects.extend(buffered_events.borrow().iter().cloned());
    effects.extend(buffered_torso.borrow().iter().cloned());
    let final_animation = animation.borrow().clone();
    let final_torso_animations = torso_animations.borrow().clone();
    let final_sequence = *sequence.borrow();
    Ok(Q3ArsenalStep {
        arsenal: ArsenalState {
            provider,
            active_weapon: q3_weapon_item(world_state.weapon).map(|weapon| weapon.item.clone()),
            state: FamilyWeaponState::Q3 {
                source_weapon: after.0,
                state: after.1,
                time_milliseconds: after.2,
            },
            ammo,
        },
        animation: final_animation,
        effects,
        torso_animations: final_torso_animations,
        runtime: Q3ArsenalRuntimeState {
            holdable_item: world_state.holdable_item,
            holdable_tag: world_state.holdable_tag,
            respawned: world_state.pm_flags & move_flags::RESPAWNED != 0,
            use_item_held: world_state.pm_flags & move_flags::USE_ITEM_HELD != 0,
            fractional_milliseconds: clock - msec as f64,
            event_sequence: final_sequence,
            external_slot: options.external_slot.unwrap_or(runtime.external_slot),
            requested_weapon: match runtime.requested_weapon {
                Some(requested) if requested == world_state.weapon => None,
                requested => requested,
            },
            ..runtime.clone()
        },
    })
}

/// Spawn arsenal runtime (donor default `eventSequence` is zero).
#[must_use]
pub fn q3_spawn_arsenal_runtime(product: Product, max_health: f64, event_sequence: i32) -> Q3ArsenalRuntimeState {
    Q3ArsenalRuntimeState {
        product,
        max_health,
        spectator: false,
        persistent_powerup_tag: 0,
        holdable_item: 0,
        holdable_tag: 0,
        respawned: true,
        use_item_held: false,
        event_sequence,
        fractional_milliseconds: 0.0,
        external_slot: Q3ExternalWeaponSlot::Active,
        requested_weapon: None,
    }
}

/// Spawn loadout.
#[must_use]
pub fn q3_spawn_loadout(provider: ProviderId, product: Product, team_deathmatch: bool) -> ArsenalState {
    let mut ammo = Vec::new();
    for weapon in Q3_WEAPON_ITEMS.iter() {
        let number = weapon.weapon as i32;
        if product == Product::Baseq3 && number >= weapon::NAILGUN {
            continue;
        }
        ammo.push(InventoryEntry {
            item: weapon.item.clone(),
            count: if number == weapon::GAUNTLET || number == weapon::MACHINEGUN {
                1.0
            } else {
                0.0
            },
        });
        if let Some(ammo_item) = weapon.ammo.as_deref() {
            ammo.push(InventoryEntry {
                item: ammo_item.to_string(),
                count: if number == weapon::MACHINEGUN {
                    if team_deathmatch {
                        50.0
                    } else {
                        100.0
                    }
                } else {
                    0.0
                },
            });
        }
    }
    ArsenalState {
        provider,
        active_weapon: Some("q3:weapon/machinegun".to_string()),
        state: FamilyWeaponState::Q3 {
            source_weapon: weapon::MACHINEGUN,
            state: weapon_state::READY,
            time_milliseconds: 0,
        },
        ammo,
    }
}

/// Spawn animation.
#[must_use]
pub fn q3_spawn_animation() -> AnimationState {
    AnimationState::Q3 {
        legs: player_animation::LEGS_IDLE,
        torso: player_animation::TORSO_STAND,
        legs_timer_milliseconds: 0,
        torso_timer_milliseconds: 0,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::time::FramePhase;
    use qa_world::movement::q3::constants::{entity_event, holdable};

    use super::*;

    fn provider() -> ProviderId {
        ProviderId::new("q3", "test")
    }

    fn input(arsenal: ArsenalState) -> WeaponStepInput {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 0);
        WeaponStepInput {
            actor: owner.owned_actor(&actor, provider()).unwrap(),
            command: UserCommand::Q3(qa_world::movement::types::Q3UserCommand {
                server_time_milliseconds: 0,
                angle_words: [0, 0, 0],
                buttons: 0,
                weapon: weapon::MACHINEGUN,
                forward_move: 0,
                right_move: 0,
                up_move: 0,
            }),
            frame: FrameContext {
                frame: 1,
                time: SourceTime::Milliseconds(16),
                elapsed: SourceTime::Milliseconds(16),
                phase: FramePhase::FrameEntry,
            },
            arsenal,
            animation: ActorAnimationState {
                provider: provider(),
                state: q3_spawn_animation(),
            },
            environment: MovementEnvironment {
                health: 100.0,
                ..Default::default()
            },
            gauntlet_hit: false,
        }
    }

    fn runtime() -> Q3ArsenalRuntimeState {
        Q3ArsenalRuntimeState {
            respawned: false,
            ..q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 0)
        }
    }

    fn controls() -> Q3ArsenalControls {
        Q3ArsenalControls {
            attack: false,
            use_holdable: false,
            requested_weapon: weapon::MACHINEGUN,
        }
    }

    #[test]
    fn weapon_requests_are_product_gated() {
        let base = runtime();
        assert!(q3_request_weapon(&base, weapon::SHOTGUN).unwrap().requested_weapon == Some(weapon::SHOTGUN));
        assert_eq!(
            q3_request_weapon(&base, weapon::NAILGUN),
            Err(ArsenalError::BadWeaponRequest)
        );
        assert_eq!(q3_request_weapon(&base, 99), Err(ArsenalError::BadWeaponRequest));
        let pack = q3_spawn_arsenal_runtime(Product::Missionpack, 100.0, 0);
        assert!(q3_request_weapon(&pack, weapon::CHAINGUN).is_ok());
    }

    #[test]
    fn holster_and_resume_follow_slot_phases() {
        let active = runtime();
        assert_eq!(
            q3_request_weapon_holster(&active).external_slot,
            Q3ExternalWeaponSlot::HolsterRequested
        );
        let resume_requested = Q3ArsenalRuntimeState {
            external_slot: Q3ExternalWeaponSlot::ResumeRequested,
            ..runtime()
        };
        assert_eq!(
            q3_request_weapon_holster(&resume_requested).external_slot,
            Q3ExternalWeaponSlot::Holstered
        );
        let holstered = Q3ArsenalRuntimeState {
            external_slot: Q3ExternalWeaponSlot::Holstered,
            ..runtime()
        };
        assert_eq!(
            q3_request_weapon_resume(&holstered).unwrap().external_slot,
            Q3ExternalWeaponSlot::ResumeRequested
        );
        assert_eq!(
            q3_request_weapon_resume(&active).unwrap().external_slot,
            Q3ExternalWeaponSlot::Active
        );
        let dropping = Q3ArsenalRuntimeState {
            external_slot: Q3ExternalWeaponSlot::Dropping,
            ..runtime()
        };
        assert_eq!(q3_request_weapon_resume(&dropping), Err(ArsenalError::ResumeBeforeDrop));
    }

    #[test]
    fn spawns_cover_runtime_loadout_and_animation() {
        let spawned = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 7);
        assert!(spawned.respawned);
        assert_eq!(spawned.event_sequence, 7);
        let loadout = q3_spawn_loadout(provider(), Product::Baseq3, false);
        assert_eq!(loadout.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
        let machinegun = loadout
            .ammo
            .iter()
            .find(|entry| entry.item == "q3:ammo/machinegun")
            .unwrap();
        assert_eq!(machinegun.count, 100.0);
        assert!(loadout.ammo.iter().all(|entry| !entry.item.contains("nailgun")));
        let team = q3_spawn_loadout(provider(), Product::Missionpack, true);
        let machinegun = team
            .ammo
            .iter()
            .find(|entry| entry.item == "q3:ammo/machinegun")
            .unwrap();
        assert_eq!(machinegun.count, 50.0);
        assert!(team.ammo.iter().any(|entry| entry.item == "q3:ammo/nailgun"));
        assert_eq!(
            q3_spawn_animation(),
            AnimationState::Q3 {
                legs: player_animation::LEGS_IDLE,
                torso: player_animation::TORSO_STAND,
                legs_timer_milliseconds: 0,
                torso_timer_milliseconds: 0,
            }
        );
    }

    #[test]
    fn idle_step_keeps_ready_state() {
        let step = step_q3_arsenal(
            &input(q3_spawn_loadout(provider(), Product::Baseq3, false)),
            &runtime(),
            &controls(),
            None,
        )
        .unwrap();
        assert!(step.effects.is_empty());
        assert!(step.torso_animations.is_empty());
        assert_eq!(step.runtime.event_sequence, 0);
        assert_eq!(step.arsenal.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
    }

    #[test]
    fn fire_step_consumes_ammo_and_reports() {
        let attacking = Q3ArsenalControls {
            attack: true,
            ..controls()
        };
        let step = step_q3_arsenal(
            &input(q3_spawn_loadout(provider(), Product::Baseq3, false)),
            &runtime(),
            &attacking,
            None,
        )
        .unwrap();
        let machinegun = step
            .arsenal
            .ammo
            .iter()
            .find(|entry| entry.item == "q3:ammo/machinegun")
            .unwrap();
        assert_eq!(machinegun.count, 99.0);
        assert_eq!(step.torso_animations, vec![player_animation::TORSO_ATTACK]);
        assert_eq!(step.runtime.event_sequence, 1);
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == 23
        )));
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Ammo { item, before, after } if item == "q3:ammo/machinegun" && *before == 100.0 && *after == 99.0
        )));
    }

    #[test]
    fn change_step_drops_and_rejects_bad_inputs() {
        let mut loadout = q3_spawn_loadout(provider(), Product::Baseq3, false);
        for entry in &mut loadout.ammo {
            if entry.item == "q3:weapon/shotgun" {
                entry.count = 1.0;
            }
        }
        let changing = Q3ArsenalControls {
            requested_weapon: weapon::SHOTGUN,
            ..controls()
        };
        let step = step_q3_arsenal(&input(loadout), &runtime(), &changing, None).unwrap();
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == 22
        )));
        let mut foreign = q3_spawn_loadout(provider(), Product::Baseq3, false);
        foreign.state = FamilyWeaponState::Q1 {
            frame: 0,
            attack_finished_seconds: 0.0,
            source_weapon: 0,
        };
        assert_eq!(
            step_q3_arsenal(&input(foreign), &runtime(), &controls(), None),
            Err(ArsenalError::NotQ3Arsenal)
        );
        let mut bad_clock = input(q3_spawn_loadout(provider(), Product::Baseq3, false));
        bad_clock.frame.elapsed = SourceTime::Milliseconds(-16);
        assert_eq!(
            step_q3_arsenal(&bad_clock, &runtime(), &controls(), None),
            Err(ArsenalError::BadClock)
        );
    }

    fn test_frame(elapsed: SourceTime) -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Milliseconds(100),
            elapsed,
            phase: FramePhase::FrameEntry,
        }
    }

    fn test_actor() -> (OwnedActor, ProviderId) {
        let owner = IdentityOwner::create("test").unwrap();
        let provider = ProviderId::new("q3", "test");
        let owned = owner.owned_actor(&owner.actor(3, 1), provider.clone()).unwrap();
        (owned, provider)
    }

    fn test_command() -> UserCommand {
        UserCommand::Q3(qa_world::movement::types::Q3UserCommand {
            server_time_milliseconds: 0,
            angle_words: [0, 0, 0],
            buttons: 0,
            weapon: weapon::MACHINEGUN,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        })
    }

    fn test_environment(health: f64) -> MovementEnvironment {
        MovementEnvironment {
            client_outputs: None,
            speed_multiplier: None,
            pose: None,
            health,
            flight: false,
            haste: false,
            invulnerable: false,
            gravity_multiplier: 1.0,
        }
    }

    fn q3_weapon(state: &FamilyWeaponState) -> (i32, i32, i32) {
        let FamilyWeaponState::Q3 {
            source_weapon,
            state,
            time_milliseconds,
        } = state
        else {
            panic!("q3 step keeps q3 weapon state");
        };
        (*source_weapon, *state, *time_milliseconds)
    }

    fn q3_torso(state: &AnimationState) -> i32 {
        let AnimationState::Q3 { torso, .. } = state else {
            panic!("q3 step keeps q3 animation");
        };
        *torso
    }

    fn arsenal_fixture() -> (WeaponStepInput, Q3ArsenalRuntimeState) {
        let (actor, provider) = test_actor();
        let arsenal = q3_spawn_loadout(provider.clone(), Product::Baseq3, false);
        let input = WeaponStepInput {
            actor,
            command: test_command(),
            frame: test_frame(SourceTime::Milliseconds(8)),
            arsenal,
            animation: ActorAnimationState {
                provider: provider.clone(),
                state: q3_spawn_animation(),
            },
            environment: test_environment(100.0),
            gauntlet_hit: false,
        };
        let runtime = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 0);
        (input, runtime)
    }

    #[test]
    fn spawn_loadout_and_weapon_requests() {
        let (_, provider) = test_actor();
        let base = q3_spawn_loadout(provider.clone(), Product::Baseq3, false);
        assert_eq!(base.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
        assert_eq!(q3_weapon(&base.state).0, weapon::MACHINEGUN);
        let mg_ammo = base
            .ammo
            .iter()
            .find(|entry| entry.item == "q3:ammo/machinegun")
            .unwrap();
        assert_eq!(mg_ammo.count, 100.0);
        assert!(base.ammo.iter().all(|entry| !entry.item.contains("nailgun")));
        let tdm = q3_spawn_loadout(provider.clone(), Product::Baseq3, true);
        assert_eq!(
            tdm.ammo
                .iter()
                .find(|entry| entry.item == "q3:ammo/machinegun")
                .unwrap()
                .count,
            50.0
        );
        let pack = q3_spawn_loadout(provider, Product::Missionpack, false);
        assert!(pack.ammo.iter().any(|entry| entry.item == "q3:weapon/nailgun"));

        let runtime = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 0);
        assert!(runtime.respawned);
        assert!(q3_request_weapon(&runtime, weapon::SHOTGUN).is_ok());
        assert!(q3_request_weapon(&runtime, weapon::NAILGUN).is_err());
        let pack_runtime = q3_spawn_arsenal_runtime(Product::Missionpack, 100.0, 0);
        assert!(q3_request_weapon(&pack_runtime, weapon::NAILGUN).is_ok());

        let holstered = q3_request_weapon_holster(&runtime);
        assert_eq!(holstered.external_slot, Q3ExternalWeaponSlot::HolsterRequested);
        let mut dropping = holstered.clone();
        dropping.external_slot = Q3ExternalWeaponSlot::Dropping;
        assert!(q3_request_weapon_resume(&dropping).is_err());
        let mut parked = runtime.clone();
        parked.external_slot = Q3ExternalWeaponSlot::Holstered;
        let resumed = q3_request_weapon_resume(&parked).unwrap();
        assert_eq!(resumed.external_slot, Q3ExternalWeaponSlot::ResumeRequested);
        let back = q3_request_weapon_holster(&resumed);
        assert_eq!(back.external_slot, Q3ExternalWeaponSlot::Holstered);
    }

    #[test]
    fn arsenal_step_fires_consumes_and_switches() {
        let (input, runtime) = arsenal_fixture();
        let ready = Q3ArsenalRuntimeState {
            respawned: false,
            ..runtime.clone()
        };
        let controls = Q3ArsenalControls {
            attack: true,
            use_holdable: false,
            requested_weapon: weapon::MACHINEGUN,
        };
        let step = step_q3_arsenal(&input, &ready, &controls, None).unwrap();
        assert_eq!(q3_weapon(&step.arsenal.state).1, weapon_state::FIRING);
        assert_eq!(
            step.arsenal
                .ammo
                .iter()
                .find(|entry| entry.item == "q3:ammo/machinegun")
                .unwrap()
                .count,
            99.0
        );
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == entity_event::FIRE_WEAPON
        )));
        assert_eq!(step.torso_animations, vec![player_animation::TORSO_ATTACK]);
        assert_ne!(q3_torso(&step.animation.state), q3_torso(&input.animation.state));

        let mut dry = input.clone();
        for entry in dry.arsenal.ammo.iter_mut() {
            if entry.item == "q3:ammo/machinegun" {
                entry.count = 0.0;
            }
        }
        let step = step_q3_arsenal(&dry, &ready, &controls, None).unwrap();
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == entity_event::NOAMMO
        )));
        assert_eq!(q3_weapon(&step.arsenal.state).2, 500);

        let mut stocked = input.clone();
        for entry in stocked.arsenal.ammo.iter_mut() {
            if entry.item == "q3:weapon/shotgun" {
                entry.count = 1.0;
            }
        }
        let switch = Q3ArsenalControls {
            attack: false,
            use_holdable: false,
            requested_weapon: weapon::SHOTGUN,
        };
        let step = step_q3_arsenal(&stocked, &ready, &switch, None).unwrap();
        assert_eq!(q3_weapon(&step.arsenal.state).1, weapon_state::DROPPING);
        assert_eq!(step.torso_animations, vec![player_animation::TORSO_DROP]);

        let mut dead = input.clone();
        dead.environment.health = 0.0;
        let step = step_q3_arsenal(&dead, &ready, &controls, None).unwrap();
        assert_eq!(q3_weapon(&step.arsenal.state).0, weapon::NONE);
        assert!(step.torso_animations.is_empty());

        let idle = Q3ArsenalControls {
            attack: false,
            use_holdable: false,
            requested_weapon: weapon::MACHINEGUN,
        };
        let step = step_q3_arsenal(&input, &runtime, &idle, None).unwrap();
        assert!(!step.runtime.respawned);

        let holdable = Q3ArsenalRuntimeState {
            respawned: false,
            holdable_item: 1,
            holdable_tag: holdable::MEDKIT,
            ..runtime.clone()
        };
        let use_controls = Q3ArsenalControls {
            attack: true,
            use_holdable: true,
            requested_weapon: weapon::MACHINEGUN,
        };
        let step = step_q3_arsenal(&input, &holdable, &use_controls, None).unwrap();
        assert_eq!(step.runtime.holdable_tag, 0);
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == entity_event::USE_ITEM0 + holdable::MEDKIT
        )));
    }

    #[test]
    fn arsenal_step_accepts_seconds_and_firing_delay() {
        let (mut input, runtime) = arsenal_fixture();
        input.frame = test_frame(SourceTime::Seconds(0.008));
        let ready = Q3ArsenalRuntimeState {
            respawned: false,
            ..runtime
        };
        let controls = Q3ArsenalControls {
            attack: true,
            use_holdable: false,
            requested_weapon: weapon::MACHINEGUN,
        };
        let delay = |milliseconds: i32| milliseconds * 2;
        let step = step_q3_arsenal(&input, &ready, &controls, Some(&delay)).unwrap();
        assert_eq!(q3_weapon(&step.arsenal.state).2, 200);
    }
}
