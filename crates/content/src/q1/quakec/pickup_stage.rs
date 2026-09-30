//! Qualified pickup stages (`src/content/q1/quakec/pickup-stage.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/pickup-stage.ts`
//! (`qcPickupStages`).

use crate::contract::{ItemId, PickupResource};

use super::qc_view::{QcInlineRegion, QcOpcode, QcProgramView};
use super::QcError;

/// Scalar source (donor `QcPickupScalar`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QcPickupScalar {
    /// Entity field.
    Field {
        /// Field name.
        name: String,
    },
    /// Global word.
    Global {
        /// Global word.
        word: u32,
    },
}

/// Pickup item discriminator (donor `QcPickupDescriptor.value`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcPickupValue {
    /// String discriminator.
    Str(String),
    /// Float discriminator.
    Num(f64),
}

/// Pickup descriptor (donor `QcPickupDescriptor`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupDescriptor {
    /// Discriminator value.
    pub value: QcPickupValue,
    /// Pickup item.
    pub item: ItemId,
    /// Resource binding.
    pub resource: Option<PickupResource>,
    /// Count source.
    pub count: Option<QcPickupScalar>,
    /// Supply offer.
    pub supply: Option<QcPickupSupply>,
}

/// Pickup supply (donor `QcPickupDescriptor.supply`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupSupply {
    /// Supply item.
    pub item: ItemId,
    /// Quantity source.
    pub quantity: QcPickupScalar,
    /// Leave word.
    pub leave: Option<u32>,
}

/// Pickup region operation (donor `QcPickupRegion.operation`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcPickupOperation {
    /// Recipient decision.
    Decision {
        /// Predicate word.
        word: u32,
        /// Accepted value.
        accepted: f64,
    },
    /// Resource grant.
    Grant,
    /// Weapon selection.
    WeaponSelection,
    /// Recipient admission.
    Admission,
    /// Pickup consumption.
    Consume,
    /// Post-consumption selection.
    ConsumedSelection,
    /// Source effect.
    SourceEffect,
    /// Cargo ownership predicate.
    CargoOwnership {
        /// Predicate word.
        word: u32,
    },
    /// Cargo current-weapon predicate.
    CargoCurrent {
        /// Predicate word.
        word: u32,
    },
    /// Counter projection.
    Counter {
        /// Counter field.
        field: String,
        /// Counter item.
        item: ItemId,
    },
}

/// Pickup region (donor `QcPickupRegion`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupRegion {
    /// Inline region.
    pub region: QcInlineRegion,
    /// Whether `self` is the recipient in the region.
    pub recipient_self: bool,
    /// Region operation.
    pub operation: QcPickupOperation,
}

/// Cargo counter (donor cargo `counters` entry).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QcPickupCounter {
    /// Counter field.
    pub field: String,
    /// Counter item.
    pub item: ItemId,
}

/// Weapon word (donor `weapons` entry).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QcWeaponWord {
    /// Global word.
    pub word: u32,
    /// Weapon item.
    pub item: ItemId,
}

/// Pickup stage descriptor (donor `QcPickupStage.descriptor`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcPickupStageDescriptor {
    /// Constant descriptor.
    Constant {
        /// Descriptor value.
        value: QcPickupDescriptor,
    },
    /// String-dispatched descriptor.
    Str {
        /// Dispatch field.
        field: String,
        /// Descriptors per value.
        values: Vec<QcPickupDescriptor>,
    },
    /// Float-dispatched descriptor.
    Float {
        /// Dispatch field.
        field: String,
        /// Descriptors per value.
        values: Vec<QcPickupDescriptor>,
    },
    /// Backpack cargo descriptor.
    Cargo {
        /// Descriptor value.
        value: QcPickupDescriptor,
        /// Cargo counters.
        counters: Vec<QcPickupCounter>,
        /// Carried weapons.
        weapons: Vec<QcWeaponWord>,
    },
}

/// Source selection continuation (donor `QcPickupStage.sourceSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcSourceSelection {
    /// Selection function.
    pub function_index: usize,
    /// Selection call statements.
    pub calls: Vec<usize>,
    /// Selectable weapons.
    pub weapons: Vec<QcWeaponWord>,
}

/// Source effect gate (donor `QcPickupStage.sourceEffect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QcSourceEffect {
    /// Gate word.
    pub word: u32,
    /// Gate value.
    pub value: f64,
}

/// Qualified pickup stage (donor `QcPickupStage`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupStage {
    /// Touch function index.
    pub function_index: usize,
    /// Dropped-pickup source.
    pub dropped: Option<QcPickupScalar>,
    /// Item descriptor.
    pub descriptor: QcPickupStageDescriptor,
    /// Qualified regions.
    pub regions: Vec<QcPickupRegion>,
    /// Source selection continuation.
    pub source_selection: Option<QcSourceSelection>,
    /// Source effect gate.
    pub source_effect: Option<QcSourceEffect>,
}

/// Recipient regions from the original id1 items.qc. Map feedback and
/// targets remain outside them (donor `qcPickupStages`).
pub fn qc_pickup_stages(program: &QcProgramView) -> Result<Vec<QcPickupStage>, QcError> {
    if program.digest == "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830" {
        return quakeworld_pickup_stages(program);
    }
    if program.digest != "sha256:f2619787f9aa0f057246eea1665b622b4691b5c5a800b1a46133d1fe8b771580" {
        return Ok(Vec::new());
    }
    let statement = |index: usize, opcode: QcOpcode, a: u16, b: u16, c: u16| -> Result<(), QcError> {
        let actual = program.statements.get(index);
        if actual.is_none_or(|actual| actual.opcode != opcode || actual.a != a || actual.b != b || actual.c != c) {
            return Err(QcError::program(
                format!("Original pickup statement {index} differs from its qualified artifact"),
                program.source,
            ));
        }
        Ok(())
    };
    let function = |name: &str, index: usize, first: i32, start: usize, words: usize| -> Result<usize, QcError> {
        let original = program.function_named(name)?;
        if original.index != index
            || original.first_statement != first
            || original.parameter_start != start
            || original.local_words != words
            || !original.parameter_sizes.is_empty()
            || original.named_builtin
        {
            return Err(QcError::program(
                format!("Unsupported original pickup caller {name}"),
                program.source,
            ));
        }
        Ok(index)
    };
    let decision = |function_index: usize, entry: usize, exit: usize, word: u16| -> Result<QcPickupRegion, QcError> {
        statement(exit, QcOpcode::IfNot, word, 2, 0)?;
        statement(exit + 1, QcOpcode::Return, 0, 0, 0)?;
        Ok(QcPickupRegion {
            region: QcInlineRegion {
                function_index,
                entry,
                exit,
                replaceable: true,
                standalone: None,
            },
            recipient_self: false,
            operation: QcPickupOperation::Decision {
                word: u32::from(word),
                accepted: 0.0,
            },
        })
    };
    let grant = |function_index: usize, entry: usize, exit: usize| QcPickupRegion {
        region: QcInlineRegion {
            function_index,
            entry,
            exit,
            replaceable: true,
            standalone: None,
        },
        recipient_self: false,
        operation: QcPickupOperation::Grant,
    };
    let armor = function("armor_touch", 128, 1949, 1931, 3)?;
    let ammo = function("ammo_touch", 142, 2389, 2145, 2)?;
    statement(1975, QcOpcode::LoadF, 29, 187, 1951)?;
    statement(1979, QcOpcode::Ge, 1953, 1954, 1955)?;
    statement(1982, QcOpcode::Address, 29, 187, 1956)?;
    statement(1994, QcOpcode::StorePF, 1965, 1958, 0)?;
    statement(1995, QcOpcode::Address, 28, 104, 1966)?;
    let armor_regions = vec![decision(armor, 1975, 1980, 1955)?, grant(armor, 1982, 1995)];
    let mut ammo_regions = Vec::new();
    for (entry, field, temporary) in [
        (2405, 158, 2153),
        (2417, 159, 2161),
        (2429, 160, 2169),
        (2441, 161, 2177),
    ] {
        statement(entry, QcOpcode::LoadF, 29, field, temporary)?;
        statement(entry + 4, QcOpcode::Address, 29, field, temporary + 2)?;
        statement(entry + 8, QcOpcode::StorePF, temporary + 5, temporary + 2, 0)?;
        ammo_regions.push(decision(ammo, entry, entry + 2, temporary + 1)?);
        ammo_regions.push(grant(ammo, entry + 4, entry + 9));
    }
    statement(2450, QcOpcode::Call0, 1990, 0, 0)?;
    statement(2451, QcOpcode::StoreV, 29, 4, 0)?;
    // The original clamp touches every ammo field. The selected owner already completed its grant.
    ammo_regions.push(grant(ammo, 2450, 2451));
    statement(2470, QcOpcode::LoadF, 29, 154, 2185)?;
    statement(2483, QcOpcode::StoreEnt, 2145, 28, 0)?;
    statement(2484, QcOpcode::Address, 28, 130, 2188)?;
    ammo_regions.push(QcPickupRegion {
        region: QcInlineRegion {
            function_index: ammo,
            entry: 2470,
            exit: 2484,
            replaceable: true,
            standalone: None,
        },
        recipient_self: false,
        operation: QcPickupOperation::WeaponSelection,
    });
    let weapon = function("weapon_touch", 135, 2119, 2021, 6)?;
    let mut weapon_regions = Vec::new();
    let mut weapon_descriptors = Vec::new();
    for (value, item, counter, entry, temporary, field, amount) in [
        (
            "weapon_nailgun",
            "q1:weapon/nailgun",
            "q1:ammo/nails",
            2138,
            2035,
            159,
            304,
        ),
        (
            "weapon_supernailgun",
            "q1:weapon/supernailgun",
            "q1:ammo/nails",
            2154,
            2045,
            159,
            304,
        ),
        (
            "weapon_supershotgun",
            "q1:weapon/supershotgun",
            "q1:ammo/shells",
            2170,
            2055,
            158,
            230,
        ),
        (
            "weapon_rocketlauncher",
            "q1:weapon/rocketlauncher",
            "q1:ammo/rockets",
            2186,
            2065,
            160,
            230,
        ),
        (
            "weapon_grenadelauncher",
            "q1:weapon/grenadelauncher",
            "q1:ammo/rockets",
            2202,
            2075,
            160,
            230,
        ),
        (
            "weapon_lightning",
            "q1:weapon/lightning",
            "q1:ammo/cells",
            2218,
            2085,
            161,
            1861,
        ),
    ] {
        statement(entry, QcOpcode::LoadF, 29, 162, temporary)?;
        statement(entry + 2, QcOpcode::And, 2026, temporary + 1, temporary + 2)?;
        statement(entry + 8, QcOpcode::Address, 29, field, temporary + 4)?;
        statement(entry + 10, QcOpcode::AddF, temporary + 5, amount, temporary + 6)?;
        statement(entry + 11, QcOpcode::StorePF, temporary + 6, temporary + 4, 0)?;
        weapon_regions.push(decision(weapon, entry, entry + 3, temporary + 2)?);
        weapon_regions.push(grant(weapon, entry + 8, entry + 12));
        weapon_descriptors.push(QcPickupDescriptor {
            value: QcPickupValue::Str(value.to_string()),
            item: item.to_string(),
            resource: Some(PickupResource::Inventory { item: item.to_string() }),
            count: None,
            supply: Some(QcPickupSupply {
                item: counter.to_string(),
                quantity: QcPickupScalar::Global { word: amount as u32 },
                leave: Some(2026),
            }),
        });
    }
    statement(2252, QcOpcode::Call0, 1990, 0, 0)?;
    statement(2270, QcOpcode::StoreEnt, 2025, 28, 0)?;
    statement(2271, QcOpcode::IfNot, 2026, 2, 0)?;
    weapon_regions.push(grant(weapon, 2252, 2271));
    let counters = ["q1:ammo/shells", "q1:ammo/nails", "q1:ammo/rockets", "q1:ammo/cells"];
    let backpack = function("BackpackTouch", 159, 3127, 2464, 6)?;
    statement(3135, QcOpcode::StoreF, 213, 2469, 0)?;
    statement(3136, QcOpcode::StoreV, 29, 4, 0)?;
    statement(3141, QcOpcode::LoadF, 29, 162, 2476)?;
    statement(3145, QcOpcode::IfNot, 2479, 9, 0)?;
    statement(3159, QcOpcode::Address, 29, 158, 2482)?;
    statement(3191, QcOpcode::Call0, 1990, 0, 0)?;
    statement(3192, QcOpcode::LoadF, 28, 158, 2505)?;
    statement(3272, QcOpcode::StoreV, 28, 4, 0)?;
    statement(3273, QcOpcode::Call1, 460, 0, 0)?;
    statement(3274, QcOpcode::StoreEnt, 29, 28, 0)?;
    statement(3275, QcOpcode::NotF, 35, 0, 2518)?;
    statement(3283, QcOpcode::Call0, 1785, 0, 0)?;
    statement(3284, QcOpcode::Done, 0, 0, 0)?;
    let backpack_region = |entry: usize, exit: usize, operation: QcPickupOperation| QcPickupRegion {
        region: QcInlineRegion {
            function_index: backpack,
            entry,
            exit,
            replaceable: true,
            standalone: None,
        },
        recipient_self: false,
        operation,
    };
    let backpack_weapons = [
        "q1:weapon/axe",
        "q1:weapon/shotgun",
        "q1:weapon/supershotgun",
        "q1:weapon/nailgun",
        "q1:weapon/supernailgun",
        "q1:weapon/grenadelauncher",
        "q1:weapon/rocketlauncher",
        "q1:weapon/lightning",
    ];
    Ok(vec![
        QcPickupStage {
            function_index: armor,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Str {
                field: "classname".to_string(),
                values: [
                    ("item_armor1", "q1:item_armor1"),
                    ("item_armor2", "q1:item_armor2"),
                    ("item_armorInv", "q1:item_armorInv"),
                ]
                .into_iter()
                .map(|(value, item)| QcPickupDescriptor {
                    value: QcPickupValue::Str(value.to_string()),
                    item: item.to_string(),
                    resource: Some(PickupResource::Protection {
                        channel: crate::contract::ProtectionChannel::Regular,
                    }),
                    count: None,
                    supply: None,
                })
                .collect(),
            },
            regions: armor_regions,
            source_selection: None,
            source_effect: None,
        },
        QcPickupStage {
            function_index: ammo,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Float {
                field: "weapon".to_string(),
                values: counters
                    .into_iter()
                    .zip([1.0, 2.0, 3.0, 4.0])
                    .map(|(item, discriminator)| QcPickupDescriptor {
                        value: QcPickupValue::Num(discriminator),
                        item: item.to_string(),
                        resource: Some(PickupResource::Inventory { item: item.to_string() }),
                        count: None,
                        supply: Some(QcPickupSupply {
                            item: item.to_string(),
                            quantity: QcPickupScalar::Field {
                                name: "aflag".to_string(),
                            },
                            leave: None,
                        }),
                    })
                    .collect(),
            },
            regions: ammo_regions,
            source_selection: None,
            source_effect: None,
        },
        QcPickupStage {
            function_index: weapon,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Str {
                field: "classname".to_string(),
                values: weapon_descriptors,
            },
            regions: weapon_regions,
            source_selection: None,
            source_effect: None,
        },
        QcPickupStage {
            function_index: backpack,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Cargo {
                value: QcPickupDescriptor {
                    value: QcPickupValue::Str("backpack".to_string()),
                    item: "q1:item_backpack".to_string(),
                    resource: None,
                    count: None,
                    supply: None,
                },
                counters: [
                    ("q1:ammo/shells", "ammo_shells"),
                    ("q1:ammo/nails", "ammo_nails"),
                    ("q1:ammo/rockets", "ammo_rockets"),
                    ("q1:ammo/cells", "ammo_cells"),
                ]
                .into_iter()
                .map(|(item, field)| QcPickupCounter {
                    item: item.to_string(),
                    field: field.to_string(),
                })
                .collect(),
                weapons: backpack_weapons
                    .into_iter()
                    .zip(253u32..)
                    .map(|(item, word)| QcWeaponWord {
                        word,
                        item: item.to_string(),
                    })
                    .collect(),
            },
            regions: vec![
                backpack_region(3135, 3136, QcPickupOperation::Admission),
                backpack_region(3141, 3145, QcPickupOperation::CargoOwnership { word: 2479 }),
                grant(backpack, 3159, 3192),
                backpack_region(3272, 3275, QcPickupOperation::Consume),
                backpack_region(3275, 3284, QcPickupOperation::ConsumedSelection),
            ],
            source_selection: None,
            source_effect: None,
        },
    ])
}

/// QuakeWorld pickup stages (donor `quakeWorldPickupStages`).
fn quakeworld_pickup_stages(program: &QcProgramView) -> Result<Vec<QcPickupStage>, QcError> {
    let statement = |index: usize, opcode: QcOpcode, a: u16, b: u16, c: u16| -> Result<(), QcError> {
        let actual = program.statements.get(index);
        if actual.is_none_or(|actual| actual.opcode != opcode || actual.a != a || actual.b != b || actual.c != c) {
            return Err(QcError::program(
                format!("QW pickup statement {index} differs from its qualified artifact"),
                program.source,
            ));
        }
        Ok(())
    };
    let function = |name: &str, index: usize, first: i32, start: usize, locals: usize| -> Result<usize, QcError> {
        let value = program.function_named(name)?;
        if value.index != index
            || value.first_statement != first
            || value.parameter_start != start
            || value.local_words != locals
            || value.named_builtin
        {
            return Err(QcError::program(
                format!("Unsupported QW pickup caller {name}"),
                program.source,
            ));
        }
        Ok(index)
    };
    let armor = function("armor_touch", 98, 1115, 1350, 3)?;
    let ammo = function("ammo_touch", 113, 1627, 1592, 2)?;
    let weapon = function("weapon_touch", 106, 1312, 1451, 7)?;
    let backpack = function("BackpackTouch", 130, 2412, 1929, 7)?;
    let selection = function("Deathmatch_Weapon", 105, 1300, 1442, 4)?;
    function("WeaponCode", 104, 1280, 1434, 0)?;
    let region = |function_index: usize, entry: usize, exit: usize, operation: QcPickupOperation| -> QcPickupRegion {
        QcPickupRegion {
            region: QcInlineRegion {
                function_index,
                entry,
                exit,
                replaceable: true,
                standalone: if matches!(operation, QcPickupOperation::Counter { .. }) {
                    Some(super::qc_view::InlineStandalone {
                        saved: 1932,
                        scope: super::qc_view::StandaloneScope::Frame,
                    })
                } else {
                    None
                },
            },
            recipient_self: false,
            operation,
        }
    };
    let decision = |function_index: usize, entry: usize, exit: usize, word: u16| -> Result<QcPickupRegion, QcError> {
        statement(exit, QcOpcode::IfNot, word, 2, 0)?;
        statement(exit + 1, QcOpcode::Return, 0, 0, 0)?;
        Ok(region(
            function_index,
            entry,
            exit,
            QcPickupOperation::Decision {
                word: u32::from(word),
                accepted: 0.0,
            },
        ))
    };
    let grant = |function_index: usize, entry: usize, exit: usize| {
        region(function_index, entry, exit, QcPickupOperation::Grant)
    };
    statement(1147, QcOpcode::LoadF, 29, 181, 1373)?;
    statement(1151, QcOpcode::Ge, 1375, 1376, 1377)?;
    statement(1154, QcOpcode::Address, 29, 181, 1378)?;
    statement(1166, QcOpcode::StorePF, 1387, 1380, 0)?;
    statement(1167, QcOpcode::Address, 28, 103, 1388)?;
    let counters = ["q1:ammo/shells", "q1:ammo/nails", "q1:ammo/rockets", "q1:ammo/cells"];
    let mut ammo_regions = Vec::new();
    for (entry, field, temporary) in [
        (1643, 153, 1600),
        (1655, 154, 1608),
        (1667, 155, 1616),
        (1679, 156, 1624),
    ] {
        statement(entry, QcOpcode::LoadF, 29, field, temporary)?;
        statement(entry + 4, QcOpcode::Address, 29, field, temporary + 2)?;
        statement(entry + 8, QcOpcode::StorePF, temporary + 5, temporary + 2, 0)?;
        ammo_regions.push(decision(ammo, entry, entry + 2, temporary + 1)?);
        ammo_regions.push(grant(ammo, entry + 4, entry + 9));
    }
    statement(1688, QcOpcode::Call0, 1412, 0, 0)?;
    statement(1711, QcOpcode::LoadF, 29, 149, 1632)?;
    statement(1724, QcOpcode::StoreEnt, 1592, 28, 0)?;
    statement(1725, QcOpcode::Address, 28, 125, 1635)?;
    ammo_regions.push(grant(ammo, 1688, 1689));
    ammo_regions.push(region(ammo, 1711, 1725, QcPickupOperation::WeaponSelection));
    let mut weapon_regions = Vec::new();
    let mut weapon_descriptors = Vec::new();
    for (value, item, counter, entry, temporary, field, amount) in [
        (
            "weapon_nailgun",
            "q1:weapon/nailgun",
            "q1:ammo/nails",
            1349,
            1471,
            154,
            298,
        ),
        (
            "weapon_supernailgun",
            "q1:weapon/supernailgun",
            "q1:ammo/nails",
            1365,
            1481,
            154,
            298,
        ),
        (
            "weapon_supershotgun",
            "q1:weapon/supershotgun",
            "q1:ammo/shells",
            1381,
            1491,
            153,
            224,
        ),
        (
            "weapon_rocketlauncher",
            "q1:weapon/rocketlauncher",
            "q1:ammo/rockets",
            1397,
            1501,
            155,
            224,
        ),
        (
            "weapon_grenadelauncher",
            "q1:weapon/grenadelauncher",
            "q1:ammo/rockets",
            1413,
            1511,
            155,
            224,
        ),
        (
            "weapon_lightning",
            "q1:weapon/lightning",
            "q1:ammo/cells",
            1429,
            1521,
            156,
            1276,
        ),
    ] {
        statement(entry, QcOpcode::LoadF, 29, 157, temporary)?;
        statement(entry + 2, QcOpcode::And, 1456, temporary + 1, temporary + 2)?;
        statement(entry + 8, QcOpcode::Address, 29, field, temporary + 4)?;
        statement(entry + 10, QcOpcode::AddF, temporary + 5, amount, temporary + 6)?;
        statement(entry + 11, QcOpcode::StorePF, temporary + 6, temporary + 4, 0)?;
        weapon_regions.push(decision(weapon, entry, entry + 3, temporary + 2)?);
        weapon_regions.push(grant(weapon, entry + 8, entry + 12));
        weapon_descriptors.push(QcPickupDescriptor {
            value: QcPickupValue::Str(value.to_string()),
            item: item.to_string(),
            resource: Some(PickupResource::Inventory { item: item.to_string() }),
            count: None,
            supply: Some(QcPickupSupply {
                item: counter.to_string(),
                quantity: QcPickupScalar::Global { word: amount as u32 },
                leave: Some(1456),
            }),
        });
    }
    statement(1466, QcOpcode::Call0, 1412, 0, 0)?;
    statement(1473, QcOpcode::StoreEnt, 28, 1455, 0)?;
    statement(1491, QcOpcode::Call0, 1084, 0, 0)?;
    statement(1492, QcOpcode::StoreEnt, 1455, 28, 0)?;
    weapon_regions.push(grant(weapon, 1466, 1473));
    let mut selection_region = region(weapon, 1491, 1492, QcPickupOperation::WeaponSelection);
    selection_region.recipient_self = true;
    weapon_regions.push(selection_region);
    let weapons = [
        (259, "q1:weapon/axe"),
        (248, "q1:weapon/shotgun"),
        (249, "q1:weapon/supershotgun"),
        (250, "q1:weapon/nailgun"),
        (251, "q1:weapon/supernailgun"),
        (252, "q1:weapon/grenadelauncher"),
        (253, "q1:weapon/rocketlauncher"),
        (254, "q1:weapon/lightning"),
    ]
    .into_iter()
    .map(|(word, item)| QcWeaponWord {
        word,
        item: item.to_string(),
    })
    .collect::<Vec<_>>();
    for call in [1486, 1490, 2704, 2708] {
        statement(call, QcOpcode::Call2, 1441, 0, 0)?;
    }
    statement(2478, QcOpcode::StoreV, 28, 4, 0)?;
    statement(2479, QcOpcode::Call1, 479, 0, 0)?;
    statement(2522, QcOpcode::StoreEnt, 29, 28, 0)?;
    statement(2446, QcOpcode::EqF, 364, 223, 1946)?;
    statement(2447, QcOpcode::IfNot, 1946, 77, 0)?;
    statement(2448, QcOpcode::Address, 29, 147, 1947)?;
    statement(2449, QcOpcode::LoadF, 29, 147, 1948)?;
    statement(2450, QcOpcode::AddF, 1948, 229, 1949)?;
    statement(2451, QcOpcode::StorePF, 1949, 1947, 0)?;
    statement(2504, QcOpcode::Address, 29, 156, 1973)?;
    statement(2505, QcOpcode::StorePF, 207, 1973, 0)?;
    statement(2524, QcOpcode::LoadF, 28, 157, 1977)?;
    statement(2525, QcOpcode::IfNot, 1977, 16, 0)?;
    statement(2526, QcOpcode::LoadF, 29, 157, 1978)?;
    statement(2529, QcOpcode::EqF, 1980, 207, 1981)?;
    statement(2546, QcOpcode::Address, 29, 153, 1984)?;
    statement(2565, QcOpcode::StorePF, 1999, 1996, 0)?;
    statement(2570, QcOpcode::LoadF, 29, 149, 2002)?;
    statement(2571, QcOpcode::StoreF, 2002, 1932, 0)?;
    statement(2572, QcOpcode::LoadF, 29, 157, 2003)?;
    statement(2579, QcOpcode::Call0, 1412, 0, 0)?;
    statement(2660, QcOpcode::EqF, 364, 222, 2021)?;
    statement(2661, QcOpcode::EqF, 364, 224, 2022)?;
    statement(2662, QcOpcode::Or, 2021, 2022, 2023)?;
    statement(2663, QcOpcode::StoreV, 1932, 4, 0)?;
    statement(2664, QcOpcode::Call1, 1433, 0, 0)?;
    statement(2665, QcOpcode::EqF, 1, 225, 2024)?;
    statement(2666, QcOpcode::StoreV, 1932, 4, 0)?;
    statement(2667, QcOpcode::Call1, 1433, 0, 0)?;
    statement(2668, QcOpcode::EqF, 1, 226, 2025)?;
    statement(2669, QcOpcode::Or, 2024, 2025, 2026)?;
    statement(2670, QcOpcode::BitAnd, 2023, 2026, 2027)?;
    statement(2671, QcOpcode::LoadF, 29, 155, 2028)?;
    statement(2672, QcOpcode::Lt, 2028, 224, 2029)?;
    statement(2673, QcOpcode::BitAnd, 2027, 2029, 2030)?;
    statement(2674, QcOpcode::IfNot, 2030, 3, 0)?;
    statement(2675, QcOpcode::Address, 29, 155, 2031)?;
    statement(2676, QcOpcode::StorePF, 224, 2031, 0)?;
    statement(2690, QcOpcode::StoreV, 28, 4, 0)?;
    statement(2691, QcOpcode::Call1, 479, 0, 0)?;
    statement(2692, QcOpcode::StoreEnt, 29, 28, 0)?;
    statement(2709, QcOpcode::Call0, 1084, 0, 0)?;
    Ok(vec![
        QcPickupStage {
            function_index: armor,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Str {
                field: "classname".to_string(),
                values: [
                    ("item_armor1", "q1:item_armor1"),
                    ("item_armor2", "q1:item_armor2"),
                    ("item_armorInv", "q1:item_armorInv"),
                ]
                .into_iter()
                .map(|(value, item)| QcPickupDescriptor {
                    value: QcPickupValue::Str(value.to_string()),
                    item: item.to_string(),
                    resource: Some(PickupResource::Protection {
                        channel: crate::contract::ProtectionChannel::Regular,
                    }),
                    count: None,
                    supply: None,
                })
                .collect(),
            },
            regions: vec![decision(armor, 1147, 1152, 1377)?, grant(armor, 1154, 1167)],
            source_selection: None,
            source_effect: None,
        },
        QcPickupStage {
            function_index: ammo,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Float {
                field: "weapon".to_string(),
                values: counters
                    .into_iter()
                    .zip([1.0, 2.0, 3.0, 4.0])
                    .map(|(item, discriminator)| QcPickupDescriptor {
                        value: QcPickupValue::Num(discriminator),
                        item: item.to_string(),
                        resource: Some(PickupResource::Inventory { item: item.to_string() }),
                        count: None,
                        supply: Some(QcPickupSupply {
                            item: item.to_string(),
                            quantity: QcPickupScalar::Field {
                                name: "aflag".to_string(),
                            },
                            leave: None,
                        }),
                    })
                    .collect(),
            },
            regions: ammo_regions,
            source_selection: None,
            source_effect: None,
        },
        QcPickupStage {
            function_index: weapon,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Str {
                field: "classname".to_string(),
                values: weapon_descriptors,
            },
            regions: weapon_regions,
            source_selection: Some(QcSourceSelection {
                function_index: selection,
                calls: vec![1486, 1490],
                weapons: weapons.clone(),
            }),
            source_effect: None,
        },
        QcPickupStage {
            function_index: backpack,
            dropped: None,
            descriptor: QcPickupStageDescriptor::Cargo {
                value: QcPickupDescriptor {
                    value: QcPickupValue::Str("backpack".to_string()),
                    item: "q1:item_backpack".to_string(),
                    resource: None,
                    count: None,
                    supply: None,
                },
                counters: [
                    ("q1:ammo/shells", "ammo_shells"),
                    ("q1:ammo/nails", "ammo_nails"),
                    ("q1:ammo/rockets", "ammo_rockets"),
                    ("q1:ammo/cells", "ammo_cells"),
                ]
                .into_iter()
                .map(|(item, field)| QcPickupCounter {
                    item: item.to_string(),
                    field: field.to_string(),
                })
                .collect(),
                weapons,
            },
            source_effect: Some(QcSourceEffect { word: 364, value: 4.0 }),
            source_selection: Some(QcSourceSelection {
                function_index: selection,
                calls: vec![2704, 2708],
                weapons: [
                    (259, "q1:weapon/axe"),
                    (248, "q1:weapon/shotgun"),
                    (249, "q1:weapon/supershotgun"),
                    (250, "q1:weapon/nailgun"),
                    (251, "q1:weapon/supernailgun"),
                    (252, "q1:weapon/grenadelauncher"),
                    (253, "q1:weapon/rocketlauncher"),
                    (254, "q1:weapon/lightning"),
                ]
                .into_iter()
                .map(|(word, item)| QcWeaponWord {
                    word,
                    item: item.to_string(),
                })
                .collect(),
            }),
            regions: vec![
                region(backpack, 2448, 2452, QcPickupOperation::SourceEffect),
                region(backpack, 2478, 2523, QcPickupOperation::Consume),
                region(
                    backpack,
                    2504,
                    2506,
                    QcPickupOperation::Counter {
                        item: "q1:ammo/cells".to_string(),
                        field: "ammo_cells".to_string(),
                    },
                ),
                region(backpack, 2524, 2525, QcPickupOperation::Admission),
                region(backpack, 2526, 2530, QcPickupOperation::CargoOwnership { word: 1981 }),
                grant(backpack, 2546, 2566),
                region(backpack, 2570, 2572, QcPickupOperation::CargoCurrent { word: 1932 }),
                grant(backpack, 2572, 2580),
                region(
                    backpack,
                    2660,
                    2677,
                    QcPickupOperation::Counter {
                        item: "q1:ammo/rockets".to_string(),
                        field: "ammo_rockets".to_string(),
                    },
                ),
                region(backpack, 2690, 2693, QcPickupOperation::Consume),
                region(backpack, 2709, 2710, QcPickupOperation::ConsumedSelection),
            ],
        },
    ])
}
