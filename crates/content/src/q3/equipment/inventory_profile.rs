//! Q3 native inventory profile (`inventory-profile.ts`).

use std::collections::HashMap;
use std::rc::Rc;

use qa_guest::error::GuestError;
use qa_guest::qvm::game_data::{artifact_abi, AbiProfile, QvmArtifact, QvmOpcode, QvmRole, QvmSharedMemory};
use qa_guest::qvm::game_inventory::{QvmInventoryCapacityContext, QvmInventoryProfile, QvmPublicInventoryProfile};

use super::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;

const STOCK_DIGEST: &str = "sha256:57c52bf22e4f528c064f8af1553a7103723bab0a02276bb11eed944bf829b219";

fn constant(artifact: &QvmArtifact, index: i32) -> Result<i32, GuestError> {
    usize::try_from(index)
        .ok()
        .and_then(|index| artifact.image.instructions.get(index))
        .filter(|instruction| instruction.opcode == QvmOpcode::OpConst)
        .map(|instruction| instruction.operand)
        .ok_or_else(|| GuestError::invalid("QVM inventory source constant is not its qualified instruction"))
}

fn region_is(artifact: &QvmArtifact, index: i32, opcode: QvmOpcode) -> bool {
    index
        .checked_add(1)
        .and_then(|next| usize::try_from(next).ok())
        .and_then(|next| artifact.image.instructions.get(next))
        .is_some_and(|instruction| instruction.opcode == opcode)
}

fn cap(artifact: &QvmArtifact, comparison: i32, store: i32) -> Result<i32, GuestError> {
    let value = constant(artifact, comparison)?;
    if constant(artifact, store)? != value
        || !region_is(artifact, comparison, QvmOpcode::OpLei)
        || !region_is(artifact, store, QvmOpcode::OpStore4)
        || value < 0
    {
        return Err(GuestError::invalid(
            "QVM inventory capacity comparison and store disagree",
        ));
    }
    Ok(value)
}

/// Capacity metadata reads the pinned `Add_Ammo` instructions, not host
/// pickup formulas.
pub fn q3_native_inventory_profile(artifact: &QvmArtifact) -> Result<Option<QvmInventoryProfile>, GuestError> {
    if artifact.module.digest != STOCK_DIGEST && artifact.module.digest != THREEWAVE_GRAPPLE_DIGEST {
        return Ok(None);
    }
    if artifact.role != QvmRole::Qagame || artifact_abi(artifact) != AbiProfile::Modern {
        return Err(GuestError::invalid(
            "Qualified Q3 inventory requires its modern server ABI",
        ));
    }
    let abi_profile = artifact_abi(artifact);
    if artifact.module.digest == STOCK_DIGEST {
        let limit = cap(artifact, 103202, 103216)?;
        return Ok(Some(QvmInventoryProfile::Public(QvmPublicInventoryProfile {
            module: artifact.module.clone(),
            abi_profile,
            weapons_offset: 192,
            ammo_offset: 376,
            capacity: Rc::new(
                move |_: &QvmSharedMemory, _: i32, _: &QvmInventoryCapacityContext| -> Result<i32, GuestError> {
                    Ok(limit)
                },
            ),
        })));
    }
    let ordinary = cap(artifact, 166830, 166844)?;
    let fallback = cap(artifact, 166753, 166764)?;
    let game_type = constant(artifact, 166769)?;
    let special_mode = constant(artifact, 166771)?;
    let lithium = constant(artifact, 166773)?;
    let first = constant(artifact, 166498)?;
    let last = constant(artifact, 166507)?;
    let jump_table = constant(artifact, 166514)?;
    let data = &artifact.image.initialized_data;
    let mut limits = HashMap::new();
    for weapon in first..=last {
        let offset = jump_table as i64 + weapon as i64 * 4;
        let at = usize::try_from(offset)
            .ok()
            .and_then(|at| data.get(at..at + 4))
            .ok_or_else(|| GuestError::invalid("QVM inventory jump table exceeds initialized data"))?;
        let branch = i32::from_le_bytes([at[0], at[1], at[2], at[3]]);
        let entry = branch
            .checked_add(10)
            .ok_or_else(|| GuestError::invalid("QVM inventory source constant is not its qualified instruction"))?;
        let join = branch
            .checked_add(21)
            .ok_or_else(|| GuestError::invalid("QVM inventory source constant is not its qualified instruction"))?;
        limits.insert(weapon, cap(artifact, entry, join)?);
    }
    Ok(Some(QvmInventoryProfile::Public(QvmPublicInventoryProfile {
        module: artifact.module.clone(),
        abi_profile,
        weapons_offset: 204,
        ammo_offset: 376,
        capacity: Rc::new(
            move |memory: &QvmSharedMemory, weapon: i32, _: &QvmInventoryCapacityContext| -> Result<i32, GuestError> {
                let address = |word: i32| {
                    usize::try_from(word)
                        .map_err(|_| GuestError::invalid("QVM inventory capacity address is outside guest memory"))
                };
                let special =
                    memory.read_i32(address(game_type)?)? == special_mode || memory.read_i32(address(lithium)?)? != 0;
                Ok(if special {
                    limits.get(&weapon).copied().unwrap_or(fallback)
                } else {
                    ordinary
                })
            },
        ),
    })))
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::{QvmImage, QvmInstruction, QvmModule};

    use super::*;
    use crate::q3::test_support::fixture_artifact;

    fn instruction(opcode: QvmOpcode, operand: i32) -> QvmInstruction {
        QvmInstruction::word(opcode, operand, 0)
    }

    fn stock_artifact() -> QvmArtifact {
        let mut artifact = fixture_artifact(STOCK_DIGEST, QvmRole::Qagame, 64, &[]);
        let mut instructions = vec![QvmInstruction::single(QvmOpcode::OpIgnore, 0); 103218];
        instructions[103202] = instruction(QvmOpcode::OpConst, 200);
        instructions[103203] = instruction(QvmOpcode::OpLei, 0);
        instructions[103216] = instruction(QvmOpcode::OpConst, 200);
        instructions[103217] = instruction(QvmOpcode::OpStore4, 0);
        artifact.image.instructions = instructions;
        artifact
    }

    fn threewave_artifact() -> QvmArtifact {
        let mut artifact = fixture_artifact(THREEWAVE_GRAPPLE_DIGEST, QvmRole::Qagame, 1_091_864, &[]);
        let mut instructions = vec![QvmInstruction::single(QvmOpcode::OpIgnore, 0); 167030];
        let set = |instructions: &mut Vec<QvmInstruction>, at: usize, opcode: QvmOpcode, operand: i32| {
            instructions[at] = instruction(opcode, operand);
        };
        for (at, value) in [
            (166498, 2),
            (166507, 2),
            (166514, 4096),
            (166769, 8192),
            (166771, 10),
            (166773, 8196),
        ] {
            set(&mut instructions, at, QvmOpcode::OpConst, value);
        }
        for (comparison, store, value) in [(166830, 166844, 200), (166753, 166764, 100), (167010, 167021, 150)] {
            set(&mut instructions, comparison, QvmOpcode::OpConst, value);
            set(&mut instructions, comparison + 1, QvmOpcode::OpLei, 0);
            set(&mut instructions, store, QvmOpcode::OpConst, value);
            set(&mut instructions, store + 1, QvmOpcode::OpStore4, 0);
        }
        artifact.image.instructions = instructions;
        let mut data = vec![0u8; 8200];
        data[4096 + 2 * 4..4096 + 2 * 4 + 4].copy_from_slice(&167000i32.to_le_bytes());
        artifact.image.initialized_data = data;
        artifact
    }

    fn capacity_of(profile: &QvmInventoryProfile, memory: &QvmSharedMemory, weapon: i32) -> Result<i32, GuestError> {
        let QvmInventoryProfile::Public(profile) = profile else {
            panic!("Q3 inventory profiles are public");
        };
        let module = QvmModule::new(
            QvmArtifact {
                module: profile.module.clone(),
                role: QvmRole::Qagame,
                abi_profile: Some(AbiProfile::Modern),
                image: QvmImage::default(),
            },
            None,
            None,
        )?;
        (profile.capacity)(
            memory,
            weapon,
            &QvmInventoryCapacityContext {
                module,
                client: 0,
                entity: 0,
                client_number: 0,
            },
        )
    }

    #[test]
    fn stock_capacity_reads_its_pinned_constant() {
        let profile = q3_native_inventory_profile(&stock_artifact()).unwrap().unwrap();
        let memory = QvmSharedMemory::new(64).unwrap();
        assert_eq!(capacity_of(&profile, &memory, 2).unwrap(), 200);
    }

    #[test]
    fn threewave_capacity_selects_special_limits() {
        let profile = q3_native_inventory_profile(&threewave_artifact()).unwrap().unwrap();
        let memory = QvmSharedMemory::new(8200).unwrap();
        memory.write_i32(8192, 0).unwrap();
        memory.write_i32(8196, 0).unwrap();
        assert_eq!(capacity_of(&profile, &memory, 2).unwrap(), 200);
        memory.write_i32(8192, 10).unwrap();
        assert_eq!(capacity_of(&profile, &memory, 2).unwrap(), 150);
        assert_eq!(capacity_of(&profile, &memory, 9).unwrap(), 100);
        memory.write_i32(8192, 0).unwrap();
        memory.write_i32(8196, 1).unwrap();
        assert_eq!(capacity_of(&profile, &memory, 2).unwrap(), 150);
    }

    #[test]
    fn rejects_unqualified_and_corrupt_images() {
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        assert!(q3_native_inventory_profile(&other).unwrap().is_none());
        let mut cgame = stock_artifact();
        cgame.role = QvmRole::Cgame;
        assert!(q3_native_inventory_profile(&cgame).is_err());
        let mut legacy = stock_artifact();
        legacy.abi_profile = Some(AbiProfile::Legacy);
        assert!(q3_native_inventory_profile(&legacy).is_err());
        let mut corrupt = stock_artifact();
        corrupt.image.instructions[103216] = instruction(QvmOpcode::OpConst, 199);
        assert!(q3_native_inventory_profile(&corrupt).is_err());
    }
}
