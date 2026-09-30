//! Primary inventory profile: private source storage or public capacity queries.
//!
//! Provenance: `src/compat/qvm/primary-inventory-profile.ts`.
//!
//! Local mirrors: [`QvmInventoryProfile`] (`game-inventory.ts`),
//! [`read_qvm_item_storage`] (`content/mods/qvm-items.ts`; storage validation
//! reuses [`super::mod_provider::validate_qvm_item_storage_mirror`]).
//! Capacity evaluation delegates to [`CapacityModule`]; the donor's closures
//! are a declarative [`InventoryCapacity`] plus [`CapacityContext`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::mod_provider::{
    ProfileReader, QvmAbi, QvmArtifact, QvmImage, QvmOpcode, QvmRegionEvaluation, namespaced_id, qualify_qvm_region_evaluation,
};
use super::mod_weapon_stage::{CapacityOverride, OverrideComparison, QvmItemCapacity, QvmItemField, QvmItemStorage, PackedItem};
use crate::error::GuestError;

/// Read one item storage declaration (mirror of `readQvmItemStorage`).
pub fn read_qvm_item_storage(reader: &ProfileReader<'_>) -> Result<QvmItemStorage, GuestError> {
    let field = reader.field("field")?;
    let source = QvmItemField { record: field.field("record")?.string()?, offset: field.field("offset")?.integer(0)? as usize };
    if reader.field("kind")?.choice(&["counter", "bits"])? == "counter" {
        let capacity = reader.field("capacity")?;
        let kind = capacity.field("kind")?.choice(&["field", "constant", "source"])?;
        Ok(QvmItemStorage::Counter {
            field: source,
            item: namespaced_id(&reader.field("item")?)?,
            capacity: match kind.as_str() {
                "field" => {
                    let target = capacity.field("field")?;
                    QvmItemCapacity::Field(QvmItemField {
                        record: target.field("record")?.string()?,
                        offset: target.field("offset")?.integer(0)? as usize,
                    })
                }
                "constant" => {
                    let value = capacity.field("value")?.integer(0)?;
                    if value > i64::from(i32::MAX) {
                        return capacity.fail("QVM item capacity exceeds its source ABI");
                    }
                    QvmItemCapacity::Constant(value as i32)
                }
                _ => QvmItemCapacity::Source {
                    instruction: capacity.field("instruction")?.integer(0)? as usize,
                    overrides: capacity.field("overrides")?.list(|value| {
                        Ok(CapacityOverride {
                            address: value.field("address")?.integer(0)? as usize,
                            comparison: match value.field("comparison")?.choice(&["equals", "not-equals"])?.as_str() {
                                "equals" => OverrideComparison::Equals,
                                _ => OverrideComparison::NotEquals,
                            },
                            value: value.field("value")?.integer(i64::from(i32::MIN))?,
                            instruction: value.field("instruction")?.integer(0)? as usize,
                        })
                    })?,
                },
            },
        })
    } else {
        let private_mask = reader.field("privateMask")?.integer(0)?;
        if private_mask > i64::from(u32::MAX) {
            return reader.fail("Invalid QVM private inventory mask");
        }
        Ok(QvmItemStorage::Bits {
            field: source,
            private_mask: private_mask as u32,
            items: reader.field("items")?.list(|value| {
                let mask = value.field("mask")?.integer(1)?;
                if mask > 0x8000_0000 {
                    return value.fail("QVM packed item masks overlap");
                }
                Ok(PackedItem { item: namespaced_id(&value.field("item")?)?, mask: mask as u32 })
            })?,
        })
    }
}

/// Capacity query context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapacityContext {
    /// Client record pointer.
    pub client: i32,
    /// Entity record pointer.
    pub entity: i32,
    /// Client number.
    pub client_number: i32,
}

/// BSS stack reservation for source queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StackReservation {
    /// Start address.
    pub start: usize,
    /// End address.
    pub end: usize,
}

/// Capacity source word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapacityWord {
    /// Constant word.
    Constant(i32),
    /// Weapon number.
    Weapon,
    /// Client pointer.
    Client,
    /// Entity pointer.
    Entity,
    /// Client number.
    ClientNumber,
}

/// Capacity argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapacityArgument {
    /// Maximum grant (`0x7fffffff`).
    MaximumGrant,
    /// Source word.
    Word(CapacityWord),
}

/// Public capacity evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum InventoryCapacity {
    /// Constant limit.
    Constant(i32),
    /// Global word.
    Global {
        /// Address.
        address: usize,
    },
    /// Counter invocation.
    Counter {
        /// Owning function.
        owner: usize,
        /// Helper functions.
        functions: Vec<usize>,
        /// Arguments.
        arguments: Vec<CapacityArgument>,
        /// Ammo offset.
        ammo_offset: usize,
        /// Stack reservation, if any.
        stack: Option<StackReservation>,
    },
    /// Region evaluation.
    Region {
        /// Owning function.
        owner: usize,
        /// Region.
        region: QvmRegionEvaluation,
        /// Arguments.
        arguments: Vec<CapacityWord>,
        /// Live-in inputs.
        inputs: Vec<CapacityWord>,
        /// Stack reservation, if any.
        stack: Option<StackReservation>,
    },
}

/// Module services evaluating capacities.
pub trait CapacityModule {
    /// Invoke a counter function.
    fn evaluate_counter(&self, arguments: &[i32], owner: usize, address: usize, functions: &[usize], stack: Option<&StackReservation>) -> Result<i32, GuestError>;
    /// Evaluate a region.
    fn evaluate_region(
        &self,
        arguments: &[i32],
        owner: usize,
        region: &QvmRegionEvaluation,
        inputs: &[i32],
        stack: Option<&StackReservation>,
    ) -> Result<i32, GuestError>;
    /// Read a global word.
    fn read_global(&self, address: usize) -> Result<i32, GuestError>;
}

fn word_value(source: CapacityWord, weapon: i32, context: CapacityContext) -> i32 {
    match source {
        CapacityWord::Constant(value) => value,
        CapacityWord::Weapon => weapon,
        CapacityWord::Client => context.client,
        CapacityWord::Entity => context.entity,
        CapacityWord::ClientNumber => context.client_number,
    }
}

impl InventoryCapacity {
    /// Evaluate the capacity for a weapon.
    pub fn evaluate(&self, module: &dyn CapacityModule, weapon: i32, context: CapacityContext) -> Result<i32, GuestError> {
        match self {
            Self::Constant(limit) => Ok(*limit),
            Self::Global { address } => module.read_global(*address),
            Self::Counter { owner, functions, arguments, ammo_offset, stack } => {
                let lowered: Vec<i32> = arguments
                    .iter()
                    .map(|source| match source {
                        CapacityArgument::MaximumGrant => 0x7fff_ffff,
                        CapacityArgument::Word(word) => word_value(*word, weapon, context),
                    })
                    .collect();
                module.evaluate_counter(&lowered, *owner, (context.client as usize) + ammo_offset + weapon as usize * 4, functions, stack.as_ref())
            }
            Self::Region { owner, region, arguments, inputs, stack } => {
                let lowered: Vec<i32> = arguments.iter().map(|source| word_value(*source, weapon, context)).collect();
                let live: Vec<i32> = inputs.iter().map(|source| word_value(*source, weapon, context)).collect();
                module.evaluate_region(&lowered, *owner, region, &live, stack.as_ref())
            }
        }
    }
}

/// Primary inventory profile (mirror of `QvmInventoryProfile`).
#[derive(Debug, Clone, PartialEq)]
pub enum QvmInventoryProfile {
    /// Private source storage.
    Private {
        /// Module identity.
        module: super::mod_provider::ModuleId,
        /// ABI profile.
        abi_profile: QvmAbi,
        /// Entity stride.
        entity_stride: usize,
        /// Client stride.
        client_stride: usize,
        /// Executable image.
        image: QvmImage,
        /// Storage.
        storage: Vec<QvmItemStorage>,
    },
    /// Public capacity queries.
    Public {
        /// Module identity.
        module: super::mod_provider::ModuleId,
        /// ABI profile.
        abi_profile: QvmAbi,
        /// Weapons offset.
        weapons_offset: usize,
        /// Ammo offset.
        ammo_offset: usize,
        /// Capacity.
        capacity: InventoryCapacity,
    },
}

/// Qualified primary record strides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrimaryRecordStrides {
    /// Client stride.
    pub client_stride: usize,
    /// Entity stride.
    pub entity_stride: usize,
}

/// Read a primary inventory profile.
pub fn read_qvm_primary_inventory_profile(
    reader: &ProfileReader<'_>,
    artifact: &QvmArtifact,
    records: Option<PrimaryRecordStrides>,
) -> Result<QvmInventoryProfile, GuestError> {
    use super::mod_provider::{QVM_MAX_PRIVATE_ARGUMENT_WORDS, qvm_player_state_bytes};
    use super::mod_provider::QvmRole;
    let abi_profile = artifact.abi();
    if artifact.role != QvmRole::Qagame {
        return reader.fail("primary inventory declarations require a qagame ABI");
    }
    if !reader.field("storage")?.is_undefined() {
        let Some(records) = records else {
            return reader.fail("private inventory requires its qualified primary source records");
        };
        let storage = reader.field("storage")?.list(read_qvm_item_storage)?;
        let mut items = HashSet::new();
        for value in &storage {
            match value {
                QvmItemStorage::Counter { item, .. } => {
                    items.insert(item.clone());
                }
                QvmItemStorage::Bits { items: packed, .. } => {
                    for entry in packed {
                        items.insert(entry.item.clone());
                    }
                }
            }
        }
        let occupied: RefCell<HashMap<(String, usize), bool>> = RefCell::new(HashMap::new());
        super::mod_provider::validate_qvm_item_storage_mirror(&storage, &items, &artifact.image, &|field, usage_capacity| {
            let bytes = if field.record == "client" {
                records.client_stride
            } else if field.record == "entity" {
                records.entity_stride
            } else {
                0
            };
            let previous = occupied.borrow().get(&(field.record.clone(), field.offset)).copied();
            if field.offset % 4 != 0 || field.offset + 4 > bytes || previous.is_some_and(|was_capacity| !(was_capacity && usage_capacity)) {
                return reader.fail("private inventory field is outside or overlaps its source record");
            }
            occupied.borrow_mut().insert((field.record.clone(), field.offset), usage_capacity);
            Ok(())
        })?;
        return Ok(QvmInventoryProfile::Private {
            module: artifact.module.clone(),
            abi_profile,
            entity_stride: records.entity_stride,
            client_stride: records.client_stride,
            image: artifact.image.clone(),
            storage,
        });
    }
    let constant = |at: &ProfileReader<'_>| -> Result<i32, GuestError> {
        let instruction = artifact.image.instruction(at.integer(0)? as usize);
        if instruction.is_none_or(|instruction| instruction.opcode != QvmOpcode::OpConst) {
            return at.fail("capacity operand must be an original OP_CONST");
        }
        Ok(instruction.expect("checked const").operand)
    };
    let read_word = |at: &ProfileReader<'_>| -> Result<CapacityWord, GuestError> {
        match at.field("kind")?.choice(&["constant", "weapon", "client", "entity", "client-number"])?.as_str() {
            "constant" => Ok(CapacityWord::Constant(constant(&at.field("instruction")?)?)),
            "weapon" => Ok(CapacityWord::Weapon),
            "client" => Ok(CapacityWord::Client),
            "entity" => Ok(CapacityWord::Entity),
            _ => Ok(CapacityWord::ClientNumber),
        }
    };
    let capacity = reader.field("capacity")?;
    let kind = capacity.field("kind")?.choice(&["constant", "global", "region", "counter"])?;
    let stack_reader = capacity.field("stack")?;
    let stack = if stack_reader.is_undefined() {
        None
    } else {
        Some(StackReservation { start: stack_reader.field("start")?.integer(0)? as usize, end: stack_reader.field("end")?.integer(0)? as usize })
    };
    if let Some(stack) = stack {
        let bss = artifact.image.data_length + artifact.image.literal_length;
        if (kind != "counter" && kind != "region")
            || stack.start % 4 != 0
            || stack.end % 4 != 0
            || stack.start < bss
            || stack.start >= stack.end
            || stack.end > artifact.image.allocated_data_length
        {
            return stack_reader.fail("capacity stack must declare an aligned original BSS reservation for a source query");
        }
    }
    let evaluate = if kind == "constant" {
        let limit = constant(&capacity.field("instruction")?)?;
        if limit < 0 {
            return capacity.fail("source ammo capacity cannot be negative");
        }
        InventoryCapacity::Constant(limit)
    } else if kind == "global" {
        let address = constant(&capacity.field("addressInstruction")?)?;
        if address < 0 || address % 4 != 0 || address as usize + 4 > artifact.image.allocated_data_length {
            return capacity.fail("source capacity word is outside aligned module data");
        }
        InventoryCapacity::Global { address: address as usize }
    } else if kind == "counter" {
        let owner = capacity.field("function")?.integer(0)? as usize;
        let functions = capacity.field("functions")?.list(|at| {
            let index = at.integer(0)? as usize;
            if artifact.image.instruction(index).is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
                return at.fail("counter operation requires original function entries");
            }
            Ok(index)
        })?;
        if !functions.contains(&owner) || functions.iter().collect::<HashSet<_>>().len() != functions.len() {
            return capacity.fail("counter operation must include its entry and distinct helpers");
        }
        let arguments = capacity.field("arguments")?.list(|at| {
            if at.field("kind")?.value() == &super::mod_provider::ProfileValue::Str("maximum-grant".to_string()) {
                at.field("kind")?.literal_str("maximum-grant")?;
                Ok(CapacityArgument::MaximumGrant)
            } else {
                read_word(at).map(CapacityArgument::Word)
            }
        })?;
        if arguments.len() > QVM_MAX_PRIVATE_ARGUMENT_WORDS {
            return capacity.fail("counter arguments exceed the private source invocation");
        }
        let ammo_offset = reader.field("ammoOffset")?.integer(0)? as usize;
        InventoryCapacity::Counter { owner, functions, arguments, ammo_offset, stack }
    } else {
        let owner = capacity.field("function")?.integer(0)? as usize;
        let source = capacity.field("region")?;
        let region = QvmRegionEvaluation {
            entry: source.field("entry")?.integer(0)? as usize,
            join: source.field("join")?.integer(0)? as usize,
            inputs: source.field("inputs")?.list(|at| at.integer(8).map(|offset| offset as usize))?,
            result: Some(source.field("result")?.integer(8)? as usize),
        };
        qualify_qvm_region_evaluation(&artifact.image.instructions, owner, &region, true)?;
        let arguments = capacity.field("arguments")?.list(&read_word)?;
        let inputs = capacity.field("inputs")?.list(&read_word)?;
        if arguments.len() > QVM_MAX_PRIVATE_ARGUMENT_WORDS || inputs.len() != region.inputs.len() {
            return capacity.fail("capacity arguments and live-ins differ from the original source frame");
        }
        InventoryCapacity::Region { owner, region, arguments, inputs, stack }
    };
    let offset = |name: &str| -> Result<usize, GuestError> {
        let at = reader.field(name)?;
        let result = at.integer(0)? as usize;
        if result % 4 != 0 || result + if name == "ammoOffset" { 64 } else { 4 } > qvm_player_state_bytes(abi_profile) {
            return at.fail("inventory field exceeds its aligned public player record");
        }
        Ok(result)
    };
    Ok(QvmInventoryProfile::Public {
        module: artifact.module.clone(),
        abi_profile,
        weapons_offset: offset("weaponsOffset")?,
        ammo_offset: offset("ammoOffset")?,
        capacity: evaluate,
    })
}

#[cfg(test)]
mod tests {
    use super::super::mod_provider::{ModuleId, QvmInstruction, QvmRole};
    use super::*;

    fn fixture_artifact() -> QvmArtifact {
        QvmArtifact {
            module: ModuleId { id: "test:game".to_string(), artifact_path: "vm/qagame.qvm".to_string(), digest: "sha256:game".to_string(), revision: "1".to_string() },
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![
                    QvmInstruction::word(QvmOpcode::OpEnter, 64),
                    QvmInstruction::word(QvmOpcode::OpConst, 200),
                    QvmInstruction::word(QvmOpcode::OpConst, 64),
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

    struct FakeModule {
        global: i32,
        counter: i32,
    }

    impl CapacityModule for FakeModule {
        fn evaluate_counter(&self, _arguments: &[i32], _owner: usize, _address: usize, _functions: &[usize], _stack: Option<&StackReservation>) -> Result<i32, GuestError> {
            Ok(self.counter)
        }
        fn evaluate_region(
            &self,
            _arguments: &[i32],
            _owner: usize,
            _region: &QvmRegionEvaluation,
            _inputs: &[i32],
            _stack: Option<&StackReservation>,
        ) -> Result<i32, GuestError> {
            Ok(7)
        }
        fn read_global(&self, _address: usize) -> Result<i32, GuestError> {
            Ok(self.global)
        }
    }

    #[test]
    fn private_storage_validates_fields() {
        let declaration = ProfileValue::record(vec![(
            "storage",
            ProfileValue::Array(vec![ProfileValue::record(vec![
                ("kind", ProfileValue::Str("counter".to_string())),
                ("field", ProfileValue::record(vec![("record", ProfileValue::Str("client".to_string())), ("offset", ProfileValue::Int(8))])),
                ("item", ProfileValue::Str("test:shells".to_string())),
                ("capacity", ProfileValue::record(vec![("kind", ProfileValue::Str("constant".to_string())), ("value", ProfileValue::Int(50))])),
            ])]),
        )]);
        let artifact = fixture_artifact();
        let records = PrimaryRecordStrides { client_stride: 468, entity_stride: 520 };
        let profile = read_qvm_primary_inventory_profile(&ProfileReader::new(&declaration), &artifact, Some(records)).unwrap();
        assert!(matches!(profile, QvmInventoryProfile::Private { .. }));
        assert!(read_qvm_primary_inventory_profile(&ProfileReader::new(&declaration), &artifact, None).is_err());
    }

    #[test]
    fn private_storage_rejects_overlaps() {
        let field = |offset: i64| ProfileValue::record(vec![("record", ProfileValue::Str("client".to_string())), ("offset", ProfileValue::Int(offset))]);
        let counter = |item: &str, offset: i64| {
            ProfileValue::record(vec![
                ("kind", ProfileValue::Str("counter".to_string())),
                ("field", field(offset)),
                ("item", ProfileValue::Str(item.to_string())),
                ("capacity", ProfileValue::record(vec![("kind", ProfileValue::Str("constant".to_string())), ("value", ProfileValue::Int(10))])),
            ])
        };
        let declaration = ProfileValue::record(vec![("storage", ProfileValue::Array(vec![counter("test:a", 8), counter("test:b", 8)]))]);
        let artifact = fixture_artifact();
        let records = PrimaryRecordStrides { client_stride: 468, entity_stride: 520 };
        assert!(read_qvm_primary_inventory_profile(&ProfileReader::new(&declaration), &artifact, Some(records)).is_err());
    }

    #[test]
    fn public_constant_and_global_evaluate() {
        let public = |capacity: ProfileValue| {
            ProfileValue::record(vec![
                ("capacity", capacity),
                ("weaponsOffset", ProfileValue::Int(8)),
                ("ammoOffset", ProfileValue::Int(64)),
            ])
        };
        let artifact = fixture_artifact();
        let constant = public(ProfileValue::record(vec![
            ("kind", ProfileValue::Str("constant".to_string())),
            ("instruction", ProfileValue::Int(1)),
        ]));
        let profile = read_qvm_primary_inventory_profile(&ProfileReader::new(&constant), &artifact, None).unwrap();
        let QvmInventoryProfile::Public { capacity, ammo_offset, .. } = profile else { panic!("expected public profile") };
        assert_eq!(ammo_offset, 64);
        let module = FakeModule { global: 9, counter: 11 };
        let context = CapacityContext { client: 100, entity: 200, client_number: 0 };
        assert_eq!(capacity.evaluate(&module, 3, context).unwrap(), 200);
        let global = public(ProfileValue::record(vec![
            ("kind", ProfileValue::Str("global".to_string())),
            ("addressInstruction", ProfileValue::Int(2)),
        ]));
        let profile = read_qvm_primary_inventory_profile(&ProfileReader::new(&global), &artifact, None).unwrap();
        let QvmInventoryProfile::Public { capacity, .. } = profile else { panic!("expected public profile") };
        assert_eq!(capacity.evaluate(&module, 3, context).unwrap(), 9);
    }

    #[test]
    fn counter_maps_maximum_grant() {
        let declaration = ProfileValue::record(vec![
            (
                "capacity",
                ProfileValue::record(vec![
                    ("kind", ProfileValue::Str("counter".to_string())),
                    ("function", ProfileValue::Int(0)),
                    ("functions", ProfileValue::Array(vec![ProfileValue::Int(0), ProfileValue::Int(4)])),
                    (
                        "arguments",
                        ProfileValue::Array(vec![
                            ProfileValue::record(vec![("kind", ProfileValue::Str("maximum-grant".to_string()))]),
                            ProfileValue::record(vec![("kind", ProfileValue::Str("weapon".to_string()))]),
                        ]),
                    ),
                ]),
            ),
            ("weaponsOffset", ProfileValue::Int(8)),
            ("ammoOffset", ProfileValue::Int(64)),
        ]);
        let artifact = fixture_artifact();
        let profile = read_qvm_primary_inventory_profile(&ProfileReader::new(&declaration), &artifact, None).unwrap();
        let QvmInventoryProfile::Public { capacity, .. } = profile else { panic!("expected public profile") };
        struct Probe;
        impl CapacityModule for Probe {
            fn evaluate_counter(
                &self,
                arguments: &[i32],
                owner: usize,
                address: usize,
                functions: &[usize],
                _stack: Option<&StackReservation>,
            ) -> Result<i32, GuestError> {
                assert_eq!(arguments, &[0x7fff_ffff, 2]);
                assert_eq!((owner, address), (0, 100 + 64 + 2 * 4));
                assert_eq!(functions, &[0, 4]);
                Ok(42)
            }
            fn evaluate_region(
                &self,
                _arguments: &[i32],
                _owner: usize,
                _region: &QvmRegionEvaluation,
                _inputs: &[i32],
                _stack: Option<&StackReservation>,
            ) -> Result<i32, GuestError> {
                unreachable!()
            }
            fn read_global(&self, _address: usize) -> Result<i32, GuestError> {
                unreachable!()
            }
        }
        let context = CapacityContext { client: 100, entity: 200, client_number: 1 };
        assert_eq!(capacity.evaluate(&Probe, 2, context).unwrap(), 42);
    }

    #[test]
    fn region_requires_matching_live_ins() {
        let declaration = ProfileValue::record(vec![
            (
                "capacity",
                ProfileValue::record(vec![
                    ("kind", ProfileValue::Str("region".to_string())),
                    ("function", ProfileValue::Int(0)),
                    (
                        "region",
                        ProfileValue::record(vec![
                            ("entry", ProfileValue::Int(1)),
                            ("join", ProfileValue::Int(3)),
                            ("inputs", ProfileValue::Array(vec![ProfileValue::Int(8)])),
                            ("result", ProfileValue::Int(12)),
                        ]),
                    ),
                    ("arguments", ProfileValue::Array(vec![])),
                    ("inputs", ProfileValue::Array(vec![])),
                ]),
            ),
            ("weaponsOffset", ProfileValue::Int(8)),
            ("ammoOffset", ProfileValue::Int(64)),
        ]);
        let artifact = fixture_artifact();
        assert!(read_qvm_primary_inventory_profile(&ProfileReader::new(&declaration), &artifact, None).is_err());
    }
}
