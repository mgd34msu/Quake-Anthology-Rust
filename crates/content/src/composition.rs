//! Cross-game pickup supply composition (`src/content/composition`).
//!
//! Donor provenance:
//! `src/content/composition/expansion-supply.ts`,
//! `src/content/composition/expansion-source-supply.ts`,
//! `src/content/composition/q2-q1-supply.ts`,
//! `src/content/composition/q2-q3-supply.ts`,
//! `src/content/composition/q3-q1-supply.ts`,
//! `src/content/composition/q3-q2-supply.ts`.
//!
//! The donor's `weaponOwnership: "all-destinations"` marker has no
//! counterpart in [`PickupSupplyProfile`], so it is dropped; owner lists
//! that are absent in the donor surface as empty vectors here.

use std::collections::{HashMap, HashSet};

use crate::contract::{ItemId, PickupOwner, PickupRoute, PickupSupplyProfile};

/// Expansion equipment set selectable in mixed-game supply
/// (`ExpansionSupply` in `expansion-supply.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpansionSupply {
    /// Quake rogue expansion equipment.
    Q1Rogue,
    /// Quake Machinegames episode 3 equipment.
    Q1Mg3,
    /// Quake II Xatrix equipment.
    Q2Xatrix,
    /// Quake II rogue equipment.
    Q2Rogue,
    /// Quake III mission pack equipment.
    Q3Missionpack,
}

impl ExpansionSupply {
    /// Donor selector text (`"q1-rogue"`, ...).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ExpansionSupply::Q1Rogue => "q1-rogue",
            ExpansionSupply::Q1Mg3 => "q1-mg3",
            ExpansionSupply::Q2Xatrix => "q2-xatrix",
            ExpansionSupply::Q2Rogue => "q2-rogue",
            ExpansionSupply::Q3Missionpack => "q3-missionpack",
        }
    }
}

/// Ammo destinations added per base item (`ammo` in `expansion-supply.ts`).
fn expansion_ammo(expansion: ExpansionSupply) -> HashMap<ItemId, Vec<ItemId>> {
    let entries: &[(&str, &[&str])] = match expansion {
        ExpansionSupply::Q1Rogue => &[
            ("q1:ammo/nails", &["rogue:ammo/lava-nails"]),
            ("q1:ammo/rockets", &["rogue:ammo/multi-rockets"]),
            ("q1:ammo/cells", &["rogue:ammo/plasma"]),
        ],
        ExpansionSupply::Q1Mg3 => &[],
        ExpansionSupply::Q2Xatrix => &[
            ("q2:ammo_grenades", &["q2:ammo_trap"]),
            ("q2:ammo_cells", &["q2:ammo_magslug"]),
            ("q2:ammo_slugs", &["q2:ammo_magslug"]),
        ],
        ExpansionSupply::Q2Rogue => &[
            ("q2:ammo_bullets", &["q2:ammo_flechettes"]),
            ("q2:ammo_grenades", &["q2:ammo_prox", "q2:ammo_tesla"]),
            ("q2:ammo_cells", &["q2:ammo_disruptor"]),
            ("q2:ammo_slugs", &["q2:ammo_disruptor"]),
        ],
        ExpansionSupply::Q3Missionpack => &[
            ("q3:ammo/machinegun", &["q3:ammo/nailgun", "q3:ammo/chaingun"]),
            ("q3:ammo/grenadelauncher", &["q3:ammo/proxlauncher"]),
        ],
    };
    entries
        .iter()
        .map(|(base, added)| ((*base).to_string(), added.iter().map(ToString::to_string).collect()))
        .collect()
}

/// Periodic ammo ownership granted per expansion
/// (`periodicAmmo` in `expansion-supply.ts`).
fn periodic_ammo(expansion: ExpansionSupply) -> Vec<PickupOwner> {
    let entries: &[(&str, &str)] = match expansion {
        ExpansionSupply::Q1Rogue => &[
            ("rogue:ammo/lava-nails", "q3:ammo/nailgun"),
            ("rogue:ammo/multi-rockets", "q3:ammo/proxlauncher"),
            ("rogue:ammo/plasma", "q3:ammo/plasmagun"),
        ],
        ExpansionSupply::Q1Mg3 => &[],
        ExpansionSupply::Q2Xatrix => &[
            ("q2:ammo_magslug", "q3:ammo/railgun"),
            ("q2:ammo_trap", "q3:ammo/proxlauncher"),
        ],
        ExpansionSupply::Q2Rogue => &[
            ("q2:ammo_flechettes", "q3:ammo/nailgun"),
            ("q2:ammo_prox", "q3:ammo/proxlauncher"),
            ("q2:ammo_tesla", "q3:ammo/proxlauncher"),
            ("q2:ammo_disruptor", "q3:ammo/bfg"),
        ],
        ExpansionSupply::Q3Missionpack => &[
            ("q3:ammo/nailgun", "q3:ammo/machinegun"),
            ("q3:ammo/chaingun", "q3:ammo/machinegun"),
            ("q3:ammo/proxlauncher", "q3:ammo/grenadelauncher"),
        ],
    };
    entries.iter().map(|(item, source)| owner(item, source)).collect()
}

/// Weapon destinations added per base item (`weapons` in `expansion-supply.ts`).
fn expansion_weapons(expansion: ExpansionSupply) -> HashMap<ItemId, Vec<ItemId>> {
    let entries: &[(&str, &[&str])] = match expansion {
        ExpansionSupply::Q1Rogue => &[
            ("q1:weapon/nailgun", &["q1:weapon/rogue:lava-nailgun"]),
            ("q1:weapon/supernailgun", &["q1:weapon/rogue:lava-supernailgun"]),
            ("q1:weapon/grenadelauncher", &["q1:weapon/rogue:multi-grenade"]),
            ("q1:weapon/rocketlauncher", &["q1:weapon/rogue:multi-rocket"]),
            ("q1:weapon/lightning", &["q1:weapon/rogue:plasma"]),
        ],
        ExpansionSupply::Q1Mg3 => &[("q1:weapon/lightning", &["q1:weapon/mg3:laser", "q1:weapon/mg3:mjolnir"])],
        ExpansionSupply::Q2Xatrix => &[
            ("q2:weapon_hyperblaster", &["q2:weapon_boomer", "q2:weapon_phalanx"]),
            ("q2:weapon_railgun", &["q2:weapon_phalanx"]),
        ],
        ExpansionSupply::Q2Rogue => &[
            ("q2:weapon_shotgun", &["q2:weapon_chainfist"]),
            ("q2:weapon_machinegun", &["q2:weapon_etf_rifle"]),
            ("q2:weapon_grenadelauncher", &["q2:weapon_proxlauncher"]),
            (
                "q2:weapon_hyperblaster",
                &["q2:weapon_plasmabeam", "q2:weapon_disintegrator"],
            ),
            ("q2:weapon_railgun", &["q2:weapon_disintegrator"]),
        ],
        ExpansionSupply::Q3Missionpack => &[
            ("q3:weapon/machinegun", &["q3:weapon/nailgun", "q3:weapon/chaingun"]),
            ("q3:weapon/grenadelauncher", &["q3:weapon/proxlauncher"]),
        ],
    };
    entries
        .iter()
        .map(|(base, added)| ((*base).to_string(), added.iter().map(ToString::to_string).collect()))
        .collect()
}

/// Build a supply route from string literals.
fn route(source: &str, destinations: &[&str]) -> PickupRoute {
    PickupRoute {
        source: source.to_string(),
        destinations: destinations.iter().map(ToString::to_string).collect(),
    }
}

/// Build an owner override from string literals.
fn owner(item: &str, source: &str) -> PickupOwner {
    PickupOwner {
        item: item.to_string(),
        source: source.to_string(),
    }
}

/// Append expansion destinations after each row's first destination,
/// preserving order and dropping repeats of the first (`extend` in
/// `expansion-supply.ts`). A row without destinations is kept as-is.
fn extend_routes(rows: &[PickupRoute], tables: &[HashMap<ItemId, Vec<ItemId>>]) -> Vec<PickupRoute> {
    rows.iter()
        .map(|row| {
            let Some((first, rest)) = row.destinations.split_first() else {
                return row.clone();
            };
            let mut merged: Vec<ItemId> = Vec::new();
            let additional = tables.iter().flat_map(|table| {
                row.destinations
                    .iter()
                    .flat_map(|item| table.get(item).map(Vec::as_slice).unwrap_or(&[]))
            });
            for item in rest.iter().chain(additional) {
                if item != first && !merged.contains(item) {
                    merged.push(item.clone());
                }
            }
            PickupRoute {
                source: row.source.clone(),
                destinations: std::iter::once(first.clone()).chain(merged).collect(),
            }
        })
        .collect()
}

/// Explicit mixed-game supply policy: retain each base grant and add the
/// selected expansions' related equipment (`expansionSupply`).
///
/// The donor only writes owner lists when the input defines them; this
/// port always carries both lists (possibly empty), so they are always
/// extended.
#[must_use]
pub fn expansion_supply(profile: &PickupSupplyProfile, expansions: &[ExpansionSupply]) -> PickupSupplyProfile {
    let ammo_tables: Vec<HashMap<ItemId, Vec<ItemId>>> =
        expansions.iter().map(|expansion| expansion_ammo(*expansion)).collect();
    let weapon_tables: Vec<HashMap<ItemId, Vec<ItemId>>> = expansions
        .iter()
        .map(|expansion| expansion_weapons(*expansion))
        .collect();
    let mut weapon_owners = profile.weapon_owners.clone();
    for table in &weapon_tables {
        for current in &profile.weapon_owners {
            if let Some(items) = table.get(&current.item) {
                for item in items {
                    weapon_owners.push(PickupOwner {
                        item: item.clone(),
                        source: current.source.clone(),
                    });
                }
            }
        }
    }
    let mut ammo_owners = profile.ammo_owners.clone();
    for expansion in expansions {
        ammo_owners.extend(periodic_ammo(*expansion));
    }
    let suffix = expansions
        .iter()
        .map(ExpansionSupply::as_str)
        .collect::<Vec<_>>()
        .join("+");
    PickupSupplyProfile {
        id: format!("{}/{}", profile.id, suffix),
        ammo: extend_routes(&profile.ammo, &ammo_tables),
        weapon_owners,
        ammo_owners,
        weapons: extend_routes(&profile.weapons, &weapon_tables),
    }
}

/// Expansion source item mapped onto its base supply group
/// (`ammoAliases` in `expansion-source-supply.ts`).
const AMMO_ALIASES: &[(&str, &str)] = &[
    ("rogue:ammo/lava-nails", "q1:ammo/nails"),
    ("rogue:ammo/multi-rockets", "q1:ammo/rockets"),
    ("rogue:ammo/plasma", "q1:ammo/cells"),
    ("q2:ammo_magslug", "q2:ammo_slugs"),
    ("q2:ammo_flechettes", "q2:ammo_bullets"),
    ("q2:ammo_disruptor", "q2:ammo_cells"),
    ("q2:ammo_trap", "q2:ammo_grenades"),
    ("q2:ammo_tesla", "q2:ammo_grenades"),
    ("q2:ammo_prox", "q2:ammo_grenades"),
    ("q3:ammo/nailgun", "q3:ammo/machinegun"),
    ("q3:ammo/chaingun", "q3:ammo/machinegun"),
    ("q3:ammo/proxlauncher", "q3:ammo/grenadelauncher"),
];

/// Expansion source item mapped onto its base supply group
/// (`weaponAliases` in `expansion-source-supply.ts`).
const WEAPON_ALIASES: &[(&str, &str)] = &[
    ("q1:weapon/hipnotic:laser", "q1:weapon/lightning"),
    ("q1:weapon/hipnotic:mjolnir", "q1:weapon/axe"),
    ("q1:weapon/hipnotic:proximity", "q1:weapon/grenadelauncher"),
    ("q1:weapon/mg3:laser", "q1:weapon/lightning"),
    ("q1:weapon/mg3:mjolnir", "q1:weapon/axe"),
    ("q1:weapon/rogue:lava-nailgun", "q1:weapon/nailgun"),
    ("q1:weapon/rogue:lava-supernailgun", "q1:weapon/supernailgun"),
    ("q1:weapon/rogue:multi-grenade", "q1:weapon/grenadelauncher"),
    ("q1:weapon/rogue:multi-rocket", "q1:weapon/rocketlauncher"),
    ("q1:weapon/rogue:plasma", "q1:weapon/lightning"),
    ("q2:weapon_boomer", "q2:weapon_hyperblaster"),
    ("q2:weapon_phalanx", "q2:weapon_railgun"),
    ("q2:weapon_plasmabeam", "q2:weapon_hyperblaster"),
    ("q2:weapon_etf_rifle", "q2:weapon_machinegun"),
    ("q2:weapon_proxlauncher", "q2:weapon_grenadelauncher"),
    ("q2:weapon_disintegrator", "q2:weapon_bfg"),
    ("q2:weapon_chainfist", "q2:weapon_shotgun"),
    ("q2:ammo_trap", "q2:ammo_grenades"),
    ("q2:ammo_tesla", "q2:ammo_grenades"),
    ("q3:weapon/nailgun", "q3:weapon/machinegun"),
    ("q3:weapon/chaingun", "q3:weapon/machinegun"),
    ("q3:weapon/proxlauncher", "q3:weapon/grenadelauncher"),
];

/// Copy each alias source onto its base row's destinations, skipping
/// aliases whose base row is missing or that already have a row
/// (`extend` in `expansion-source-supply.ts`).
fn extend_with_aliases(rows: &[PickupRoute], aliases: &[(&str, &str)]) -> Vec<PickupRoute> {
    let mut extended = rows.to_vec();
    for (source, base) in aliases {
        let mapping = rows.iter().find(|row| row.source == *base);
        let exists = rows.iter().any(|row| row.source == *source);
        if let Some(mapping) = mapping {
            if !exists {
                extended.push(PickupRoute {
                    source: (*source).to_string(),
                    destinations: mapping.destinations.clone(),
                });
            }
        }
    }
    extended
}

/// Expansion pickups reuse the selected composition's explicit base supply
/// groups, retaining original quantities (`expansionSourceSupply`).
#[must_use]
pub fn expansion_source_supply(profile: &PickupSupplyProfile) -> PickupSupplyProfile {
    let mut ammo_owners = profile.ammo_owners.clone();
    let mut seen: HashSet<&ItemId> = HashSet::new();
    for row in &profile.ammo {
        for item in &row.destinations {
            if !seen.insert(item) {
                continue;
            }
            if ammo_owners.iter().any(|owned| owned.item == *item) {
                continue;
            }
            let sources: Vec<&PickupRoute> = profile
                .ammo
                .iter()
                .filter(|row| row.destinations.contains(item))
                .collect();
            if let [single] = sources.as_slice() {
                ammo_owners.push(PickupOwner {
                    item: item.clone(),
                    source: single.source.clone(),
                });
            }
        }
    }
    PickupSupplyProfile {
        id: format!("{}/expansion-sources", profile.id),
        ammo: extend_with_aliases(&profile.ammo, AMMO_ALIASES),
        weapon_owners: profile.weapon_owners.clone(),
        ammo_owners,
        weapons: extend_with_aliases(&profile.weapons, WEAPON_ALIASES),
    }
}

/// Q2 quantities feed Q1 capacities. Slugs become nails; energy weapons
/// use lightning (`Q2_Q1_SUPPLY_PROFILE` in `q2-q1-supply.ts`).
#[must_use]
pub fn q2_q1_supply_profile() -> PickupSupplyProfile {
    PickupSupplyProfile {
        id: "composition:q2-base-q1-supply".to_string(),
        ammo: vec![
            route("q2:ammo_shells", &["q1:ammo/shells"]),
            route("q2:ammo_bullets", &["q1:ammo/nails"]),
            route("q2:ammo_cells", &["q2:ammo_cells", "q1:ammo/cells"]),
            route("q2:ammo_rockets", &["q1:ammo/rockets"]),
            route("q2:ammo_slugs", &["q1:ammo/nails"]),
            route("q2:ammo_grenades", &["q2:ammo_grenades", "q1:ammo/rockets"]),
        ],
        weapon_owners: vec![
            owner("q1:weapon/axe", "q2:weapon_blaster"),
            owner("q1:weapon/shotgun", "q2:weapon_shotgun"),
            owner("q1:weapon/supershotgun", "q2:weapon_supershotgun"),
            owner("q1:weapon/nailgun", "q2:weapon_machinegun"),
            owner("q1:weapon/supernailgun", "q2:weapon_chaingun"),
            owner("q1:weapon/grenadelauncher", "q2:weapon_grenadelauncher"),
            owner("q1:weapon/rocketlauncher", "q2:weapon_rocketlauncher"),
            owner("q1:weapon/lightning", "q2:weapon_hyperblaster"),
        ],
        ammo_owners: Vec::new(),
        weapons: vec![
            route("q2:weapon_blaster", &["q1:weapon/axe"]),
            route("q2:weapon_shotgun", &["q1:weapon/shotgun"]),
            route("q2:weapon_supershotgun", &["q1:weapon/supershotgun"]),
            route("q2:weapon_machinegun", &["q1:weapon/nailgun"]),
            route("q2:weapon_chaingun", &["q1:weapon/supernailgun"]),
            route("q2:ammo_grenades", &["q1:weapon/grenadelauncher"]),
            route("q2:weapon_grenadelauncher", &["q1:weapon/grenadelauncher"]),
            route("q2:weapon_rocketlauncher", &["q1:weapon/rocketlauncher"]),
            route("q2:weapon_hyperblaster", &["q1:weapon/lightning"]),
            route("q2:weapon_railgun", &["q1:weapon/supernailgun"]),
            route("q2:weapon_bfg", &["q1:weapon/lightning"]),
        ],
    }
}

/// Hipnotic weapon grants layered onto the Q2-to-Q1 profile
/// (`Q2_HIPNOTIC_SUPPLY_PROFILE` in `q2-q1-supply.ts`).
#[must_use]
pub fn q2_hipnotic_supply_profile() -> PickupSupplyProfile {
    const HIPNOTIC: &[(&str, &[&str])] = &[
        ("q2:weapon_hyperblaster", &["q1:weapon/hipnotic:laser"]),
        ("q2:weapon_grenadelauncher", &["q1:weapon/hipnotic:proximity"]),
        ("q2:weapon_bfg", &["q1:weapon/hipnotic:mjolnir"]),
    ];
    let base = q2_q1_supply_profile();
    let mut weapon_owners = base.weapon_owners.clone();
    for (source, destinations) in HIPNOTIC {
        for item in *destinations {
            weapon_owners.push(owner(item, source));
        }
    }
    let weapons = base
        .weapons
        .iter()
        .map(|entry| {
            let mut destinations = entry.destinations.clone();
            if let Some((_, added)) = HIPNOTIC.iter().find(|(source, _)| *source == entry.source) {
                destinations.extend(added.iter().map(ToString::to_string));
            }
            PickupRoute {
                source: entry.source.clone(),
                destinations,
            }
        })
        .collect();
    PickupSupplyProfile {
        id: "composition:q2-hipnotic-supply".to_string(),
        ammo: base.ammo.clone(),
        weapon_owners,
        ammo_owners: base.ammo_owners.clone(),
        weapons,
    }
}

/// Base Q2 supply quantities feed the selected Q3 arsenal's capacities
/// (`Q2_Q3_SUPPLY_PROFILE` in `q2-q3-supply.ts`).
#[must_use]
pub fn q2_q3_supply_profile() -> PickupSupplyProfile {
    PickupSupplyProfile {
        id: "composition:q2-base-q3-supply".to_string(),
        ammo: vec![
            route("q2:ammo_shells", &["q3:ammo/shotgun"]),
            route("q2:ammo_bullets", &["q3:ammo/machinegun"]),
            route(
                "q2:ammo_cells",
                &["q2:ammo_cells", "q3:ammo/plasmagun", "q3:ammo/lightning", "q3:ammo/bfg"],
            ),
            route("q2:ammo_rockets", &["q3:ammo/rocketlauncher"]),
            route("q2:ammo_slugs", &["q3:ammo/railgun"]),
            route("q2:ammo_grenades", &["q2:ammo_grenades", "q3:ammo/grenadelauncher"]),
        ],
        weapon_owners: vec![
            owner("q3:weapon/shotgun", "q2:weapon_shotgun"),
            owner("q3:weapon/machinegun", "q2:weapon_machinegun"),
            owner("q3:weapon/grenadelauncher", "q2:weapon_grenadelauncher"),
            owner("q3:weapon/rocketlauncher", "q2:weapon_rocketlauncher"),
            owner("q3:weapon/plasmagun", "q2:weapon_hyperblaster"),
            owner("q3:weapon/lightning", "q2:weapon_hyperblaster"),
            owner("q3:weapon/railgun", "q2:weapon_railgun"),
            owner("q3:weapon/bfg", "q2:weapon_bfg"),
        ],
        ammo_owners: Vec::new(),
        weapons: vec![
            route("q2:weapon_shotgun", &["q3:weapon/shotgun"]),
            route("q2:weapon_supershotgun", &["q3:weapon/shotgun"]),
            route("q2:weapon_machinegun", &["q3:weapon/machinegun"]),
            route("q2:weapon_chaingun", &["q3:weapon/machinegun"]),
            route("q2:ammo_grenades", &["q3:weapon/grenadelauncher"]),
            route("q2:weapon_grenadelauncher", &["q3:weapon/grenadelauncher"]),
            route("q2:weapon_rocketlauncher", &["q3:weapon/rocketlauncher"]),
            route(
                "q2:weapon_hyperblaster",
                &["q3:weapon/plasmagun", "q3:weapon/lightning"],
            ),
            route("q2:weapon_railgun", &["q3:weapon/railgun"]),
            route("q2:weapon_bfg", &["q3:weapon/bfg"]),
        ],
    }
}

/// Q3 quantities feed Q1 capacities. Rail uses nails; plasma and BFG use
/// lightning (`Q3_Q1_SUPPLY_PROFILE` in `q3-q1-supply.ts`).
#[must_use]
pub fn q3_q1_supply_profile() -> PickupSupplyProfile {
    PickupSupplyProfile {
        id: "composition:q3-base-q1-supply".to_string(),
        ammo: vec![
            route("q3:ammo/shotgun", &["q1:ammo/shells"]),
            route("q3:ammo/machinegun", &["q1:ammo/nails"]),
            route("q3:ammo/grenadelauncher", &["q1:ammo/rockets"]),
            route("q3:ammo/rocketlauncher", &["q1:ammo/rockets"]),
            route("q3:ammo/lightning", &["q1:ammo/cells"]),
            route("q3:ammo/railgun", &["q1:ammo/nails"]),
            route("q3:ammo/plasmagun", &["q1:ammo/cells"]),
            route("q3:ammo/bfg", &["q1:ammo/cells"]),
        ],
        weapon_owners: Vec::new(),
        ammo_owners: vec![
            owner("q1:ammo/shells", "q3:ammo/shotgun"),
            owner("q1:ammo/nails", "q3:ammo/machinegun"),
            owner("q1:ammo/rockets", "q3:ammo/rocketlauncher"),
            owner("q1:ammo/cells", "q3:ammo/lightning"),
        ],
        weapons: vec![
            route("q3:weapon/gauntlet", &["q1:weapon/axe"]),
            route("q3:weapon/shotgun", &["q1:weapon/supershotgun"]),
            route("q3:weapon/machinegun", &["q1:weapon/nailgun"]),
            route("q3:weapon/grenadelauncher", &["q1:weapon/grenadelauncher"]),
            route("q3:weapon/rocketlauncher", &["q1:weapon/rocketlauncher"]),
            route("q3:weapon/lightning", &["q1:weapon/lightning"]),
            route("q3:weapon/railgun", &["q1:weapon/supernailgun"]),
            route("q3:weapon/plasmagun", &["q1:weapon/lightning"]),
            route("q3:weapon/bfg", &["q1:weapon/lightning"]),
        ],
    }
}

/// Hipnotic weapon grants layered onto the Q3-to-Q1 profile
/// (`Q3_HIPNOTIC_SUPPLY_PROFILE` in `q3-q1-supply.ts`).
#[must_use]
pub fn q3_hipnotic_supply_profile() -> PickupSupplyProfile {
    const HIPNOTIC: &[(&str, &[&str])] = &[
        ("q3:weapon/plasmagun", &["q1:weapon/hipnotic:laser"]),
        ("q3:weapon/grenadelauncher", &["q1:weapon/hipnotic:proximity"]),
        ("q3:weapon/bfg", &["q1:weapon/hipnotic:mjolnir"]),
    ];
    let base = q3_q1_supply_profile();
    let weapons = base
        .weapons
        .iter()
        .map(|entry| {
            let mut destinations = entry.destinations.clone();
            if let Some((_, added)) = HIPNOTIC.iter().find(|(source, _)| *source == entry.source) {
                destinations.extend(added.iter().map(ToString::to_string));
            }
            PickupRoute {
                source: entry.source.clone(),
                destinations,
            }
        })
        .collect();
    PickupSupplyProfile {
        id: "composition:q3-hipnotic-supply".to_string(),
        ammo: base.ammo.clone(),
        weapon_owners: base.weapon_owners.clone(),
        ammo_owners: base.ammo_owners.clone(),
        weapons,
    }
}

/// Base Q3 pickup quantities feed the selected base Q2 arsenal and its
/// shared equipment pools (`Q3_Q2_SUPPLY_PROFILE` in `q3-q2-supply.ts`).
#[must_use]
pub fn q3_q2_supply_profile() -> PickupSupplyProfile {
    PickupSupplyProfile {
        id: "composition:q3-base-q2-supply".to_string(),
        ammo: vec![
            route("q3:ammo/machinegun", &["q2:ammo_bullets"]),
            route("q3:ammo/shotgun", &["q2:ammo_shells"]),
            route("q3:ammo/grenadelauncher", &["q2:ammo_grenades"]),
            route("q3:ammo/rocketlauncher", &["q2:ammo_rockets"]),
            route("q3:ammo/lightning", &["q2:ammo_cells"]),
            route("q3:ammo/plasmagun", &["q2:ammo_cells"]),
            route("q3:ammo/bfg", &["q2:ammo_cells"]),
            route("q3:ammo/railgun", &["q2:ammo_slugs"]),
        ],
        weapon_owners: Vec::new(),
        ammo_owners: vec![
            owner("q2:ammo_bullets", "q3:ammo/machinegun"),
            owner("q2:ammo_shells", "q3:ammo/shotgun"),
            owner("q2:ammo_grenades", "q3:ammo/grenadelauncher"),
            owner("q2:ammo_rockets", "q3:ammo/rocketlauncher"),
            owner("q2:ammo_cells", "q3:ammo/lightning"),
            owner("q2:ammo_slugs", "q3:ammo/railgun"),
        ],
        weapons: vec![
            route("q3:weapon/gauntlet", &["q2:weapon_blaster"]),
            route("q3:weapon/machinegun", &["q2:weapon_machinegun", "q2:weapon_chaingun"]),
            route("q3:weapon/shotgun", &["q2:weapon_shotgun", "q2:weapon_supershotgun"]),
            route("q3:weapon/grenadelauncher", &["q2:weapon_grenadelauncher"]),
            route("q3:weapon/rocketlauncher", &["q2:weapon_rocketlauncher"]),
            route("q3:weapon/lightning", &["q2:weapon_hyperblaster"]),
            route("q3:weapon/plasmagun", &["q2:weapon_hyperblaster"]),
            route("q3:weapon/railgun", &["q2:weapon_railgun"]),
            route("q3:weapon/bfg", &["q2:weapon_bfg"]),
        ],
    }
}

/// One loadout inventory grant (`q3Q2SupplyLoadout` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyLoadoutEntry {
    /// Granted item.
    pub item: ItemId,
    /// Granted count.
    pub count: u32,
}

/// Starting weapon plus inventory for the Q3-to-Q2 composition
/// (`q3Q2SupplyLoadout` in `q3-q2-supply.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyLoadout {
    /// Selected starting weapon.
    pub weapon: ItemId,
    /// Granted inventory.
    pub inventory: Vec<SupplyLoadoutEntry>,
}

/// Starting weapon plus inventory for the Q3-to-Q2 composition
/// (`q3Q2SupplyLoadout`).
#[must_use]
pub fn q3_q2_supply_loadout() -> SupplyLoadout {
    SupplyLoadout {
        weapon: "q2:weapon_machinegun".to_string(),
        inventory: vec![
            SupplyLoadoutEntry {
                item: "q2:weapon_blaster".to_string(),
                count: 1,
            },
            SupplyLoadoutEntry {
                item: "q2:weapon_machinegun".to_string(),
                count: 1,
            },
            SupplyLoadoutEntry {
                item: "q2:weapon_chaingun".to_string(),
                count: 1,
            },
            SupplyLoadoutEntry {
                item: "q2:ammo_bullets".to_string(),
                count: 100,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_supply_adds_related_equipment() {
        let profile = q2_q3_supply_profile();
        let extended = expansion_supply(&profile, &[ExpansionSupply::Q3Missionpack]);
        assert_eq!(extended.id, "composition:q2-base-q3-supply/q3-missionpack");
        let bullets = extended
            .ammo
            .iter()
            .find(|row| row.source == "q2:ammo_bullets")
            .expect("bullets row");
        assert_eq!(
            bullets.destinations,
            vec![
                "q3:ammo/machinegun".to_string(),
                "q3:ammo/nailgun".to_string(),
                "q3:ammo/chaingun".to_string(),
            ]
        );
        let weapons = extended
            .weapons
            .iter()
            .find(|row| row.source == "q2:weapon_machinegun")
            .expect("machinegun weapon row");
        assert_eq!(
            weapons.destinations,
            vec![
                "q3:weapon/machinegun".to_string(),
                "q3:weapon/nailgun".to_string(),
                "q3:weapon/chaingun".to_string(),
            ]
        );
        assert!(extended
            .weapon_owners
            .iter()
            .any(|owned| owned.item == "q3:weapon/nailgun" && owned.source == "q2:weapon_machinegun"));
        assert!(extended
            .weapon_owners
            .iter()
            .any(|owned| owned.item == "q3:weapon/proxlauncher" && owned.source == "q2:weapon_grenadelauncher"));
        assert!(extended
            .ammo_owners
            .iter()
            .any(|owned| owned.item == "q3:ammo/nailgun" && owned.source == "q3:ammo/machinegun"));
        let empty = expansion_supply(&profile, &[ExpansionSupply::Q1Mg3]);
        assert_eq!(empty.ammo, profile.ammo);
        assert_eq!(empty.weapons, profile.weapons);
    }

    #[test]
    fn expansion_supply_keeps_first_destination_first() {
        let profile = q3_q2_supply_profile();
        let extended = expansion_supply(&profile, &[ExpansionSupply::Q2Xatrix, ExpansionSupply::Q2Rogue]);
        assert_eq!(extended.id, "composition:q3-base-q2-supply/q2-xatrix+q2-rogue");
        let lightning = extended
            .ammo
            .iter()
            .find(|row| row.source == "q3:ammo/lightning")
            .expect("lightning row");
        assert_eq!(
            lightning.destinations,
            vec![
                "q2:ammo_cells".to_string(),
                "q2:ammo_magslug".to_string(),
                "q2:ammo_disruptor".to_string(),
            ]
        );
        let hyperblaster = extended
            .weapons
            .iter()
            .find(|row| row.source == "q3:weapon/lightning")
            .expect("hyperblaster row");
        assert_eq!(
            hyperblaster.destinations,
            vec![
                "q2:weapon_hyperblaster".to_string(),
                "q2:weapon_boomer".to_string(),
                "q2:weapon_phalanx".to_string(),
                "q2:weapon_plasmabeam".to_string(),
                "q2:weapon_disintegrator".to_string(),
            ]
        );
        assert!(extended
            .ammo_owners
            .iter()
            .any(|owned| owned.item == "q2:ammo_trap" && owned.source == "q3:ammo/proxlauncher"));
        assert!(extended
            .ammo_owners
            .iter()
            .any(|owned| owned.item == "q2:ammo_disruptor" && owned.source == "q3:ammo/bfg"));
    }

    #[test]
    fn expansion_source_supply_reuses_base_groups() {
        let profile = q2_q1_supply_profile();
        let extended = expansion_source_supply(&profile);
        assert_eq!(extended.id, "composition:q2-base-q1-supply/expansion-sources");
        let trap = extended
            .ammo
            .iter()
            .find(|row| row.source == "q2:ammo_trap")
            .expect("trap row");
        assert_eq!(
            trap.destinations,
            vec!["q2:ammo_grenades".to_string(), "q1:ammo/rockets".to_string()]
        );
        let boomer = extended
            .weapons
            .iter()
            .find(|row| row.source == "q2:weapon_boomer")
            .expect("boomer row");
        assert_eq!(boomer.destinations, vec!["q1:weapon/lightning".to_string()]);
        assert!(
            extended
                .ammo_owners
                .iter()
                .any(|owned| owned.item == "q1:ammo/shells" && owned.source == "q2:ammo_shells"),
            "single-source destination gains an owner"
        );
        assert!(
            !extended.ammo_owners.iter().any(|owned| owned.item == "q1:ammo/nails"),
            "multi-source destination gains no owner"
        );
    }

    #[test]
    fn hipnotic_profiles_layer_extra_weapons() {
        let q2 = q2_hipnotic_supply_profile();
        assert_eq!(q2.id, "composition:q2-hipnotic-supply");
        let hyperblaster = q2
            .weapons
            .iter()
            .find(|row| row.source == "q2:weapon_hyperblaster")
            .expect("hyperblaster row");
        assert!(hyperblaster
            .destinations
            .contains(&"q1:weapon/hipnotic:laser".to_string()));
        assert!(q2
            .weapon_owners
            .iter()
            .any(|owned| owned.item == "q1:weapon/hipnotic:mjolnir" && owned.source == "q2:weapon_bfg"));
        let q3 = q3_hipnotic_supply_profile();
        assert_eq!(q3.id, "composition:q3-hipnotic-supply");
        let plasma = q3
            .weapons
            .iter()
            .find(|row| row.source == "q3:weapon/plasmagun")
            .expect("plasma row");
        assert!(plasma.destinations.contains(&"q1:weapon/hipnotic:laser".to_string()));
        assert!(q3.weapon_owners.is_empty());
    }

    #[test]
    fn base_profiles_carry_expected_routes() {
        let q2_q1 = q2_q1_supply_profile();
        assert_eq!(q2_q1.ammo.len(), 6);
        assert_eq!(q2_q1.weapons.len(), 11);
        assert_eq!(q2_q1.weapon_owners.len(), 8);
        let q2_q3 = q2_q3_supply_profile();
        assert_eq!(q2_q3.ammo.len(), 6);
        assert_eq!(q2_q3.weapons.len(), 10);
        let q3_q1 = q3_q1_supply_profile();
        assert_eq!(q3_q1.ammo.len(), 8);
        assert_eq!(q3_q1.ammo_owners.len(), 4);
        let q3_q2 = q3_q2_supply_profile();
        assert_eq!(q3_q2.ammo.len(), 8);
        assert_eq!(q3_q2.ammo_owners.len(), 6);
        let loadout = q3_q2_supply_loadout();
        assert_eq!(loadout.weapon, "q2:weapon_machinegun");
        assert_eq!(loadout.inventory.len(), 4);
        assert_eq!(loadout.inventory[3].count, 100);
    }
}
