//! QVM mod objective addresses: direct words and global-pointer selectors.
//!
//! Ports `src/compat/qvm/mod-objectives.ts`. The address type comes from
//! [`super::mod_actors`].

use super::game_data::QvmSharedMemory;
use super::mod_actors::{QvmModInputPointerBase, QvmModObjectiveAddress};
use crate::error::GuestError;

/// Check an aligned word against the data extent.
fn check_word(end: usize, address: i64) -> Result<usize, GuestError> {
    if address < 0 || address % 4 != 0 || address as usize + 4 > end {
        return Err(GuestError::invalid(
            "Objective address exceeds original QVM data or is not aligned",
        ));
    }
    Ok(address as usize)
}

/// Validate an objective address against the data extent.
pub fn validate_qvm_objective_address(end: usize, address: &QvmModObjectiveAddress) -> Result<(), GuestError> {
    match address {
        QvmModObjectiveAddress::Direct(address) => {
            check_word(end, *address as i64)?;
            Ok(())
        }
        QvmModObjectiveAddress::Indirect(pointer) => {
            let QvmModInputPointerBase::Global { address } = pointer.base else {
                return Err(GuestError::invalid(
                    "Objective selector requires a global source pointer",
                ));
            };
            check_word(end, address as i64)?;
            check_word(end, pointer.offset as i64)?;
            for offset in &pointer.indirections {
                check_word(end, *offset as i64)?;
            }
            Ok(())
        }
    }
}

/// Resolve an objective address, refreshing currency before each read.
pub fn resolve_qvm_objective_address(
    memory: &QvmSharedMemory,
    end: usize,
    address: &QvmModObjectiveAddress,
    current: &dyn Fn(),
) -> Result<usize, GuestError> {
    current();
    match address {
        QvmModObjectiveAddress::Direct(address) => check_word(end, *address as i64),
        QvmModObjectiveAddress::Indirect(pointer) => {
            let QvmModInputPointerBase::Global { address } = pointer.base else {
                return Err(GuestError::invalid(
                    "Objective selector requires a global source pointer",
                ));
            };
            let mut selected = memory.read_i32(check_word(end, address as i64)?)?;
            for offset in &pointer.indirections {
                current();
                if selected == 0 {
                    return Err(GuestError::invalid("Objective selector follows a null source pointer"));
                }
                let base = check_word(end, i64::from(selected))?;
                selected = memory.read_i32(check_word(end, base as i64 + *offset as i64)?)?;
            }
            current();
            if selected == 0 {
                return Err(GuestError::invalid("Objective selector follows a null source pointer"));
            }
            let base = check_word(end, i64::from(selected))?;
            check_word(end, base as i64 + pointer.offset as i64)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::super::mod_actors::QvmModInputPointer;
    use super::*;

    #[test]
    fn validates_direct_words() {
        assert!(validate_qvm_objective_address(1024, &QvmModObjectiveAddress::Direct(64)).is_ok());
        assert!(validate_qvm_objective_address(1024, &QvmModObjectiveAddress::Direct(62)).is_err());
        assert!(validate_qvm_objective_address(64, &QvmModObjectiveAddress::Direct(64)).is_err());
    }

    #[test]
    fn validates_selectors() {
        let selector = QvmModObjectiveAddress::Indirect(QvmModInputPointer {
            base: QvmModInputPointerBase::Global { address: 100 },
            indirections: vec![8],
            offset: 4,
        });
        assert!(validate_qvm_objective_address(1024, &selector).is_ok());
        let argument = QvmModObjectiveAddress::Indirect(QvmModInputPointer {
            base: QvmModInputPointerBase::Argument { index: 0 },
            indirections: Vec::new(),
            offset: 0,
        });
        assert!(validate_qvm_objective_address(1024, &argument).is_err());
        let skewed = QvmModObjectiveAddress::Indirect(QvmModInputPointer {
            base: QvmModInputPointerBase::Global { address: 100 },
            indirections: vec![7],
            offset: 4,
        });
        assert!(validate_qvm_objective_address(1024, &skewed).is_err());
    }

    #[test]
    fn resolves_chains_and_rejects_nulls() {
        let memory = QvmSharedMemory::new(1024).unwrap();
        memory.write_i32(100, 200).unwrap();
        memory.write_i32(208, 300).unwrap();
        let selector = QvmModObjectiveAddress::Indirect(QvmModInputPointer {
            base: QvmModInputPointerBase::Global { address: 100 },
            indirections: vec![8],
            offset: 4,
        });
        let calls = Rc::new(Cell::new(0));
        let tick = Rc::clone(&calls);
        let address = resolve_qvm_objective_address(&memory, 1024, &selector, &|| {
            tick.set(tick.get() + 1);
        })
        .unwrap();
        assert_eq!(address, 304);
        assert_eq!(calls.get(), 3);

        assert_eq!(
            resolve_qvm_objective_address(&memory, 1024, &QvmModObjectiveAddress::Direct(64), &|| {}).unwrap(),
            64
        );

        memory.write_i32(208, 0).unwrap();
        assert!(resolve_qvm_objective_address(&memory, 1024, &selector, &|| {}).is_err());
        memory.write_i32(100, 0).unwrap();
        assert!(resolve_qvm_objective_address(&memory, 1024, &selector, &|| {}).is_err());
    }
}
