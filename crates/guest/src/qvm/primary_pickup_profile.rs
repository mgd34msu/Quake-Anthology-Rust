//! Primary pickup profile: original eligibility gates and grant regions.
//!
//! Provenance: `src/compat/qvm/primary-pickup-profile.ts`.
//!
//! Local mirrors: [`QvmPickupProfile`]/[`QvmPickupGrant`] (`game-pickups.ts`),
//! [`QvmItemLayout`]/[`parse_qvm_item_layout`] (`item-catalog.ts`; record
//! scanning stays with the item-catalog owner). Regions and images reuse
//! [`super::mod_provider`]. The donor's `eligible` interpreter closure is a
//! declarative [`EligibilityOverride`] list plus [`QvmPickupGrant::decide`].

use std::cell::RefCell;
use std::collections::HashSet;

use super::mod_provider::{
    ProfileReader, ProfileValue, QvmAbi, QvmArtifact, QvmOpcode, QvmRegionEvaluation, QVM_MAX_PRIVATE_ARGUMENT_WORDS, qualify_qvm_region, qualify_qvm_region_evaluation,
    qvm_player_state_bytes, qvm_shared_entity_bytes,
};
use crate::error::GuestError;

// ---------------------------------------------------------------------------
// Item layout mirror (`item-catalog.ts`).
// ---------------------------------------------------------------------------

/// Item-table address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableAddress {
    /// Direct address.
    Direct(usize),
    /// Global locator.
    Global {
        /// Global address.
        global: usize,
    },
}

/// Item-table count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableCount {
    /// Direct count.
    Direct(usize),
    /// Global locator.
    Global {
        /// Global address.
        global: usize,
        /// Maximum count.
        maximum: usize,
    },
}

/// Item record fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemTableFields {
    /// Class-name offset.
    pub class_name: usize,
    /// Pickup-name offset.
    pub pickup_name: usize,
    /// Type offset.
    pub type_: usize,
    /// Tag offset.
    pub tag: usize,
}

/// Item table layout (mirror of `QvmItemLayout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmItemLayout {
    /// Table address.
    pub address: TableAddress,
    /// Table count.
    pub count: TableCount,
    /// Live source storage flag.
    pub live: bool,
    /// Record stride.
    pub stride: usize,
    /// Record fields.
    pub fields: ItemTableFields,
    /// Weapon type.
    pub weapon_type: usize,
    /// Ammo type.
    pub ammo_type: usize,
}

/// Parse an item table layout.
pub fn parse_qvm_item_layout(reader: &ProfileReader<'_>) -> Result<QvmItemLayout, GuestError> {
    let fields = reader.field("fields")?;
    let stride = reader.field("stride")?.integer(4)? as usize;
    let offset = |name: &str| -> Result<usize, GuestError> {
        let value = fields.field(name)?.integer(0)? as usize;
        if value % 4 != 0 || value + 4 > stride {
            return fields.field(name)?.fail("item field exceeds its record or is unaligned");
        }
        Ok(value)
    };
    let pointer = reader.field("address")?;
    let count = reader.field("count")?;
    let address = match pointer.value() {
        ProfileValue::Int(_) | ProfileValue::Float(_) => TableAddress::Direct(pointer.integer(4)? as usize),
        _ => TableAddress::Global { global: pointer.field("global")?.integer(0)? as usize },
    };
    let count = match count.value() {
        ProfileValue::Int(_) | ProfileValue::Float(_) => TableCount::Direct(count.integer(1)? as usize),
        _ => TableCount::Global { global: count.field("global")?.integer(0)? as usize, maximum: count.field("maximum")?.integer(1)? as usize },
    };
    let live = !reader.field("source")?.is_undefined();
    if live {
        reader.field("source")?.literal_str("live")?;
    }
    let layout = QvmItemLayout {
        address,
        count,
        live,
        stride,
        fields: ItemTableFields {
            class_name: offset("className")?,
            pickup_name: offset("pickupName")?,
            type_: offset("type")?,
            tag: offset("tag")?,
        },
        weapon_type: reader.field("weaponType")?.integer(0)? as usize,
        ammo_type: reader.field("ammoType")?.integer(0)? as usize,
    };
    let base = match layout.address {
        TableAddress::Direct(address) => address,
        TableAddress::Global { global } => global,
    };
    let count_global = match layout.count {
        TableCount::Direct(_) => None,
        TableCount::Global { global, .. } => Some(global),
    };
    if base % 4 != 0 || count_global.is_some_and(|global| global % 4 != 0) || stride % 4 != 0 || layout.weapon_type == layout.ammo_type {
        return reader.fail("invalid item table layout");
    }
    if !live && (!matches!(layout.address, TableAddress::Direct(_)) || !matches!(layout.count, TableCount::Direct(_))) {
        return reader.fail("runtime item table locations require live source storage");
    }
    Ok(layout)
}

// ---------------------------------------------------------------------------
// Pickup profile mirror (`game-pickups.ts`).
// ---------------------------------------------------------------------------

/// Declared original call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCalls {
    /// Owning entry.
    pub entry: usize,
    /// Call instructions.
    pub calls: Vec<usize>,
}

/// Eligibility gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateProfile {
    /// Owning entry.
    pub entry: usize,
    /// Call instructions.
    pub calls: Vec<usize>,
    /// Item argument.
    pub item_argument: usize,
    /// Player argument.
    pub player_argument: usize,
}

/// Weapon grant location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponGrantLocation {
    /// Private inventory projection.
    Inventory,
    /// Public record offsets.
    Offsets {
        /// Bits offset.
        bits_offset: usize,
        /// Ammo offset.
        ammo_offset: usize,
    },
}

/// Weapon grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponGrant {
    /// Location.
    pub location: WeaponGrantLocation,
    /// Quantity evaluation.
    pub quantity: QvmRegionEvaluation,
}

/// Grant operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickupOperation {
    /// Accepted nonzero return.
    Return {
        /// Accepted return instruction.
        accepted_return: usize,
    },
    /// Quantity region.
    Region {
        /// Entry instruction.
        entry: usize,
        /// Join instruction.
        join: usize,
        /// Quantity local.
        quantity: usize,
        /// Weapon grant, if any.
        weapon: Option<WeaponGrant>,
    },
}

/// Eligibility override of one gate conditional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EligibilityOverride {
    /// Instruction index.
    pub instruction_index: usize,
    /// Forced direction.
    pub taken: bool,
}

/// Pickup grant (mirror of `QvmPickupGrant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPickupGrant {
    /// Owning entry.
    pub entry: usize,
    /// Call instructions.
    pub calls: Vec<usize>,
    /// Item type.
    pub item_type: i64,
    /// Operation.
    pub operation: PickupOperation,
    /// Eligibility overrides.
    pub eligibility: Vec<EligibilityOverride>,
}

impl QvmPickupGrant {
    /// Forced direction of one override.
    #[must_use]
    pub const fn decide(override_: &EligibilityOverride) -> bool {
        override_.taken
    }
}

/// Pickup fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PickupFields {
    /// In-use offset.
    pub inuse: usize,
    /// Client offset.
    pub client: usize,
    /// Health offset.
    pub health: usize,
    /// Item offset.
    pub item: usize,
    /// Count offset.
    pub count: usize,
    /// Flags offset.
    pub flags: usize,
}

/// Primary pickup profile (mirror of `QvmPickupProfile`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPickupProfile {
    /// Module identity.
    pub module: super::mod_provider::ModuleId,
    /// ABI profile.
    pub abi_profile: QvmAbi,
    /// Entity stride.
    pub entity_stride: usize,
    /// Client stride.
    pub client_stride: usize,
    /// Fields.
    pub fields: PickupFields,
    /// Dropped flag.
    pub dropped_flag: i64,
    /// Item layout.
    pub items: QvmItemLayout,
    /// Touch entry.
    pub touch: usize,
    /// Gate.
    pub gate: GateProfile,
    /// Targets.
    pub targets: FunctionCalls,
    /// Free entry.
    pub free: usize,
    /// Objective types.
    pub objective_types: Vec<i64>,
    /// Grants.
    pub grants: Vec<QvmPickupGrant>,
}

/// Read a primary pickup profile.
pub fn read_qvm_primary_pickup_profile(reader: &ProfileReader<'_>, artifact: &QvmArtifact) -> Result<QvmPickupProfile, GuestError> {
    use super::mod_provider::QvmRole;
    let abi_profile = artifact.abi();
    if artifact.role != QvmRole::Qagame {
        return reader.fail("primary pickup declarations require a qagame ABI");
    }
    let instructions = &artifact.image.instructions;
    let read_entry = |at: &ProfileReader<'_>| -> Result<usize, GuestError> {
        let value = at.integer(0)? as usize;
        if artifact.image.instruction(value).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
            return at.fail("pickup requires an original function entry");
        }
        Ok(value)
    };
    let owner_end = |owner: usize| -> usize {
        let mut end = owner + 1;
        while end < instructions.len() && instructions[end].opcode != QvmOpcode::OpEnter {
            end += 1;
        }
        end
    };
    let read_calls = |at: &ProfileReader<'_>, owner: usize| -> Result<Vec<usize>, GuestError> {
        at.list(|value| {
            let index = value.integer(1)? as usize;
            let target = index.checked_sub(1).and_then(|at| instructions.get(at));
            if instructions.get(index).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpCall)
                || target.is_none_or(|target| target.opcode != QvmOpcode::OpConst || target.operand != owner as i32)
            {
                return value.fail("pickup call differs from its declared original target");
            }
            Ok(index)
        })
    };
    let read_function_calls = |at: &ProfileReader<'_>| -> Result<FunctionCalls, GuestError> {
        let target = read_entry(&at.field("entry")?)?;
        Ok(FunctionCalls { entry: target, calls: read_calls(&at.field("calls")?, target)? })
    };
    let read_argument = |at: &ProfileReader<'_>| -> Result<usize, GuestError> {
        let index = at.integer(0)? as usize;
        if index >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
            return at.fail("pickup argument is outside the source ABI");
        }
        Ok(index)
    };
    let gate_reader = reader.field("gate")?;
    let gate_calls = read_function_calls(&gate_reader)?;
    let gate = GateProfile {
        entry: gate_calls.entry,
        calls: gate_calls.calls,
        item_argument: read_argument(&gate_reader.field("itemArgument")?)?,
        player_argument: read_argument(&gate_reader.field("playerArgument")?)?,
    };
    let read_evaluation = |at: &ProfileReader<'_>, owner: usize| -> Result<QvmRegionEvaluation, GuestError> {
        let result = QvmRegionEvaluation {
            entry: at.field("entry")?.integer(0)? as usize,
            join: at.field("join")?.integer(0)? as usize,
            inputs: at.field("inputs")?.list(|value| value.integer(8).map(|offset| offset as usize))?,
            result: at.field("result")?.nullable(|value| value.integer(8).map(|offset| offset as usize))?,
        };
        qualify_qvm_region_evaluation(instructions, owner, &result, false)?;
        Ok(result)
    };
    let mut grants = Vec::new();
    for grant in reader.field("grants")?.list(|at| {
        let source = read_function_calls(at)?;
        let op = at.field("operation")?;
        let kind = op.field("kind")?.choice(&["return", "region"])?;
        let operation = if kind == "return" {
            let accepted = op.field("acceptedReturn")?.integer(source.entry as i64 + 1)? as usize;
            let value = instructions.get(accepted);
            if accepted + 1 >= owner_end(source.entry)
                || value.is_none_or(|value| value.opcode != QvmOpcode::OpConst || value.operand == 0)
                || instructions.get(accepted + 1).is_none_or(|next| next.opcode != QvmOpcode::OpLeave)
            {
                return op.fail("pickup accepted return must be original nonzero CONST/LEAVE in its owning function");
            }
            PickupOperation::Return { accepted_return: accepted }
        } else {
            let region = (
                op.field("entry")?.integer(0)? as usize,
                op.field("join")?.integer(0)? as usize,
                op.field("quantity")?.integer(8)? as usize,
            );
            let frame = qualify_qvm_region(instructions, source.entry, region.0, region.1)?;
            if region.2 % 4 != 0 || region.2 + 4 > frame {
                return op.fail("pickup quantity is outside the original local frame");
            }
            let weapon = op.field("weapon")?;
            if !weapon.is_undefined() && !weapon.field("storage")?.is_undefined() {
                weapon.field("storage")?.literal_str("inventory")?;
            }
            let grant = if weapon.is_undefined() {
                None
            } else {
                let storage = weapon.field("storage")?;
                Some(WeaponGrant {
                    location: if !storage.is_undefined() && storage.value() == &super::mod_provider::ProfileValue::Str("inventory".to_string()) {
                        storage.literal_str("inventory")?;
                        WeaponGrantLocation::Inventory
                    } else {
                        WeaponGrantLocation::Offsets {
                            bits_offset: weapon.field("bitsOffset")?.integer(0)? as usize,
                            ammo_offset: weapon.field("ammoOffset")?.integer(0)? as usize,
                        }
                    },
                    quantity: read_evaluation(&weapon.field("quantity")?, source.entry)?,
                })
            };
            PickupOperation::Region { entry: region.0, join: region.1, quantity: region.2, weapon: grant }
        };
        let seen = RefCell::new(HashSet::new());
        let branches = at.field("eligibility")?.field("branches")?.list(|value| {
            let instruction_index = value.field("instructionIndex")?.integer(gate.entry as i64 + 1)? as usize;
            let instruction = instructions.get(instruction_index);
            if instruction_index >= owner_end(gate.entry) || instruction.is_none_or(|instruction| !instruction.opcode.is_branch()) || !seen.borrow_mut().insert(instruction_index) {
                return value.fail("eligibility override must name a distinct original conditional inside the gate");
            }
            Ok(EligibilityOverride { instruction_index, taken: value.field("taken")?.boolean()? })
        })?;
        Ok(QvmPickupGrant { entry: source.entry, calls: source.calls, item_type: at.field("itemType")?.integer(0)?, operation, eligibility: branches })
    })? {
        grants.push(grant);
    }
    let entity_stride = reader.field("entityStride")?.integer(qvm_shared_entity_bytes(abi_profile) as i64)? as usize;
    let client_stride = reader.field("clientStride")?.integer(qvm_player_state_bytes(abi_profile) as i64)? as usize;
    if entity_stride % 4 != 0 || client_stride % 4 != 0 {
        return reader.fail("source record strides must be aligned");
    }
    let fields = reader.field("fields")?;
    let read_field = |name: &str| -> Result<usize, GuestError> {
        let value = fields.field(name)?.integer(0)? as usize;
        if value % 4 != 0 || value + 4 > entity_stride {
            return fields.field(name)?.fail("pickup field is outside its aligned entity record");
        }
        Ok(value)
    };
    Ok(QvmPickupProfile {
        module: artifact.module.clone(),
        abi_profile,
        entity_stride,
        client_stride,
        fields: PickupFields {
            inuse: read_field("inuse")?,
            client: read_field("client")?,
            health: read_field("health")?,
            item: read_field("item")?,
            count: read_field("count")?,
            flags: read_field("flags")?,
        },
        dropped_flag: reader.field("droppedFlag")?.integer(0)?,
        items: parse_qvm_item_layout(&reader.field("items")?)?,
        touch: read_entry(&reader.field("touch")?)?,
        gate,
        targets: read_function_calls(&reader.field("targets")?)?,
        free: read_entry(&reader.field("free")?)?,
        objective_types: reader.field("objectiveTypes")?.list(|at| at.integer(0))?,
        grants,
    })
}

#[cfg(test)]
mod tests {
    use super::super::mod_provider::{ModuleId, ProfileValue, QvmImage, QvmInstruction, QvmRole};
    use super::*;

    fn fixture_artifact() -> QvmArtifact {
        QvmArtifact {
            module: ModuleId { id: "test:game".to_string(), artifact_path: "vm/qagame.qvm".to_string(), digest: "sha256:game".to_string(), revision: "1".to_string() },
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![
                    QvmInstruction::word(QvmOpcode::OpEnter, 64),
                    QvmInstruction::word(QvmOpcode::OpEq, 0),
                    QvmInstruction::word(QvmOpcode::OpConst, 1),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpConst, 4),
                    QvmInstruction::word(QvmOpcode::OpCall, 0),
                    QvmInstruction::word(QvmOpcode::OpConst, 9),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                ],
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 8192,
            },
        }
    }

    fn int(value: i64) -> ProfileValue {
        ProfileValue::Int(value)
    }

    fn text(value: &str) -> ProfileValue {
        ProfileValue::Str(value.to_string())
    }

    fn rec(fields: Vec<(&str, ProfileValue)>) -> ProfileValue {
        ProfileValue::record(fields)
    }

    fn arr(items: Vec<ProfileValue>) -> ProfileValue {
        ProfileValue::Array(items)
    }

    fn layout_value() -> ProfileValue {
        rec(vec![
            ("address", int(64)),
            ("count", int(8)),
            ("stride", int(32)),
            ("fields", rec(vec![("className", int(0)), ("pickupName", int(4)), ("type", int(8)), ("tag", int(12))])),
            ("weaponType", int(2)),
            ("ammoType", int(3)),
        ])
    }

    fn declaration() -> ProfileValue {
        rec(vec![
            ("entityStride", int(520)),
            ("clientStride", int(468)),
            (
                "fields",
                rec(vec![
                    ("inuse", int(516)),
                    ("client", int(64)),
                    ("health", int(68)),
                    ("item", int(72)),
                    ("count", int(76)),
                    ("flags", int(80)),
                ]),
            ),
            ("droppedFlag", int(1)),
            ("items", layout_value()),
            ("touch", int(9)),
            (
                "gate",
                rec(vec![
                    ("entry", int(0)),
                    ("calls", arr(vec![])),
                    ("itemArgument", int(0)),
                    ("playerArgument", int(1)),
                ]),
            ),
            ("targets", rec(vec![("entry", int(11)), ("calls", arr(vec![]))])),
            ("free", int(13)),
            ("objectiveTypes", arr(vec![int(4)])),
            (
                "grants",
                arr(vec![rec(vec![
                    ("entry", int(4)),
                    ("calls", arr(vec![int(6)])),
                    ("itemType", int(2)),
                    ("operation", rec(vec![("kind", text("return")), ("acceptedReturn", int(7))])),
                    ("eligibility", rec(vec![("branches", arr(vec![rec(vec![("instructionIndex", int(1)), ("taken", ProfileValue::Bool(true))])]))])),
                ])]),
            ),
        ])
    }

    #[test]
    fn layout_parses_table_extents() {
        let layout = parse_qvm_item_layout(&ProfileReader::new(&layout_value())).unwrap();
        assert_eq!(layout.stride, 32);
        assert!(!layout.live);
        assert_eq!(layout, parse_qvm_item_layout(&ProfileReader::new(&layout_value())).unwrap());
        let bad = rec(vec![
            ("address", int(64)),
            ("count", int(8)),
            ("stride", int(32)),
            ("fields", rec(vec![("className", int(0)), ("pickupName", int(4)), ("type", int(8)), ("tag", int(30))])),
            ("weaponType", int(2)),
            ("ammoType", int(3)),
        ]);
        assert!(parse_qvm_item_layout(&ProfileReader::new(&bad)).is_err());
        let same_type = rec(vec![
            ("address", int(64)),
            ("count", int(8)),
            ("stride", int(32)),
            ("fields", rec(vec![("className", int(0)), ("pickupName", int(4)), ("type", int(8)), ("tag", int(12))])),
            ("weaponType", int(2)),
            ("ammoType", int(2)),
        ]);
        assert!(parse_qvm_item_layout(&ProfileReader::new(&same_type)).is_err());
    }

    #[test]
    fn profile_reads_gate_and_grants() {
        let profile = read_qvm_primary_pickup_profile(&ProfileReader::new(&declaration()), &fixture_artifact()).unwrap();
        assert_eq!(profile.gate.entry, 0);
        assert_eq!(profile.grants.len(), 1);
        assert_eq!(profile.grants[0].item_type, 2);
        assert!(matches!(profile.grants[0].operation, PickupOperation::Return { accepted_return: 7 }));
        assert_eq!(profile.grants[0].eligibility.len(), 1);
        assert!(QvmPickupGrant::decide(&profile.grants[0].eligibility[0]));
        assert_eq!(profile.fields.client, 64);
    }

    #[test]
    fn profile_rejects_bad_calls_and_branches() {
        let mut bad = declaration();
        if let ProfileValue::Record(fields) = &mut bad {
            for (name, value) in fields.iter_mut() {
                if name == "grants" {
                    *value = arr(vec![rec(vec![
                        ("entry", int(4)),
                        ("calls", arr(vec![int(5)])),
                        ("itemType", int(2)),
                        ("operation", rec(vec![("kind", text("return")), ("acceptedReturn", int(7))])),
                        ("eligibility", rec(vec![("branches", arr(vec![]))])),
                    ])]);
                }
            }
        }
        assert!(read_qvm_primary_pickup_profile(&ProfileReader::new(&bad), &fixture_artifact()).is_err());
        let mut duplicated = declaration();
        if let ProfileValue::Record(fields) = &mut duplicated {
            for (name, value) in fields.iter_mut() {
                if name == "grants" {
                    if let ProfileValue::Array(grants) = value {
                        if let ProfileValue::Record(grant) = &mut grants[0] {
                            for (key, entry) in grant.iter_mut() {
                                if key == "eligibility" {
                                    *entry = rec(vec![(
                                        "branches",
                                        arr(vec![
                                            rec(vec![("instructionIndex", int(1)), ("taken", ProfileValue::Bool(true))]),
                                            rec(vec![("instructionIndex", int(1)), ("taken", ProfileValue::Bool(false))]),
                                        ]),
                                    )]);
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(read_qvm_primary_pickup_profile(&ProfileReader::new(&duplicated), &fixture_artifact()).is_err());
    }

    #[test]
    fn region_grants_qualify_quantity_frames() {
        let artifact = QvmArtifact {
            module: ModuleId { id: "test:game".to_string(), artifact_path: "vm/qagame.qvm".to_string(), digest: "sha256:game".to_string(), revision: "1".to_string() },
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![
                    QvmInstruction::word(QvmOpcode::OpEnter, 64),
                    QvmInstruction::word(QvmOpcode::OpEq, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 16),
                    QvmInstruction::word(QvmOpcode::OpConst, 3),
                    QvmInstruction::word(QvmOpcode::OpCall, 0),
                    QvmInstruction::word(QvmOpcode::OpConst, 1),
                    QvmInstruction::word(QvmOpcode::OpPop, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                ],
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 8192,
            },
        };
        let declaration = rec(vec![
            ("entityStride", int(520)),
            ("clientStride", int(468)),
            (
                "fields",
                rec(vec![
                    ("inuse", int(516)),
                    ("client", int(64)),
                    ("health", int(68)),
                    ("item", int(72)),
                    ("count", int(76)),
                    ("flags", int(80)),
                ]),
            ),
            ("droppedFlag", int(1)),
            ("items", layout_value()),
            ("touch", int(9)),
            ("gate", rec(vec![("entry", int(0)), ("calls", arr(vec![])), ("itemArgument", int(0)), ("playerArgument", int(1))])),
            ("targets", rec(vec![("entry", int(11)), ("calls", arr(vec![]))])),
            ("free", int(13)),
            ("objectiveTypes", arr(vec![])),
            (
                "grants",
                arr(vec![rec(vec![
                    ("entry", int(3)),
                    ("calls", arr(vec![int(5)])),
                    ("itemType", int(2)),
                    (
                        "operation",
                        rec(vec![
                            ("kind", text("region")),
                            ("entry", int(6)),
                            ("join", int(8)),
                            ("quantity", int(8)),
                            (
                                "weapon",
                                rec(vec![
                                    ("storage", text("inventory")),
                                    (
                                        "quantity",
                                        rec(vec![
                                            ("entry", int(6)),
                                            ("join", int(8)),
                                            ("inputs", arr(vec![])),
                                            ("result", ProfileValue::Null),
                                        ]),
                                    ),
                                ]),
                            ),
                        ]),
                    ),
                    ("eligibility", rec(vec![("branches", arr(vec![]))])),
                ])]),
            ),
        ]);
        let profile = read_qvm_primary_pickup_profile(&ProfileReader::new(&declaration), &artifact).unwrap();
        assert!(matches!(
            profile.grants[0].operation,
            PickupOperation::Region { quantity: 8, weapon: Some(WeaponGrant { location: WeaponGrantLocation::Inventory, .. }), .. }
        ));
    }
}