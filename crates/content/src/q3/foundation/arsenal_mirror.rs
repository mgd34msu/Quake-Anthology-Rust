//! SIBLING-MIRROR of `src/content/q3/foundation/arsenal.ts`.
//!
//! The canonical port is owned by sibling lane impl-content-q3 and will
//! union-merge at `crate::q3::foundation::arsenal`; this module keeps the
//! predecessor flat-port content so the foundation group compiles standalone.
//! Delete at unification and re-point imports at the canonical module.

use crate::contract::InventoryEntry;
use qa_core::identity::ProviderId;
use qa_core::time::SourceTime;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// arsenal.ts: PM_Weapon adapter and ClientSpawn loadout.
// ---------------------------------------------------------------------------

/// Weapon item mapping (`Q3WeaponItem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WeaponItem {
    /// Source weapon.
    pub weapon: i32,
    /// Weapon item.
    pub item: &'static str,
    /// Ammo item, when consumable.
    pub ammo: Option<&'static str>,
}

/// Weapon to item mapping (`Q3_WEAPON_ITEMS`).
pub const Q3_WEAPON_ITEMS: [Q3WeaponItem; 13] = [
    Q3WeaponItem {
        weapon: Q3Weapon::GAUNTLET,
        item: "q3:weapon/gauntlet",
        ammo: None,
    },
    Q3WeaponItem {
        weapon: Q3Weapon::MACHINEGUN,
        item: "q3:weapon/machinegun",
        ammo: Some("q3:ammo/machinegun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::SHOTGUN,
        item: "q3:weapon/shotgun",
        ammo: Some("q3:ammo/shotgun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::GRENADE_LAUNCHER,
        item: "q3:weapon/grenadelauncher",
        ammo: Some("q3:ammo/grenadelauncher"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::ROCKET_LAUNCHER,
        item: "q3:weapon/rocketlauncher",
        ammo: Some("q3:ammo/rocketlauncher"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::LIGHTNING,
        item: "q3:weapon/lightning",
        ammo: Some("q3:ammo/lightning"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::RAILGUN,
        item: "q3:weapon/railgun",
        ammo: Some("q3:ammo/railgun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::PLASMAGUN,
        item: "q3:weapon/plasmagun",
        ammo: Some("q3:ammo/plasmagun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::BFG,
        item: "q3:weapon/bfg",
        ammo: Some("q3:ammo/bfg"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::GRAPPLING_HOOK,
        item: "q3:weapon/grapple",
        ammo: None,
    },
    Q3WeaponItem {
        weapon: Q3Weapon::NAILGUN,
        item: "q3:weapon/nailgun",
        ammo: Some("q3:ammo/nailgun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::PROX_LAUNCHER,
        item: "q3:weapon/proxlauncher",
        ammo: Some("q3:ammo/proxlauncher"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::CHAINGUN,
        item: "q3:weapon/chaingun",
        ammo: Some("q3:ammo/chaingun"),
    },
];

/// Look up a weapon item (`q3WeaponItem`).
#[must_use]
pub fn q3_weapon_item(weapon: i32) -> Option<Q3WeaponItem> {
    Q3_WEAPON_ITEMS.iter().find(|entry| entry.weapon == weapon).copied()
}

/// Arsenal controls (`Q3ArsenalControls`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ArsenalControls {
    /// Attack held.
    pub attack: bool,
    /// Use-holdable held.
    pub use_holdable: bool,
    /// Requested weapon.
    pub requested_weapon: i32,
}

/// Arsenal runtime state (`Q3ArsenalRuntimeState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalRuntimeState {
    /// Product.
    pub product: Q3Product,
    /// Maximum health.
    pub max_health: i32,
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
    /// Fractional milliseconds.
    pub fractional_ms: f64,
    /// External slot.
    pub external_slot: Q3ExternalWeaponSlot,
    /// Requested weapon override.
    pub requested_weapon: Option<i32>,
}

/// Request a weapon (`q3RequestWeapon`).
pub fn q3_request_weapon(
    runtime: &Q3ArsenalRuntimeState,
    weapon: i32,
) -> Result<Q3ArsenalRuntimeState, Q3FoundationError> {
    let owned = Q3_WEAPON_ITEMS
        .iter()
        .any(|entry| entry.weapon == weapon && (runtime.product == Q3Product::MissionPack || weapon <= 10));
    if !owned {
        return Err(failed("Requested weapon does not belong to the Q3 product"));
    }
    let mut next = runtime.clone();
    next.requested_weapon = Some(weapon);
    Ok(next)
}

/// Request a holster (`q3RequestWeaponHolster`).
#[must_use]
pub fn q3_request_weapon_holster(runtime: &Q3ArsenalRuntimeState) -> Q3ArsenalRuntimeState {
    if runtime.external_slot == Q3ExternalWeaponSlot::ResumeRequested {
        let mut next = runtime.clone();
        next.external_slot = Q3ExternalWeaponSlot::Holstered;
        return next;
    }
    if runtime.external_slot == Q3ExternalWeaponSlot::Active {
        let mut next = runtime.clone();
        next.external_slot = Q3ExternalWeaponSlot::HolsterRequested;
        return next;
    }
    runtime.clone()
}

/// Request a resume (`q3RequestWeaponResume`).
pub fn q3_request_weapon_resume(runtime: &Q3ArsenalRuntimeState) -> Result<Q3ArsenalRuntimeState, Q3FoundationError> {
    if runtime.external_slot == Q3ExternalWeaponSlot::Active
        || runtime.external_slot == Q3ExternalWeaponSlot::ResumeRequested
    {
        return Ok(runtime.clone());
    }
    if runtime.external_slot != Q3ExternalWeaponSlot::Holstered {
        return Err(failed("Q3 primary must finish its source drop before resuming"));
    }
    let mut next = runtime.clone();
    next.external_slot = Q3ExternalWeaponSlot::ResumeRequested;
    Ok(next)
}

/// Arsenal step result (`Q3ArsenalStep`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalStep {
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Effects.
    pub effects: Vec<MovementEffect>,
    /// Runtime.
    pub runtime: Q3ArsenalRuntimeState,
    /// Torso animation requests.
    pub torso_animations: Vec<i32>,
}

/// Step the Q3 arsenal (`stepQ3Arsenal`).
pub fn step_q3_arsenal(
    input: &WeaponStepInput,
    runtime: &Q3ArsenalRuntimeState,
    controls: &Q3ArsenalControls,
    firing_delay: Option<&mut dyn FnMut(i32) -> i32>,
) -> Result<Q3ArsenalStep, Q3FoundationError> {
    let elapsed = match input.frame.elapsed {
        SourceTime::Milliseconds(value) => f64::from(value),
        SourceTime::Seconds(value) => f64::from(value) * 1000.0,
    };
    let clock = elapsed + runtime.fractional_ms;
    let msec_value = clock.trunc();
    if !clock.is_finite() || msec_value < 0.0 {
        return Err(range("Q3 weapon step clock must be finite and nonnegative"));
    }
    let msec = msec_value as i32;
    let mut owned_weapons = 0i32;
    for weapon in &Q3_WEAPON_ITEMS {
        let count = input
            .arsenal
            .ammo
            .iter()
            .find(|entry| entry.item == weapon.item)
            .map_or(0.0, |entry| entry.count);
        if count > 0.0 {
            owned_weapons |= 1 << weapon.weapon;
        }
    }
    let mut pm_flags = (if runtime.respawned { Q3MoveFlags::RESPAWNED } else { 0 })
        | (if runtime.use_item_held {
            Q3MoveFlags::USE_ITEM_HELD
        } else {
            0
        });
    if input.environment.health > 0 && !controls.attack && !controls.use_holdable {
        pm_flags &= !Q3MoveFlags::RESPAWNED;
    }
    let mut work = WeaponStepWork {
        product: runtime.product,
        pm_flags,
        weapon: input.arsenal.state.source_weapon,
        weapon_state: input.arsenal.state.state,
        weapon_time: input.arsenal.state.time_milliseconds,
        owned_weapons,
        health: input.environment.health,
        max_health: runtime.max_health,
        spectator: runtime.spectator,
        haste: input.environment.haste,
        persistent_powerup_tag: runtime.persistent_powerup_tag,
        holdable_item: runtime.holdable_item,
        holdable_tag: runtime.holdable_tag,
        entries: input.arsenal.ammo.clone(),
        provider: input.arsenal.provider.clone(),
        effects: Vec::new(),
        event_sequence: runtime.event_sequence,
        torso_requests: Vec::new(),
        external_slot: runtime.external_slot,
        buttons: (if controls.attack { Q3CommandButtons::ATTACK } else { 0 })
            | (if controls.use_holdable {
                Q3CommandButtons::USE_HOLDABLE
            } else {
                0
            }),
        requested_command_weapon: runtime.requested_weapon.unwrap_or(controls.requested_weapon),
        msec,
        gauntlet_hit: input.gauntlet_hit,
    };
    work.run(firing_delay)?;
    let mut animation = input.animation.clone();
    let mut effects = std::mem::take(&mut work.effects);
    let mut torso_animations = Vec::new();
    let event_sequence = work.event_sequence;
    for torso in work.torso_requests.drain(..) {
        torso_animations.push(torso);
        if input.environment.health <= 0 {
            continue;
        }
        let result = run_q3_torso_operation(
            torso,
            &Q3AnimationContext {
                animation: animation.clone(),
                dead: false,
                elapsed_ms: f64::from(msec),
                buttons: 0,
                product: runtime.product,
                event_sequence,
            },
            false,
        );
        effects.extend(result.effects);
        animation = result.animation;
    }
    let weapon = work.snapshot();
    let active_weapon = q3_weapon_item(work.weapon).map(|item| item.item.to_string());
    Ok(Q3ArsenalStep {
        arsenal: ArsenalState {
            provider: input.arsenal.provider.clone(),
            active_weapon,
            state: weapon.clone(),
            ammo: work.entries.clone(),
        },
        animation,
        effects,
        torso_animations,
        runtime: Q3ArsenalRuntimeState {
            holdable_item: work.holdable_item,
            holdable_tag: work.holdable_tag,
            respawned: work.pm_flags & Q3MoveFlags::RESPAWNED != 0,
            use_item_held: work.pm_flags & Q3MoveFlags::USE_ITEM_HELD != 0,
            fractional_ms: clock - msec_value,
            event_sequence: work.event_sequence,
            external_slot: work.external_slot,
            requested_weapon: if runtime.requested_weapon == Some(weapon.source_weapon) {
                None
            } else {
                runtime.requested_weapon
            },
            ..runtime.clone()
        },
    })
}

/// Spawn runtime state (`q3SpawnArsenalRuntime`).
#[must_use]
pub fn q3_spawn_arsenal_runtime(product: Q3Product, max_health: i32, event_sequence: i32) -> Q3ArsenalRuntimeState {
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
        fractional_ms: 0.0,
        external_slot: Q3ExternalWeaponSlot::Active,
        requested_weapon: None,
    }
}

/// Spawn loadout (`q3SpawnLoadout`).
#[must_use]
pub fn q3_spawn_loadout(provider: ProviderId, product: Q3Product, team_deathmatch: bool) -> ArsenalState {
    let mut ammo = Vec::new();
    for weapon in &Q3_WEAPON_ITEMS {
        if product == Q3Product::BaseQ3 && weapon.weapon >= Q3Weapon::NAILGUN {
            continue;
        }
        ammo.push(InventoryEntry {
            item: weapon.item.to_string(),
            count: if weapon.weapon == Q3Weapon::GAUNTLET || weapon.weapon == Q3Weapon::MACHINEGUN {
                1.0
            } else {
                0.0
            },
            capacity: 1.0,
            count_policy: None,
        });
        if let Some(rounds) = weapon.ammo {
            ammo.push(InventoryEntry {
                item: rounds.to_string(),
                count: if weapon.weapon == Q3Weapon::MACHINEGUN {
                    if team_deathmatch {
                        50.0
                    } else {
                        100.0
                    }
                } else {
                    0.0
                },
                capacity: 200.0,
                count_policy: None,
            });
        }
    }
    ArsenalState {
        provider,
        active_weapon: Some("q3:weapon/machinegun".to_string()),
        state: Q3WeaponState {
            source_weapon: Q3Weapon::MACHINEGUN,
            state: Q3WeaponPhase::READY,
            time_milliseconds: 0,
        },
        ammo,
    }
}

/// Spawn animation (`q3SpawnAnimation`).
#[must_use]
pub fn q3_spawn_animation() -> Q3AnimationState {
    Q3AnimationState {
        legs: Q3PlayerAnimation::LEGS_IDLE,
        torso: Q3PlayerAnimation::TORSO_STAND,
        legs_timer_ms: 0.0,
        torso_timer_ms: 0.0,
    }
}
