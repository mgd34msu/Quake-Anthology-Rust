//! Q1-to-Q3 and Q1-Hipnotic pickup supply composition.
//!
//! Donor provenance:
//! `src/content/composition/q1-q3-supply.ts`,
//! `src/content/composition/q1-hipnotic-supply.ts`.
//!
//! The donor's `weaponOwnership: "all-destinations"` marker has no
//! counterpart in [`PickupSupplyProfile`], so it is dropped, and the donor
//! declares no owner overrides, so both owner lists are empty here (same
//! convention as [`crate::composition`]).
//!
//! The donor loadout returns the movement contract's `ArsenalState`, which
//! lives in `qa-world`; `qa-world` depends on `qa-content`, so this module
//! mirrors that result in [`Q1Q3SupplyLoadout`] instead of naming it.

use qa_core::identity::ProviderId;

use crate::contract::{InventoryEntry, ItemId, PickupRoute, PickupSupplyProfile};

/// Q3 product selector (`Product` in
/// `src/content/q3/base/shared/definitions.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q3SupplyProduct {
    /// Base Quake III (`"baseq3"`).
    #[default]
    BaseQ3,
    /// Mission pack (`"missionpack"`).
    Missionpack,
}

impl Q3SupplyProduct {
    /// Donor selector text (`"baseq3"`, `"missionpack"`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q3SupplyProduct::BaseQ3 => "baseq3",
            Q3SupplyProduct::Missionpack => "missionpack",
        }
    }
}

/// Q3 weapon numbers (`Weapon` in `src/movement/q3/constants.ts`; the
/// canonical Rust port lives in `qa-world`, which `qa-content` cannot name).
const WP_GAUNTLET: i32 = 1;
const WP_MACHINEGUN: i32 = 2;
const WP_SHOTGUN: i32 = 3;
const WP_NAILGUN: i32 = 11;

/// Q3 ready weapon state (`WeaponState.WEAPON_READY` in
/// `src/movement/q3/constants.ts`).
const WEAPON_READY: i32 = 0;

/// Q3 weapon inventory rows in `Q3_WEAPON_ITEMS` order
/// (`src/content/q3/foundation/arsenal.ts`): weapon number, weapon item,
/// ammo item.
const Q3_WEAPON_ITEMS: &[(i32, &str, Option<&str>)] = &[
    (WP_GAUNTLET, "q3:weapon/gauntlet", None),
    (WP_MACHINEGUN, "q3:weapon/machinegun", Some("q3:ammo/machinegun")),
    (WP_SHOTGUN, "q3:weapon/shotgun", Some("q3:ammo/shotgun")),
    (4, "q3:weapon/grenadelauncher", Some("q3:ammo/grenadelauncher")),
    (5, "q3:weapon/rocketlauncher", Some("q3:ammo/rocketlauncher")),
    (6, "q3:weapon/lightning", Some("q3:ammo/lightning")),
    (7, "q3:weapon/railgun", Some("q3:ammo/railgun")),
    (8, "q3:weapon/plasmagun", Some("q3:ammo/plasmagun")),
    (9, "q3:weapon/bfg", Some("q3:ammo/bfg")),
    (10, "q3:weapon/grapple", None),
    (WP_NAILGUN, "q3:weapon/nailgun", Some("q3:ammo/nailgun")),
    (12, "q3:weapon/proxlauncher", Some("q3:ammo/proxlauncher")),
    (13, "q3:weapon/chaingun", Some("q3:ammo/chaingun")),
];

/// Build a supply route from string literals.
fn route(source: &str, destinations: &[&str]) -> PickupRoute {
    PickupRoute {
        source: source.to_string(),
        destinations: destinations.iter().map(ToString::to_string).collect(),
    }
}

/// Build an inventory entry without a count policy (donor entries omit
/// `countPolicy`, which selects a nonnegative stack).
fn entry(item: &str, count: f64, capacity: f64) -> InventoryEntry {
    InventoryEntry {
        item: item.to_string(),
        count,
        capacity,
        count_policy: None,
    }
}

/// Base Q1 authored pickups with Q1 progression and selected base Q3
/// weapons (`Q1_Q3_SUPPLY_PROFILE` in `q1-q3-supply.ts`).
#[must_use]
pub fn q1_q3_supply_profile() -> PickupSupplyProfile {
    PickupSupplyProfile {
        id: "composition:q1-base-q3-supply".to_string(),
        ammo: vec![
            route("q1:ammo/shells", &["q3:ammo/shotgun"]),
            route("q1:ammo/nails", &["q3:ammo/machinegun"]),
            route(
                "q1:ammo/rockets",
                &["q3:ammo/rocketlauncher", "q3:ammo/grenadelauncher"],
            ),
            route("q1:ammo/cells", &["q3:ammo/lightning"]),
        ],
        weapon_owners: Vec::new(),
        ammo_owners: Vec::new(),
        weapons: vec![
            route("q1:weapon/axe", &["q3:weapon/gauntlet"]),
            route("q1:weapon/shotgun", &["q3:weapon/shotgun"]),
            route("q1:weapon/supershotgun", &["q3:weapon/shotgun"]),
            route("q1:weapon/nailgun", &["q3:weapon/machinegun"]),
            route("q1:weapon/supernailgun", &["q3:weapon/machinegun"]),
            route("q1:weapon/grenadelauncher", &["q3:weapon/grenadelauncher"]),
            route("q1:weapon/rocketlauncher", &["q3:weapon/rocketlauncher"]),
            route("q1:weapon/lightning", &["q3:weapon/lightning"]),
        ],
    }
}

/// Hipnotic weapon grants layered onto the Q1-to-Q3 profile
/// (`Q1_HIPNOTIC_SUPPLY_PROFILE` in `q1-hipnotic-supply.ts`): ammo routes
/// collapse to self-destinations and weapon routes become self plus the
/// Hipnotic extras.
#[must_use]
pub fn q1_hipnotic_supply_profile() -> PickupSupplyProfile {
    const HIPNOTIC: &[(&str, &[&str])] = &[
        (
            "q1:weapon/lightning",
            &["q1:weapon/hipnotic:laser", "q1:weapon/hipnotic:mjolnir"],
        ),
        ("q1:weapon/grenadelauncher", &["q1:weapon/hipnotic:proximity"]),
    ];
    let base = q1_q3_supply_profile();
    let ammo = base.ammo.iter().map(|row| route(&row.source, &[&row.source])).collect();
    let weapons = base
        .weapons
        .iter()
        .map(|row| {
            let mut destinations = vec![row.source.clone()];
            if let Some((_, added)) = HIPNOTIC.iter().find(|(source, _)| *source == row.source) {
                destinations.extend(added.iter().map(ToString::to_string));
            }
            PickupRoute {
                source: row.source.clone(),
                destinations,
            }
        })
        .collect();
    PickupSupplyProfile {
        id: "composition:q1-hipnotic-supply".to_string(),
        ammo,
        weapon_owners: base.weapon_owners.clone(),
        ammo_owners: base.ammo_owners.clone(),
        weapons,
    }
}

/// Q1-to-Q3 starting arsenal (`q1Q3SupplyLoadout` in `q1-q3-supply.ts`),
/// mirrored content-locally because `qa-content` cannot name
/// `qa-world`'s `ArsenalState`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Q3SupplyLoadout {
    /// Owning provider.
    pub provider: ProviderId,
    /// Active weapon (`"q3:weapon/shotgun"`).
    pub active_weapon: ItemId,
    /// Source weapon number (`WP_SHOTGUN`).
    pub source_weapon: i32,
    /// Weapon state word (`WEAPON_READY`).
    pub weapon_state: i32,
    /// Weapon timer in milliseconds.
    pub time_milliseconds: i32,
    /// Ammo counters: gauntlet and shotgun owned, 25 shotgun shells.
    pub ammo: Vec<InventoryEntry>,
}

/// Q3 spawn inventory table (`q3SpawnLoadout` in
/// `src/content/q3/foundation/arsenal.ts`) with `teamDeathmatch: false`:
/// gauntlet and machinegun owned, 100 machinegun rounds.
fn q3_spawn_loadout_entries(product: Q3SupplyProduct) -> Vec<InventoryEntry> {
    let mut ammo = Vec::new();
    for (weapon, item, ammo_item) in Q3_WEAPON_ITEMS {
        if product == Q3SupplyProduct::BaseQ3 && *weapon >= WP_NAILGUN {
            continue;
        }
        let owned = *weapon == WP_GAUNTLET || *weapon == WP_MACHINEGUN;
        ammo.push(entry(item, f64::from(u8::from(owned)), 1.0));
        if let Some(ammo_item) = ammo_item {
            let count = if *weapon == WP_MACHINEGUN { 100.0 } else { 0.0 };
            ammo.push(entry(ammo_item, count, 200.0));
        }
    }
    ammo
}

/// Starting Q3 shotgun arsenal for Q1-progression play
/// (`q1Q3SupplyLoadout`). The donor throws when the spawn loadout reports a
/// foreign weapon state; the spawn table built here is always Q3, so there
/// is no error path.
#[must_use]
pub fn q1_q3_supply_loadout(provider: ProviderId, product: Q3SupplyProduct) -> Q1Q3SupplyLoadout {
    let ammo = q3_spawn_loadout_entries(product)
        .into_iter()
        .map(|row| {
            let count = if row.item == "q3:weapon/gauntlet" || row.item == "q3:weapon/shotgun" {
                1.0
            } else if row.item == "q3:ammo/shotgun" {
                25.0
            } else {
                0.0
            };
            InventoryEntry { count, ..row }
        })
        .collect();
    Q1Q3SupplyLoadout {
        provider,
        active_weapon: "q3:weapon/shotgun".to_string(),
        source_weapon: WP_SHOTGUN,
        weapon_state: WEAPON_READY,
        time_milliseconds: 0,
        ammo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_of(loadout: &Q1Q3SupplyLoadout, item: &str) -> f64 {
        loadout
            .ammo
            .iter()
            .find(|row| row.item == item)
            .unwrap_or_else(|| panic!("missing loadout entry for {item}"))
            .count
    }

    #[test]
    fn q1_q3_routes_cover_q1_progression() {
        let profile = q1_q3_supply_profile();
        assert_eq!(profile.id, "composition:q1-base-q3-supply");
        assert_eq!(profile.ammo.len(), 4);
        assert_eq!(profile.weapons.len(), 8);
        assert!(profile.weapon_owners.is_empty());
        assert!(profile.ammo_owners.is_empty());
        let rockets = profile.ammo.iter().find(|row| row.source == "q1:ammo/rockets").unwrap();
        assert_eq!(
            rockets.destinations,
            vec!["q3:ammo/rocketlauncher", "q3:ammo/grenadelauncher"]
        );
        let axe = profile
            .weapons
            .iter()
            .find(|row| row.source == "q1:weapon/axe")
            .unwrap();
        assert_eq!(axe.destinations, vec!["q3:weapon/gauntlet"]);
    }

    #[test]
    fn q1_hipnotic_layers_extras_on_self_routes() {
        let profile = q1_hipnotic_supply_profile();
        assert_eq!(profile.id, "composition:q1-hipnotic-supply");
        assert_eq!(profile.ammo.len(), 4);
        for row in &profile.ammo {
            assert_eq!(row.destinations, vec![row.source.clone()]);
        }
        assert_eq!(profile.weapons.len(), 8);
        let lightning = profile
            .weapons
            .iter()
            .find(|row| row.source == "q1:weapon/lightning")
            .unwrap();
        assert_eq!(
            lightning.destinations,
            vec![
                "q1:weapon/lightning",
                "q1:weapon/hipnotic:laser",
                "q1:weapon/hipnotic:mjolnir"
            ]
        );
        let grenade = profile
            .weapons
            .iter()
            .find(|row| row.source == "q1:weapon/grenadelauncher")
            .unwrap();
        assert_eq!(
            grenade.destinations,
            vec!["q1:weapon/grenadelauncher", "q1:weapon/hipnotic:proximity"]
        );
        let shotgun = profile
            .weapons
            .iter()
            .find(|row| row.source == "q1:weapon/shotgun")
            .unwrap();
        assert_eq!(shotgun.destinations, vec!["q1:weapon/shotgun"]);
    }

    #[test]
    fn q1_q3_loadout_starts_with_shotgun() {
        let provider = ProviderId::new("test", "spawn");
        let loadout = q1_q3_supply_loadout(provider.clone(), Q3SupplyProduct::BaseQ3);
        assert_eq!(loadout.provider, provider);
        assert_eq!(loadout.active_weapon, "q3:weapon/shotgun");
        assert_eq!(loadout.source_weapon, WP_SHOTGUN);
        assert_eq!(loadout.weapon_state, WEAPON_READY);
        assert_eq!(loadout.time_milliseconds, 0);
        assert_eq!(count_of(&loadout, "q3:weapon/gauntlet"), 1.0);
        assert_eq!(count_of(&loadout, "q3:weapon/shotgun"), 1.0);
        assert_eq!(count_of(&loadout, "q3:weapon/machinegun"), 0.0);
        assert_eq!(count_of(&loadout, "q3:ammo/shotgun"), 25.0);
        assert_eq!(count_of(&loadout, "q3:ammo/machinegun"), 0.0);
        assert_eq!(loadout.ammo.len(), 18);
        assert!(loadout.ammo.iter().all(|row| row.count_policy.is_none()));
    }

    #[test]
    fn q1_q3_loadout_product_selects_missionpack_rows() {
        let provider = ProviderId::new("test", "spawn");
        let loadout = q1_q3_supply_loadout(provider, Q3SupplyProduct::Missionpack);
        assert_eq!(count_of(&loadout, "q3:weapon/nailgun"), 0.0);
        assert_eq!(count_of(&loadout, "q3:ammo/nailgun"), 0.0);
        assert_eq!(count_of(&loadout, "q3:weapon/chaingun"), 0.0);
        assert_eq!(loadout.ammo.len(), 24);
    }

    #[test]
    fn supply_product_names_match_donor() {
        assert_eq!(Q3SupplyProduct::BaseQ3.as_str(), "baseq3");
        assert_eq!(Q3SupplyProduct::Missionpack.as_str(), "missionpack");
        assert_eq!(Q3SupplyProduct::default(), Q3SupplyProduct::BaseQ3);
    }
}
