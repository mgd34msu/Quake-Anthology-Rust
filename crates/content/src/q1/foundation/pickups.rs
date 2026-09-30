//! Q1 pickup behavior (`src/content/q1/foundation/pickups.ts`).
//!
//! items.qc, Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.
//!
//! Supply preview and the source-backpack admission port the used
//! surface of `SharedPickupAdmission`/`previewPickupGrants` (donor
//! `src/world/gameplay/pickups.ts`) over the host inventory table;
//! the full port belongs to the world lane. Pickup takes dispatch by
//! [`Q1TakeKind`]; original-grant touches run through
//! [`touch_q1_pickup`] with a [`Q1TouchTake`] take object.

use std::cell::RefCell;
use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_core::time::SourceTime;

use crate::contract::{
    InventoryEntry, ItemId, OriginalPickupContinuation, OriginalPickupOffer, PickupAmmoGrant, PickupAmmoReceipt,
    PickupCargoEntry, PickupCargoKind, PickupCount, PickupOwner, PickupResource, PickupRoute, PickupSelection,
    PickupSupplyObservation, PickupSupplyOffer, PickupSupplyPreview, PickupWeaponGrant, ProtectionChannel,
    RegularArmorState,
};

use super::entity_services::{ammo_item, Q1EntityServices};
use super::types::{
    vadd, Q1AutoSwitch, Q1BaseWeapon, Q1Edition, Q1Effect, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel,
    Q1TraceRequest, Q1Weapon, WEAPONS, ZERO,
};
use crate::q1::{q1_error, q1_range, Q1Error};

/// Pickup take outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1PickupTake {
    /// Take refused.
    Refused,
    /// Take accepted.
    Taken,
    /// Take accepted; the pickup stays.
    Leave,
}

/// Pickup collision bounds selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1PickupBounds {
    /// Small box.
    Box,
    /// Weapon box.
    Weapon,
    /// Artifact box.
    Artifact,
}

/// Pickup take kind with captured definition values.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1TakeKind {
    /// Health grant.
    Health {
        /// Big-health flag.
        big: bool,
        /// Megahealth flag.
        mega: bool,
        /// Grant amount.
        amount: f64,
    },
    /// Armor grant.
    Armor {
        /// Absorption.
        absorption: f64,
        /// Armor points.
        points: f64,
        /// Armor item.
        item: ItemId,
    },
    /// Weapon grant.
    Weapon {
        /// Weapon.
        weapon: Q1BaseWeapon,
    },
    /// Ammo grant.
    Ammo {
        /// Ammo offer.
        offer: PickupAmmoGrant,
    },
    /// Key grant.
    Key {
        /// Key item.
        item: ItemId,
    },
    /// Powerup grant.
    Powerup {
        /// Powerup kind.
        kind: Q1Powerup,
    },
    /// Backpack cargo grant.
    Backpack {
        /// Cargo entries.
        cargo: Vec<PickupCargoEntry>,
    },
}

/// Pickup definition (`Pickup`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PickupDef {
    /// Backpack cargo, if any.
    pub cargo: Option<Vec<PickupCargoEntry>>,
    /// Supply offer, if any.
    pub supply: Option<PickupSupplyOffer>,
    /// Model path.
    pub model: String,
    /// Pickup sound.
    pub sound: String,
    /// Bounds selector.
    pub bounds: Q1PickupBounds,
    /// Model skin.
    pub skin: i32,
    /// Respawn interval in seconds.
    pub respawn: f64,
    /// Take kind.
    pub take: Q1TakeKind,
}

/// Weapon supply offer (`weaponOffer`).
#[must_use]
pub fn weapon_offer(weapon: Q1Weapon) -> PickupWeaponGrant {
    let amount = if weapon == Q1Weapon::Nailgun || weapon == Q1Weapon::Supernailgun {
        30.0
    } else if weapon == Q1Weapon::Lightning {
        15.0
    } else {
        5.0
    };
    PickupWeaponGrant {
        item: super::types::weapon_item(weapon),
        ammo: ammo_item(weapon)
            .map(|item| vec![PickupAmmoGrant { item, amount }])
            .unwrap_or_default(),
    }
}

/// Weapon selection rank (`rank`; unknown weapons sort before the
/// roster, donor `indexOf`).
#[must_use]
pub fn q1_pickup_rank(weapon: Q1Weapon) -> i32 {
    const ORDER: [Q1Weapon; 8] = [
        Q1Weapon::Lightning,
        Q1Weapon::Rocketlauncher,
        Q1Weapon::Supernailgun,
        Q1Weapon::Grenadelauncher,
        Q1Weapon::Supershotgun,
        Q1Weapon::Nailgun,
        Q1Weapon::Shotgun,
        Q1Weapon::Axe,
    ];
    ORDER
        .iter()
        .position(|candidate| *candidate == weapon)
        .map(|index| index as i32)
        .unwrap_or(-1)
}

/// Pickup definition for an entity (`pickupDefinition`).
pub fn pickup_definition(game: &Q1EntityServices, id: &ActorId) -> Result<Option<Q1PickupDef>, Q1Error> {
    let entity = game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let name = entity.classname.clone();
    let big = (entity.spawnflags & if name == "item_weapon" { 8 } else { 1 }) != 0;
    if name == "item_health" {
        let mega = !big && (entity.spawnflags & 2) != 0;
        let amount = if big {
            15.0
        } else if mega {
            100.0
        } else {
            25.0
        };
        let model = if big {
            10
        } else if mega {
            100
        } else {
            25
        };
        let sound = if big {
            "items/r_item1.wav"
        } else if mega {
            "items/r_item2.wav"
        } else {
            "items/health1.wav"
        };
        return Ok(Some(Q1PickupDef {
            cargo: None,
            supply: None,
            model: format!("maps/b_bh{model}.bsp"),
            sound: sound.to_string(),
            bounds: Q1PickupBounds::Box,
            skin: 0,
            respawn: if mega { 120.0 } else { 20.0 },
            take: Q1TakeKind::Health { big, mega, amount },
        }));
    }
    if name == "item_armor1" || name == "item_armor2" || name == "item_armorInv" {
        let absorption = if name == "item_armor1" {
            0.3
        } else if name == "item_armor2" {
            0.6
        } else {
            0.8
        };
        let points = if name == "item_armor1" {
            100.0
        } else if name == "item_armor2" {
            150.0
        } else {
            200.0
        };
        let skin = if name == "item_armor1" {
            0
        } else if name == "item_armor2" {
            1
        } else {
            2
        };
        return Ok(Some(Q1PickupDef {
            cargo: None,
            supply: None,
            model: String::from("progs/armor.mdl"),
            sound: String::from("items/armor1.wav"),
            bounds: Q1PickupBounds::Weapon,
            skin,
            respawn: 20.0,
            take: Q1TakeKind::Armor {
                absorption,
                points,
                item: format!("q1:{name}"),
            },
        }));
    }
    if let Some(weapon) = WEAPONS
        .iter()
        .find(|candidate| name == format!("weapon_{}", candidate.as_str()))
        .copied()
    {
        if weapon != Q1BaseWeapon::Axe && weapon != Q1BaseWeapon::Shotgun {
            let model = match weapon {
                Q1BaseWeapon::Supershotgun => "g_shot",
                Q1BaseWeapon::Nailgun => "g_nail",
                Q1BaseWeapon::Supernailgun => "g_nail2",
                Q1BaseWeapon::Grenadelauncher => "g_rock",
                Q1BaseWeapon::Rocketlauncher => "g_rock2",
                Q1BaseWeapon::Lightning => "g_light",
                _ => "g_shot",
            };
            return Ok(Some(Q1PickupDef {
                cargo: None,
                supply: Some(PickupSupplyOffer::Weapon(weapon_offer(Q1Weapon::from(weapon)))),
                model: format!("progs/{model}.mdl"),
                sound: String::from("weapons/pkup.wav"),
                bounds: Q1PickupBounds::Weapon,
                skin: 0,
                respawn: 30.0,
                take: Q1TakeKind::Weapon { weapon },
            }));
        }
    }
    let ammo = if name == "item_weapon" && (entity.spawnflags & 2) != 0 {
        Some(("q1:ammo/rockets", "rock", if big { 10.0 } else { 5.0 }))
    } else if name == "item_weapon" && (entity.spawnflags & 4) != 0 {
        Some(("q1:ammo/nails", "nail", if big { 40.0 } else { 20.0 }))
    } else if (name == "item_weapon" && (entity.spawnflags & 1) != 0) || name == "item_shells" {
        Some(("q1:ammo/shells", "shell", if big { 40.0 } else { 20.0 }))
    } else if name == "item_spikes" {
        Some(("q1:ammo/nails", "nail", if big { 50.0 } else { 25.0 }))
    } else if name == "item_rockets" {
        Some(("q1:ammo/rockets", "rock", if big { 10.0 } else { 5.0 }))
    } else if name == "item_cells" {
        Some(("q1:ammo/cells", "batt", if big { 12.0 } else { 6.0 }))
    } else {
        None
    };
    if let Some((item, model, amount)) = ammo {
        let deathmatch = game.options().deathmatch;
        return Ok(Some(Q1PickupDef {
            cargo: None,
            supply: Some(PickupSupplyOffer::Ammo(PickupAmmoGrant {
                item: item.to_string(),
                amount,
            })),
            model: format!("maps/b_{model}{}.bsp", if big { 1 } else { 0 }),
            sound: String::from("weapons/lock4.wav"),
            bounds: Q1PickupBounds::Box,
            skin: 0,
            respawn: if deathmatch == 3 || deathmatch == 5 { 15.0 } else { 30.0 },
            take: Q1TakeKind::Ammo {
                offer: PickupAmmoGrant {
                    item: item.to_string(),
                    amount,
                },
            },
        }));
    }
    if name == "item_key1" || name == "item_key2" {
        let item = if name == "item_key1" {
            "q1:key/silver"
        } else {
            "q1:key/gold"
        };
        let prefix = if game.world_type == 0 {
            "w"
        } else if game.world_type == 1 {
            "m"
        } else {
            "b"
        };
        let sound = if game.world_type == 2 {
            "misc/basekey.wav"
        } else if game.world_type == 1 {
            "misc/runekey.wav"
        } else {
            "misc/medkey.wav"
        };
        return Ok(Some(Q1PickupDef {
            cargo: None,
            supply: None,
            model: format!(
                "progs/{prefix}_{}.mdl",
                if name == "item_key1" { "s_key" } else { "g_key" }
            ),
            sound: sound.to_string(),
            bounds: Q1PickupBounds::Weapon,
            skin: 0,
            respawn: -1.0,
            take: Q1TakeKind::Key { item: item.to_string() },
        }));
    }
    let powerup = if name == "item_artifact_invulnerability" {
        Some((Q1Powerup::Invulnerability, "invulner", "protect"))
    } else if name == "item_artifact_invisibility" {
        Some((Q1Powerup::Invisibility, "invisibl", "inv1"))
    } else if name == "item_artifact_envirosuit" {
        Some((Q1Powerup::Suit, "suit", "suit"))
    } else if name == "item_artifact_super_damage" {
        Some((Q1Powerup::Quad, "quaddama", "damage"))
    } else {
        None
    };
    if let Some((kind, model, sound)) = powerup {
        return Ok(Some(Q1PickupDef {
            cargo: None,
            supply: None,
            model: format!("progs/{model}.mdl"),
            sound: format!("items/{sound}.wav"),
            bounds: Q1PickupBounds::Artifact,
            skin: 0,
            respawn: if kind == Q1Powerup::Invulnerability || kind == Q1Powerup::Invisibility {
                300.0
            } else {
                60.0
            },
            take: Q1TakeKind::Powerup { kind },
        }));
    }
    if name == "item_backpack" {
        let counters: [(&str, ItemId); 4] = [
            ("shells", String::from("q1:ammo/shells")),
            ("nails", String::from("q1:ammo/nails")),
            ("rockets", String::from("q1:ammo/rockets")),
            ("cells", String::from("q1:ammo/cells")),
        ];
        let mut cargo: Vec<PickupCargoEntry> = counters
            .iter()
            .map(|(field, item)| PickupCargoEntry {
                kind: PickupCargoKind::Counter,
                item: item.clone(),
                count: entity.number(field),
            })
            .collect();
        let carried = entity.text("weapon");
        if !carried.is_empty() {
            let weapon = WEAPONS.iter().find(|weapon| {
                weapon.as_str() == carried || super::types::weapon_item(Q1Weapon::from(**weapon)) == carried
            });
            match weapon {
                Some(weapon) => cargo.push(PickupCargoEntry {
                    kind: PickupCargoKind::Weapon,
                    item: super::types::weapon_item(Q1Weapon::from(*weapon)),
                    count: 1.0,
                }),
                None => return Err(q1_error(format!("Unsupported Q1 backpack weapon {carried}"))),
            }
        }
        return Ok(Some(Q1PickupDef {
            cargo: Some(cargo.clone()),
            supply: None,
            model: String::from("progs/backpack.mdl"),
            sound: String::from("weapons/lock4.wav"),
            bounds: Q1PickupBounds::Artifact,
            skin: 0,
            respawn: -1.0,
            take: Q1TakeKind::Backpack { cargo },
        }));
    }
    Ok(None)
}

/// Weapon leave/owned eligibility (`weaponEligibility`).
pub struct Q1WeaponEligibility {
    /// Whether the pickup stays.
    pub leave: bool,
    /// Whether the weapon is owned.
    pub owned: bool,
}

/// Resolve weapon leave/owned eligibility.
pub fn weapon_eligibility(
    game: &mut Q1EntityServices,
    player: &ActorId,
    item: &ItemId,
) -> Result<Q1WeaponEligibility, Q1Error> {
    let hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_leave);
    let leave = match hook {
        Some(leave) => leave(game)?,
        None => game.options().coop || [2, 3, 5].contains(&game.options().deathmatch),
    };
    let owned = match game.pickup_admission.as_ref() {
        Some(admission) => admission.owns(player, item),
        None => game.host.inventory.count(player, item) > 0.0,
    };
    Ok(Q1WeaponEligibility { leave, owned })
}

/// Run an ammo-pickup selection (`q1AmmoPickupSelection`).
pub fn q1_ammo_pickup_selection(
    game: &mut Q1EntityServices,
    player: &ActorId,
    before: Q1Weapon,
    auto_switch: bool,
) -> Result<(), Q1Error> {
    let current = game.player_ref(player).map(|player| player.weapon);
    if auto_switch && current == Some(before) {
        let owned = game.player_owned(player).expect("player");
        let best = game.choose_best(&owned, None)?;
        game.select_weapon(&owned, best)?;
    }
    Ok(())
}

/// Run a weapon-pickup selection (`q1WeaponPickupSelection`).
pub fn q1_weapon_pickup_selection(
    game: &mut Q1EntityServices,
    player: &ActorId,
    weapon: Q1Weapon,
    selection: PickupSelection,
) -> Result<(), Q1Error> {
    let rank_hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_rank);
    let current = game
        .player_ref(player)
        .map(|player| player.weapon)
        .unwrap_or(Q1Weapon::Axe);
    let better = selection == PickupSelection::Always
        || selection == PickupSelection::Better
            && rank_hook
                .map(|rank| rank(weapon))
                .unwrap_or_else(|| q1_pickup_rank(weapon))
                < rank_hook
                    .map(|rank| rank(current))
                    .unwrap_or_else(|| q1_pickup_rank(current));
    if better {
        let owned = game.player_owned(player).expect("player");
        game.select_weapon(&owned, weapon)?;
    }
    Ok(())
}

fn take_weapon(game: &mut Q1EntityServices, player: &ActorId, weapon: Q1Weapon) -> Result<Q1PickupTake, Q1Error> {
    let offer = weapon_offer(weapon);
    let eligibility = weapon_eligibility(game, player, &offer.item)?;
    if eligibility.leave && eligibility.owned {
        return Ok(Q1PickupTake::Refused);
    }
    if game.pickup_admission.is_some() {
        let hook = game.pickup_rules.as_ref().and_then(|rules| rules.auto_switch);
        let auto_switch = match hook {
            Some(auto_switch) => auto_switch(game, player, eligibility.owned)?,
            None => {
                let states = game.player_ref(player).cloned().expect("player");
                states.auto_switch == Q1AutoSwitch::Always
                    || states.auto_switch == Q1AutoSwitch::New && !eligibility.owned
            }
        };
        let selection = if !auto_switch {
            PickupSelection::Never
        } else if game.options().deathmatch == 0 {
            PickupSelection::Always
        } else {
            PickupSelection::Better
        };
        let owned = game.player_owned(player).expect("player");
        let accepted = game
            .pickup_admission
            .as_ref()
            .expect("admission")
            .weapon(&owned, &offer, selection);
        if !accepted {
            return Ok(Q1PickupTake::Refused);
        }
        return Ok(if eligibility.leave {
            Q1PickupTake::Leave
        } else {
            Q1PickupTake::Taken
        });
    }
    let owned = game.player_owned(player).expect("player");
    game.host.inventory.give(&owned, &offer.item, 1.0);
    let granted_hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_granted);
    let selected = match granted_hook {
        Some(granted) => granted(game, player, weapon)?,
        None => weapon,
    };
    for ammo in &offer.ammo {
        let grant_hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_ammo_grant);
        let amount = match grant_hook {
            Some(grant) => grant(game, player, weapon, ammo.amount)?,
            None => ammo.amount,
        };
        game.host.inventory.give(&owned, &ammo.item, amount);
    }
    let switch_hook = game.pickup_rules.as_ref().and_then(|rules| rules.auto_switch);
    let auto_switch = match switch_hook {
        Some(auto_switch) => auto_switch(game, player, eligibility.owned)?,
        None => {
            let states = game.player_ref(player).cloned().expect("player");
            states.auto_switch == Q1AutoSwitch::Always || states.auto_switch == Q1AutoSwitch::New && !eligibility.owned
        }
    };
    if auto_switch {
        let selection = if game.options().deathmatch == 0 {
            PickupSelection::Always
        } else {
            PickupSelection::Better
        };
        q1_weapon_pickup_selection(game, player, selected, selection)?;
    }
    Ok(if eligibility.leave {
        Q1PickupTake::Leave
    } else {
        Q1PickupTake::Taken
    })
}

/// Run a pickup take (`Pickup["take"]`).
pub fn take_pickup(game: &mut Q1EntityServices, kind: &Q1TakeKind, player: &ActorId) -> Result<Q1PickupTake, Q1Error> {
    match kind {
        Q1TakeKind::Health { mega, amount, .. } => {
            let health = game.health(player);
            let max_health = game.player_ref(player).map(|player| player.max_health).unwrap_or(0.0);
            if health <= 0.0 || health >= if *mega { 250.0 } else { max_health } {
                return Ok(Q1PickupTake::Refused);
            }
            let owned = game.player_owned(player).expect("player");
            let cap = if *mega { 250.0 } else { max_health };
            game.host.combat.set_health(&owned, (health + amount).min(cap))?;
            Ok(Q1PickupTake::Taken)
        }
        Q1TakeKind::Armor {
            absorption,
            points,
            item,
        } => {
            let armor = game.host.combat.read(player).map(|combat| combat.armor.regular.clone());
            if matches!(armor, Some(RegularArmorState::Source { .. })) {
                return Ok(Q1PickupTake::Refused);
            }
            let current = match armor {
                None | Some(RegularArmorState::None) => 0.0,
                Some(RegularArmorState::Q1 { points, absorption, .. }) => points * absorption,
                Some(RegularArmorState::Q2 {
                    points,
                    normal_protection,
                    ..
                }) => points * normal_protection,
                Some(RegularArmorState::Q3 { points, protection, .. }) => points * protection,
                Some(RegularArmorState::Source { .. }) => 0.0,
            };
            if current >= absorption * points {
                return Ok(Q1PickupTake::Refused);
            }
            let owned = game.player_owned(player).expect("player");
            game.host.combat.set_regular_armor(
                &owned,
                &RegularArmorState::Q1 {
                    points: *points,
                    absorption: *absorption,
                    item: item.clone(),
                },
            )?;
            Ok(Q1PickupTake::Taken)
        }
        Q1TakeKind::Weapon { weapon } => take_weapon(game, player, Q1Weapon::from(*weapon)),
        Q1TakeKind::Ammo { offer } => {
            if let Some(admission) = game.pickup_admission.as_ref() {
                let owned = game.player_owned(player).expect("player");
                let accepted = admission.ammo(&owned, offer, true);
                return Ok(if accepted {
                    Q1PickupTake::Taken
                } else {
                    Q1PickupTake::Refused
                });
            }
            let owned = game.player_owned(player).expect("player");
            let before = game.choose_best(&owned, None)?;
            if game.host.inventory.give(&owned, &offer.item, offer.amount) == 0.0 {
                return Ok(Q1PickupTake::Refused);
            }
            let auto_switch = game
                .player_ref(player)
                .is_some_and(|player| player.auto_switch != Q1AutoSwitch::Never);
            q1_ammo_pickup_selection(game, player, before, auto_switch)?;
            Ok(Q1PickupTake::Taken)
        }
        Q1TakeKind::Key { item } => {
            let owned = game.player_owned(player).expect("player");
            if game.host.inventory.give(&owned, item, 1.0) == 0.0 {
                return Ok(Q1PickupTake::Refused);
            }
            Ok(if game.options().coop {
                Q1PickupTake::Leave
            } else {
                Q1PickupTake::Taken
            })
        }
        Q1TakeKind::Powerup { kind } => {
            game.give_powerup(player, *kind, 30.0)?;
            Ok(Q1PickupTake::Taken)
        }
        Q1TakeKind::Backpack { cargo } => {
            let admission = Q1SharedPickupAdmission::backpack();
            let owned = game.player_owned(player).expect("player");
            let selection = if game.options().deathmatch == 0 {
                PickupSelection::Always
            } else {
                PickupSelection::Better
            };
            let (accepted, grants) = admission.cargo(game, &owned, cargo, selection)?;
            for (item, selection) in grants {
                let weapon = WEAPONS
                    .iter()
                    .find(|weapon| super::types::weapon_item(Q1Weapon::from(**weapon)) == item)
                    .copied()
                    .ok_or_else(|| q1_error("Q1 backpack selected a foreign weapon"))?;
                q1_weapon_pickup_selection(game, player, Q1Weapon::from(weapon), selection)?;
            }
            Ok(if accepted {
                Q1PickupTake::Taken
            } else {
                Q1PickupTake::Refused
            })
        }
    }
}

/// Ammo acceptance mode for preview plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1PreviewAcceptance {
    /// Any nonzero grant accepts.
    Positive,
    /// Alias for [`Q1PreviewAcceptance::Positive`] matching the donor's
    /// `"nonzero"` spelling.
    Nonzero,
}

/// Grant preview plan (`PickupGrantPlan`, donor
/// `world/gameplay/pickups.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1GrantPlan {
    /// Weapon plan.
    Weapon {
        /// Weapon grants.
        weapons: Vec<PickupAmmoGrant>,
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
    },
    /// Ammo plan with bundled weapon grants.
    Ammo {
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
        /// Bundled weapon grants.
        weapons: Vec<PickupAmmoGrant>,
        /// Acceptance mode.
        acceptance: Q1PreviewAcceptance,
    },
    /// Ammo plan with shared-ammo weapon items.
    SharedAmmo {
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
        /// Shared weapon items.
        weapons: Vec<ItemId>,
        /// Acceptance mode.
        acceptance: Q1PreviewAcceptance,
    },
}

fn source_count(entry: &InventoryEntry, value: f64) -> Result<f64, Q1Error> {
    if !value.is_finite() {
        return Err(q1_range("Inventory counter must be finite"));
    }
    match entry.count_policy {
        None | Some(crate::contract::InventoryCountPolicy::Stack) => {
            if value < 0.0 {
                return Err(q1_range("Inventory quantity must be finite and nonnegative"));
            }
            Ok(value)
        }
        Some(crate::contract::InventoryCountPolicy::SourceCounter(arithmetic)) => {
            use crate::contract::SourceCounterArithmetic;
            match arithmetic {
                SourceCounterArithmetic::Binary32 => {
                    let rounded = f64::from(value as f32);
                    if !rounded.is_finite() {
                        return Err(q1_range("Inventory counter exceeds binary32 range"));
                    }
                    Ok(rounded)
                }
                SourceCounterArithmetic::Binary64 => Ok(value),
                SourceCounterArithmetic::Int32 => Ok(f64::from(value as i32)),
            }
        }
    }
}

fn inventory_give(entry: &InventoryEntry, count: f64) -> Result<(Option<InventoryEntry>, f64), Q1Error> {
    if !count.is_finite() || count < 0.0 {
        return Err(q1_range("Inventory quantity must be finite and nonnegative"));
    }
    if entry.capacity < 0.0 || !entry.capacity.is_finite() {
        return Err(q1_range("Inventory quantity must be finite and nonnegative"));
    }
    let given = count.min((entry.capacity - entry.count).max(0.0));
    if given == 0.0 {
        return Ok((None, 0.0));
    }
    let next = InventoryEntry {
        count: source_count(entry, entry.count + source_count(entry, given)?)?,
        ..entry.clone()
    };
    let delta = next.count - entry.count;
    Ok((Some(next), delta))
}

/// Preview a grant plan without mutating (`previewPickupGrants`,
/// donor `world/gameplay/pickups.ts`).
pub fn preview_pickup_grants(inventory: &[InventoryEntry], plan: &Q1GrantPlan) -> Result<PickupSupplyPreview, Q1Error> {
    let (weapons, ammo, acceptance, shared) = match plan {
        Q1GrantPlan::Weapon { weapons, ammo } => (
            weapons.iter().map(|grant| grant.item.clone()).collect::<Vec<_>>(),
            ammo.clone(),
            None,
            None,
        ),
        Q1GrantPlan::Ammo {
            ammo,
            weapons,
            acceptance,
        } => (
            weapons.iter().map(|grant| grant.item.clone()).collect::<Vec<_>>(),
            ammo.clone(),
            Some(*acceptance),
            None,
        ),
        Q1GrantPlan::SharedAmmo {
            ammo,
            weapons,
            acceptance,
        } => (weapons.clone(), ammo.clone(), Some(*acceptance), Some(weapons.clone())),
    };
    let mut entries: HashMap<ItemId, InventoryEntry> = inventory
        .iter()
        .map(|entry| (entry.item.clone(), entry.clone()))
        .collect();
    for item in weapons.iter().chain(ammo.iter().map(|grant| &grant.item)) {
        if !entries.contains_key(item) {
            return Err(q1_error(format!("Pickup destination {item} was not admitted")));
        }
    }
    if let Some(shared) = shared {
        for item in &shared {
            if !ammo.iter().any(|grant| &grant.item == item) {
                return Err(q1_error(format!("Shared weapon {item} has no ammo grant")));
            }
        }
    }
    let mut give = |grant: &PickupAmmoGrant| -> Result<PickupAmmoReceipt, Q1Error> {
        let entry = entries
            .get(&grant.item)
            .ok_or_else(|| q1_error(format!("Pickup destination {} was not admitted", grant.item)))?
            .clone();
        let (next, given) = inventory_give(&entry, grant.amount)?;
        if let Some(next) = next {
            entries.insert(grant.item.clone(), next);
        }
        Ok(PickupAmmoReceipt {
            item: grant.item.clone(),
            before: entry.count,
            given,
        })
    };
    match plan {
        Q1GrantPlan::Weapon { weapons, ammo } => {
            let mut receipts = Vec::with_capacity(weapons.len());
            for grant in weapons {
                receipts.push(give(grant)?);
            }
            let mut ammo_receipts = Vec::with_capacity(ammo.len());
            for grant in ammo {
                ammo_receipts.push(give(grant)?);
            }
            Ok(PickupSupplyPreview {
                accepted: true,
                ammo: ammo_receipts,
                weapons: receipts,
            })
        }
        _ => {
            let mut ammo_receipts = Vec::with_capacity(ammo.len());
            for grant in &ammo {
                ammo_receipts.push(give(grant)?);
            }
            let positive = matches!(acceptance, Some(Q1PreviewAcceptance::Positive));
            let accepted = ammo_receipts.iter().any(|receipt| {
                if positive {
                    receipt.given > 0.0
                } else {
                    receipt.given != 0.0
                }
            });
            if !accepted {
                return Ok(PickupSupplyPreview {
                    accepted: false,
                    ammo: ammo_receipts,
                    weapons: Vec::new(),
                });
            }
            match plan {
                Q1GrantPlan::SharedAmmo { weapons, .. } => {
                    let receipts = ammo_receipts
                        .iter()
                        .filter(|receipt| weapons.contains(&receipt.item))
                        .cloned()
                        .collect();
                    Ok(PickupSupplyPreview {
                        accepted,
                        ammo: ammo_receipts,
                        weapons: receipts,
                    })
                }
                Q1GrantPlan::Ammo { weapons, .. } => {
                    let mut receipts = Vec::new();
                    for grant in weapons {
                        match ammo_receipts.iter().find(|receipt| receipt.item == grant.item) {
                            Some(receipt) => receipts.push(receipt.clone()),
                            None => receipts.push(give(grant)?),
                        }
                    }
                    Ok(PickupSupplyPreview {
                        accepted,
                        ammo: ammo_receipts,
                        weapons: receipts,
                    })
                }
                Q1GrantPlan::Weapon { .. } => unreachable!("handled above"),
            }
        }
    }
}

/// Source-backpack supply profile (`SharedPickupAdmission` profile,
/// donor `world/gameplay/pickups.ts`). The donor's `weaponOwnership`
/// is the `"all-destinations"` literal, so the every-destination rule
/// is hardcoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SupplyProfile {
    /// Profile id.
    pub id: ItemId,
    /// Ammo routes.
    pub ammo: Vec<PickupRoute>,
    /// Weapon owner overrides.
    pub weapon_owners: Vec<PickupOwner>,
    /// Ammo owner overrides.
    pub ammo_owners: Vec<PickupOwner>,
    /// Weapon routes.
    pub weapons: Vec<PickupRoute>,
}

/// Source pickup admission over the host inventory table
/// (`SharedPickupAdmission`, donor `world/gameplay/pickups.ts`). Only
/// the cargo path Q1 uses is ported; weapon grants return for the
/// caller to select instead of invoking game-bound closures.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SharedPickupAdmission {
    profile: Q1SupplyProfile,
}

impl Q1SharedPickupAdmission {
    /// Source-backpack profile admission.
    #[must_use]
    pub fn backpack() -> Self {
        Self {
            profile: Q1SupplyProfile {
                id: String::from("q1:source-backpack"),
                ammo: ["q1:ammo/shells", "q1:ammo/nails", "q1:ammo/rockets", "q1:ammo/cells"]
                    .iter()
                    .map(|item| PickupRoute {
                        source: item.to_string(),
                        destinations: vec![item.to_string()],
                    })
                    .collect(),
                weapon_owners: Vec::new(),
                ammo_owners: Vec::new(),
                weapons: WEAPONS
                    .iter()
                    .map(|weapon| {
                        let item = super::types::weapon_item(Q1Weapon::from(*weapon));
                        PickupRoute {
                            source: item.clone(),
                            destinations: vec![item],
                        }
                    })
                    .collect(),
            },
        }
    }

    /// Fresh admission with validation (donor constructor).
    pub fn new(profile: Q1SupplyProfile) -> Result<Self, Q1Error> {
        use std::collections::HashSet;
        for mappings in [&profile.ammo, &profile.weapons] {
            let mut sources = HashSet::new();
            for mapping in mappings {
                if !sources.insert(mapping.source.clone()) {
                    return Err(q1_error(format!("Duplicate pickup mapping for {}", mapping.source)));
                }
                if mapping.destinations.iter().collect::<HashSet<_>>().len() != mapping.destinations.len() {
                    return Err(q1_error(format!("Duplicate pickup destination for {}", mapping.source)));
                }
            }
        }
        Ok(Self { profile })
    }

    fn destinations(&self, weapons: bool, item: &ItemId) -> Result<&[ItemId], Q1Error> {
        let mappings = if weapons {
            &self.profile.weapons
        } else {
            &self.profile.ammo
        };
        mappings
            .iter()
            .find(|mapping| &mapping.source == item)
            .map(|mapping| mapping.destinations.as_slice())
            .ok_or_else(|| {
                q1_error(format!(
                    "Pickup supply {} has no {} mapping for {item}",
                    self.profile.id,
                    if weapons { "weapons" } else { "ammo" }
                ))
            })
    }

    fn require_entries(&self, game: &Q1EntityServices, actor: &ActorId, items: &[ItemId]) -> Result<(), Q1Error> {
        let inventory = game.host.inventory.entries(actor);
        for item in items {
            if !inventory.iter().any(|entry| &entry.item == item) {
                return Err(q1_error(format!("Pickup destination {item} was not admitted")));
            }
        }
        Ok(())
    }

    fn resolve_ammo(&self, offers: &[PickupAmmoGrant]) -> Result<Vec<PickupAmmoGrant>, Q1Error> {
        let mut resolved = Vec::new();
        for offer in offers {
            if !offer.amount.is_finite() || offer.amount < 0.0 {
                return Err(q1_range("Pickup amount must be finite and nonnegative"));
            }
            for item in self.destinations(false, &offer.item)? {
                resolved.push(PickupAmmoGrant {
                    item: item.clone(),
                    amount: offer.amount,
                });
            }
        }
        Ok(resolved)
    }

    /// Grant backpack cargo, returning acceptance plus weapon grants
    /// for the caller to select (`cargo`).
    pub fn cargo(
        &self,
        game: &mut Q1EntityServices,
        actor: &OwnedActor,
        cargo: &[PickupCargoEntry],
        selection: PickupSelection,
    ) -> Result<(bool, Vec<(ItemId, PickupSelection)>), Q1Error> {
        use std::collections::HashSet;
        if cargo.iter().map(|row| &row.item).collect::<HashSet<_>>().len() != cargo.len()
            || cargo
                .iter()
                .any(|row| !row.count.is_finite() || row.kind == PickupCargoKind::Weapon && row.count != 1.0)
        {
            return Err(q1_error("Invalid pickup cargo"));
        }
        let mut weapons = Vec::new();
        let mut seen = HashSet::new();
        for row in cargo.iter().filter(|row| row.kind == PickupCargoKind::Weapon) {
            for destination in self.destinations(true, &row.item)? {
                if seen.insert(destination.clone()) {
                    weapons.push(destination.clone());
                }
            }
        }
        let counters: Vec<PickupAmmoGrant> = cargo
            .iter()
            .filter(|row| row.kind == PickupCargoKind::Counter)
            .map(|row| PickupAmmoGrant {
                item: row.item.clone(),
                amount: row.count,
            })
            .collect();
        let ammo = self.resolve_ammo(&counters)?;
        let mut required = weapons.clone();
        required.extend(ammo.iter().map(|grant| grant.item.clone()));
        self.require_entries(game, actor.id(), &required)?;
        for weapon in &weapons {
            game.host.inventory.give(actor, weapon, 1.0);
        }
        for grant in &ammo {
            game.host.inventory.give(actor, &grant.item, grant.amount);
        }
        let grants = if weapons.is_empty() {
            Vec::new()
        } else {
            weapons.iter().map(|weapon| (weapon.clone(), selection)).collect()
        };
        Ok((true, grants))
    }
}

/// Original-grant take for an original-pickup touch. Implemented by
/// the foundation pickup flow and downstream custom touches.
pub trait Q1TouchTake {
    /// Touch eligibility check.
    fn eligible(&mut self, _game: &mut Q1EntityServices) -> Result<bool, Q1Error> {
        Ok(true)
    }
    /// Run the original source grant.
    fn original(&mut self, game: &mut Q1EntityServices) -> Result<bool, Q1Error>;
    /// Complete a settled attempt.
    fn complete(&mut self, game: &mut Q1EntityServices, taken: bool) -> Result<(), Q1Error>;
}

/// Touch-scoped guarded continuation. The take object runs
/// synchronously inside the touch call, so the raw game pointer never
/// outlives the borrow; fallible take errors stash for the touch
/// caller since the shared continuation API is infallible.
struct Q1GuardedTake<T: Q1TouchTake> {
    game: *mut Q1EntityServices,
    entity: ActorId,
    other: ActorId,
    take: RefCell<T>,
    error: RefCell<Option<Q1Error>>,
}

impl<T: Q1TouchTake> Q1GuardedTake<T> {
    #[allow(clippy::mut_from_ref)]
    fn game(&self) -> &mut Q1EntityServices {
        // SAFETY: the touch call is synchronous; the pointer never
        // outlives the borrowed game.
        unsafe { &mut *self.game }
    }

    fn live(&self) -> bool {
        let game = self.game();
        game.entity_present(&self.entity).is_some() && game.is_live(&self.entity) && game.is_live(&self.other)
    }
}

impl<T: Q1TouchTake> OriginalPickupContinuation for Q1GuardedTake<T> {
    fn eligible(&self) -> bool {
        let game = self.game();
        match self.take.borrow_mut().eligible(game) {
            Ok(eligible) => eligible,
            Err(error) => {
                *self.error.borrow_mut() = Some(error);
                false
            }
        }
    }

    fn original(&self) -> bool {
        let game = self.game();
        match self.take.borrow_mut().original(game) {
            Ok(taken) => taken,
            Err(error) => {
                *self.error.borrow_mut() = Some(error);
                false
            }
        }
    }

    fn complete(&self, taken: bool) {
        if !self.live() {
            return;
        }
        let game = self.game();
        if let Err(error) = self.take.borrow_mut().complete(game, taken) {
            *self.error.borrow_mut() = Some(error);
        }
    }
}

/// Run an original-grant pickup touch (`touchQ1Pickup`).
pub fn touch_q1_pickup<T: Q1TouchTake>(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    item: ItemId,
    default_resource: Option<PickupResource>,
    take: T,
    cargo: Option<&[PickupCargoEntry]>,
) -> Result<(), Q1Error> {
    let live = game.entity_present(id).is_some() && game.is_live(id) && game.is_live(other);
    if !live {
        return Ok(());
    }
    let entity = game.entity_ref(id).cloned().expect("pickup");
    let guarded = Q1GuardedTake {
        game: game as *mut Q1EntityServices,
        entity: id.clone(),
        other: other.clone(),
        take: RefCell::new(take),
        error: RefCell::new(None),
    };
    // SAFETY: `guarded` only runs synchronously below while `game` is
    // exclusively borrowed through it; the original `game` borrow is
    // shadowed until the touch settles.
    let game_shadowed: &mut Q1EntityServices = unsafe { &mut *guarded.game };
    let shadowed_time = game_shadowed.time;
    match game_shadowed.host.original_pickups.as_ref() {
        Some(admission) => {
            let offer = OriginalPickupOffer {
                recipient: other.clone(),
                pickup: id.clone(),
                source: entity.actor.owner().clone(),
                item,
                default_resource,
                count: if entity.count == 0.0 {
                    PickupCount::Default
                } else {
                    PickupCount::Override { amount: entity.count }
                },
                dropped: entity.classname == "item_backpack",
                time: SourceTime::Seconds(shadowed_time as f32),
                cargo: cargo.unwrap_or(&[]).to_vec(),
                grant: None,
            };
            admission.touch(&offer, &guarded);
        }
        None => {
            if guarded.eligible() && guarded.live() {
                guarded.complete(guarded.original());
            }
        }
    }
    if let Some(error) = guarded.error.into_inner() {
        return Err(error);
    }
    Ok(())
}

/// Foundation pickup take state.
struct Q1PickupTakeState {
    entity: ActorId,
    player: ActorId,
    key: Option<ItemId>,
    sound: String,
    cargo: Option<Vec<PickupCargoEntry>>,
    respawn: f64,
    supply_weapon: bool,
    take: Q1TakeKind,
    leave: bool,
    original_ran: bool,
}

impl Q1TouchTake for Q1PickupTakeState {
    fn original(&mut self, game: &mut Q1EntityServices) -> Result<bool, Q1Error> {
        self.original_ran = true;
        let result = take_pickup(game, &self.take, &self.player)?;
        self.leave = result == Q1PickupTake::Leave;
        Ok(result != Q1PickupTake::Refused)
    }

    fn complete(&mut self, game: &mut Q1EntityServices, taken: bool) -> Result<(), Q1Error> {
        if !taken {
            return Ok(());
        }
        if !self.original_ran {
            let hook = game.pickup_rules.as_ref().and_then(|rules| rules.weapon_leave);
            self.leave = if self.supply_weapon {
                match hook {
                    Some(leave) => leave(game)?,
                    None => game.options().coop || [2, 3, 5].contains(&game.options().deathmatch),
                }
            } else {
                self.key.is_some() && game.options().coop
            };
        }
        let entity = game.entity_ref(&self.entity).cloned().expect("pickup");
        if game.options().edition == Q1Edition::Rerelease
            && entity.classname == "item_health"
            && (entity.spawnflags & 3) == 2
        {
            let time = game.time;
            game.update_player(&self.player, |player| player.mega_rot_at = time + 5.0)?;
        }
        let owned = game.player_owned(&self.player).expect("player");
        game.sound(owned.id(), &self.sound, Q1SoundChannel::Item, 1.0, 1.0)?;
        let origin = game.body(&self.entity).map(|body| body.origin)?;
        game.effect(Q1Effect::Pickup, origin, Some(&self.player), 1);
        if self.cargo.is_some() {
            return game.remove(&self.entity);
        }
        if self.leave {
            if !entity.classname.starts_with("weapon_") {
                let other = self.player.clone();
                game.use_targets(&self.entity, Some(&other))?;
            }
            return Ok(());
        }
        game.update_entity(&self.entity, |entity| {
            entity.solid = Q1Solid::None;
            entity.model.clear();
        })?;
        game.link(&self.entity)?;
        let respawn_hook = game.pickup_rules.as_ref().and_then(|rules| rules.respawn);
        let respawn = match respawn_hook {
            Some(respawn) => respawn(game, &self.entity, self.respawn)?,
            None => self.respawn,
        };
        let entity = game.entity_ref(&self.entity).cloned().expect("pickup");
        let respawns = game.options().deathmatch != 0
            && respawn > 0.0
            && (game.options().deathmatch != 2 || entity.classname.starts_with("item_artifact_"));
        if game.options().edition == Q1Edition::Classic
            && entity.classname == "item_health"
            && (entity.spawnflags & 3) == 2
        {
            let player = self.player.clone();
            game.update_entity(&self.entity, |entity| entity.owner = Some(player))?;
            game.schedule(&self.entity, 5.0, "health_rot")?;
        } else if respawns {
            game.schedule(&self.entity, respawn, "SUB_regen")?;
        } else {
            game.cancel(&self.entity);
        }
        let other = self.player.clone();
        game.use_targets(&self.entity, Some(&other))
    }
}

fn pickup_player(game: &mut Q1EntityServices, other: &ActorId) -> Option<ActorId> {
    if game.health(other) <= 0.0 {
        return None;
    }
    game.player_owned(other).map(|owned| owned.id().clone())
}

fn pickup_touch(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let definition = pickup_definition(game, &id)?
        .ok_or_else(|| q1_error(format!("Unknown saved Q1 pickup: {}", entity.classname)))?;
    if entity.solid != Q1Solid::Trigger {
        return Ok(());
    }
    let player = match pickup_player(game, &other) {
        Some(player) => player,
        None => return Ok(()),
    };
    let key = if entity.classname == "item_key1" {
        Some(String::from("q1:key/silver"))
    } else if entity.classname == "item_key2" {
        Some(String::from("q1:key/gold"))
    } else {
        None
    };
    let item = match &definition.supply {
        Some(PickupSupplyOffer::Ammo(offer)) => offer.item.clone(),
        Some(PickupSupplyOffer::Weapon(offer)) => offer.item.clone(),
        Some(PickupSupplyOffer::AmmoWeapon(offer)) => offer.item.clone(),
        None => key.clone().unwrap_or_else(|| format!("q1:{}", entity.classname)),
    };
    let armor = ["item_armor1", "item_armor2", "item_armorInv"].contains(&entity.classname.as_str());
    let resource = if armor {
        Some(PickupResource::Protection {
            channel: ProtectionChannel::Regular,
        })
    } else if definition.supply.is_some() || key.is_some() {
        Some(PickupResource::Inventory { item: item.clone() })
    } else {
        None
    };
    let take = Q1PickupTakeState {
        entity: id.clone(),
        player: player.clone(),
        key,
        sound: definition.sound.clone(),
        cargo: definition.cargo.clone(),
        respawn: definition.respawn,
        supply_weapon: matches!(definition.supply, Some(PickupSupplyOffer::Weapon(_))),
        take: definition.take.clone(),
        leave: false,
        original_ran: false,
    };
    touch_q1_pickup(game, &id, &other, item, resource, take, definition.cargo.as_deref())
}

/// Null also covers unsupported source callbacks or native custom
/// weapon grants; it is not proof of absence (`observeQ1Supply`).
pub fn observe_q1_supply(
    game: &mut Q1EntityServices,
    pickup: &ActorId,
    other: &ActorId,
) -> Option<PickupSupplyObservation> {
    let entity = game.entity_ref(pickup)?.clone();
    let offer = pickup_definition(game, pickup).ok()??.supply?;
    if entity.touch.as_deref() != Some("item_touch") {
        return None;
    }
    if matches!(offer, PickupSupplyOffer::Weapon(_))
        && game.pickup_admission.is_none()
        && game
            .pickup_rules
            .as_ref()
            .is_some_and(|rules| rules.weapon_granted.is_some() || rules.weapon_ammo_grant.is_some())
    {
        return None;
    }
    if entity.solid == Q1Solid::Trigger {
        let player = pickup_player(game, other);
        let weapon = match (&player, &offer) {
            (Some(player), PickupSupplyOffer::Weapon(offer)) => weapon_eligibility(game, player, &offer.item).ok(),
            _ => None,
        };
        let eligible = player.is_some() && !weapon.is_some_and(|weapon| weapon.leave && weapon.owned);
        return Some(PickupSupplyObservation {
            actor: pickup.clone(),
            offer,
            availability: crate::contract::PickupAvailability::Ready { eligible },
        });
    }
    let regenerating = entity.next_think >= 0.0 && entity.think.as_deref() == Some("SUB_regen");
    Some(PickupSupplyObservation {
        actor: pickup.clone(),
        offer,
        availability: if regenerating {
            crate::contract::PickupAvailability::Respawning {
                at_seconds: entity.next_think,
            }
        } else {
            crate::contract::PickupAvailability::Inactive
        },
    })
}

/// Preview a supply offer for a recipient (`previewQ1Supply`).
pub fn preview_q1_supply(
    game: &mut Q1EntityServices,
    pickup: &ActorId,
    recipient: &ActorId,
) -> Result<Option<PickupSupplyPreview>, Q1Error> {
    let observation = match observe_q1_supply(game, pickup, recipient) {
        Some(observation) => observation,
        None => return Ok(None),
    };
    let offer = observation.offer;
    if let Some(admission) = game.pickup_admission.as_ref() {
        return Ok(Some(admission.preview(recipient, &offer)));
    }
    match offer {
        PickupSupplyOffer::Weapon(offer) => {
            let entries = game.host.inventory.entries(recipient);
            preview_pickup_grants(
                &entries,
                &Q1GrantPlan::Weapon {
                    weapons: vec![PickupAmmoGrant {
                        item: offer.item.clone(),
                        amount: 1.0,
                    }],
                    ammo: offer.ammo.clone(),
                },
            )
            .map(Some)
        }
        PickupSupplyOffer::Ammo(offer) => {
            let entries = game.host.inventory.entries(recipient);
            preview_pickup_grants(
                &entries,
                &Q1GrantPlan::Ammo {
                    ammo: vec![offer],
                    weapons: Vec::new(),
                    acceptance: Q1PreviewAcceptance::Nonzero,
                },
            )
            .map(Some)
        }
        PickupSupplyOffer::AmmoWeapon(_) => Ok(None),
    }
}

/// Spawn a pickup from an entity (`spawnPickup`).
pub fn spawn_pickup(game: &mut Q1EntityServices, id: &ActorId) -> Result<bool, Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let definition = match pickup_definition(game, id)? {
        Some(definition) => definition,
        None => return Ok(false),
    };
    if game.uses_id1_precaches() {
        if entity.classname == "item_weapon" {
            let size = if (entity.spawnflags & 8) != 0 { 1 } else { 0 };
            if (entity.spawnflags & 1) != 0 {
                game.precache_model(&format!("maps/b_shell{size}.bsp"))?;
            }
            if (entity.spawnflags & 4) != 0 {
                game.precache_model(&format!("maps/b_nail{size}.bsp"))?;
            }
            if (entity.spawnflags & 2) != 0 {
                game.precache_model(&format!("maps/b_rock{size}.bsp"))?;
            }
        } else if entity.classname != "item_backpack" {
            game.precache_model(&definition.model)?;
        }
        if entity.classname == "item_health" || entity.classname == "item_key1" || entity.classname == "item_key2" {
            game.precache_sound(&definition.sound)?;
        } else if entity.classname == "item_artifact_invulnerability" {
            game.precache_sound("items/protect.wav")?;
            game.precache_sound("items/protect2.wav")?;
            game.precache_sound("items/protect3.wav")?;
        } else if entity.classname == "item_artifact_envirosuit" {
            game.precache_sound("items/suit.wav")?;
            game.precache_sound("items/suit2.wav")?;
        } else if entity.classname == "item_artifact_invisibility" {
            game.precache_sound("items/inv1.wav")?;
            game.precache_sound("items/inv2.wav")?;
            game.precache_sound("items/inv3.wav")?;
        } else if entity.classname == "item_artifact_super_damage" {
            game.precache_sound("items/damage.wav")?;
            game.precache_sound("items/damage2.wav")?;
            game.precache_sound("items/damage3.wav")?;
        }
    }
    let model = definition.model.clone();
    let skin = definition.skin;
    let bounds = match definition.bounds {
        Q1PickupBounds::Box => Bounds {
            min: ZERO,
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 56.0,
            },
        },
        Q1PickupBounds::Weapon => Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 56.0,
            },
        },
        Q1PickupBounds::Artifact => Bounds {
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
    };
    game.update_entity(id, |entity| {
        entity.model = model;
        entity.skin = skin;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })?;
    let original = game
        .entity_ref(id)
        .map(|entity| entity.model.clone())
        .unwrap_or_default();
    game.update_entity(id, |entity| entity.original_model = original)?;
    game.set_bounds(id, bounds)?;
    let touch = game.named.touch("item_touch")?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    if entity.classname == "item_backpack" {
        game.update_entity(id, |entity| {
            entity.solid = Q1Solid::Trigger;
            entity.movement = Q1MoveType::Toss;
            entity.movement_flags = 256;
        })?;
        game.set_bounds(
            id,
            Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: 0.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 56.0,
                },
            },
        )?;
        let rx = game.host.random() * 200.0 - 100.0;
        let ry = game.host.random() * 200.0 - 100.0;
        game.set_body(
            id,
            &super::gameplay::BodyPatch {
                velocity: Some(Vec3 {
                    x: rx as f32,
                    y: ry as f32,
                    z: 300.0,
                }),
                ..Default::default()
            },
        )?;
        game.link(id)?;
        return Ok(true);
    }
    // PlaceItem waits until all brush entities exist, lifts six units,
    // then drops 256 units.
    game.schedule(id, 0.2, "PlaceItem")?;
    Ok(true)
}

/// Give a pickup directly (`givePickup`).
pub fn give_pickup(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<bool, Q1Error> {
    if pickup_definition(game, id)?.is_none() {
        return Ok(false);
    }
    game.update_entity(id, |entity| entity.solid = Q1Solid::Trigger)?;
    pickup_touch(game, id, other)?;
    Ok(true)
}

fn place_item(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let body = game.body(&id)?;
    let start = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 6.0 });
    let trace = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(
            start,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -256.0,
            },
        ),
        bounds: body.bounds,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if trace.all_solid || trace.fraction == 1.0 {
        return game.remove(&id);
    }
    game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
        entity.movement_flags = 256 | 512;
    })?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            origin: Some(trace.end),
            velocity: Some(ZERO),
            ground: Some(trace.actor.clone()),
            ..Default::default()
        },
    )?;
    game.link(&id)
}

fn sub_regen(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let original = game
        .entity_ref(&id)
        .map(|entity| entity.original_model.clone())
        .unwrap_or_default();
    game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.model = original;
    })?;
    game.sound_simple(&id, "items/itembk2.wav")?;
    game.link(&id)
}

fn health_rot(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let owner = game.entity_ref(&id).and_then(|entity| entity.owner.clone());
    let player = owner.as_ref().and_then(|owner| game.player_owned(owner));
    if let Some(player) = player {
        if game.health(player.id())
            > game
                .player_ref(player.id())
                .map(|player| player.max_health)
                .unwrap_or(0.0)
        {
            let health = game.health(player.id());
            game.host.combat.set_health(&player, health - 1.0)?;
            return game.schedule(&id, 1.0, "health_rot");
        }
    }
    if game.options().deathmatch == 1 {
        return game.schedule(&id, 20.0, "SUB_regen");
    }
    Ok(())
}

/// Register pickup callbacks.
pub fn register_pickup_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "item_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| { pickup_touch(game, id, other) },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "PlaceItem",
        super::callbacks::Q1CallbackHandlers {
            action: Some(place_item),
            ..Default::default()
        },
    )?;
    game.named.register(
        "SUB_regen",
        super::callbacks::Q1CallbackHandlers {
            action: Some(sub_regen),
            ..Default::default()
        },
    )?;
    game.named.register(
        "health_rot",
        super::callbacks::Q1CallbackHandlers {
            action: Some(health_rot),
            ..Default::default()
        },
    )?;
    Ok(())
}
