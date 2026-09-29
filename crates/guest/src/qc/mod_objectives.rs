//! QuakeC objective storage selectors.
//!
//! Ported from `src/compat/qc/mod-objectives.ts`. The storage selector and
//! program-view mirrors (`QcModObjectiveStorage`, `QcProgramView`,
//! `QcValueType`) live in `super::mod_provider`, absorbed from
//! `src/contracts/mod-callbacks.ts` and `src/compat/qc/program.ts`.
//!
//! Local mirrors: `QcObjectiveMachine` mirrors the `QcMachine` surface
//! (`globals`, `entities`, `globalOffset`, `fieldOffset`) from
//! `src/compat/qc/machine.ts` that objective storage touches.

use super::mod_provider::{QcModObjectiveStorage, QcProgramView, QcValueType};
use crate::error::GuestError;

/// Machine surface for objective storage resolution.
pub trait QcObjectiveMachine {
    /// Read a global integer.
    fn global_int(&self, name: &str) -> Result<i32, GuestError>;
    /// Read an entity integer by reference.
    fn entity_int(&self, reference: i32, field: &str) -> Result<i32, GuestError>;
}

/// Validate one objective storage selector against its program.
pub fn validate_qc_objective_storage(
    program: &dyn QcProgramView,
    storage: &QcModObjectiveStorage,
    expected: QcValueType,
) -> Result<(), GuestError> {
    match storage {
        QcModObjectiveStorage::Global(name) => {
            if program.global_type(name) != Some(expected) {
                return Err(GuestError::invalid(format!(
                    "Objective global {name} requires original {expected:?} storage"
                )));
            }
        }
        QcModObjectiveStorage::EntityField {
            global,
            indirections,
            field,
        } => {
            if program.global_type(global) != Some(QcValueType::Entity) {
                return Err(GuestError::invalid(
                    "Objective field requires an original global entity reference",
                ));
            }
            for name in indirections {
                if program.field_type(name) != Some(QcValueType::Entity) {
                    return Err(GuestError::invalid(format!(
                        "Objective selector {name} requires an original entity field"
                    )));
                }
            }
            if program.field_type(field) != Some(expected) {
                return Err(GuestError::invalid(format!(
                    "Objective field {field} requires original {expected:?} storage"
                )));
            }
        }
    }
    Ok(())
}

/// Resolved objective storage: words identity plus offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcObjectiveLocation {
    /// Entity reference when the leaf lives on an entity.
    pub reference: Option<i32>,
    /// Whether the leaf lives on globals.
    pub on_globals: bool,
    /// Leaf field or global name.
    pub name: String,
}

/// Resolve an objective storage selector, checking currency along the path.
pub fn resolve_qc_objective_storage(
    machine: &dyn QcObjectiveMachine,
    storage: &QcModObjectiveStorage,
    current: &dyn Fn(Option<i32>) -> Result<(), GuestError>,
) -> Result<QcObjectiveLocation, GuestError> {
    current(None)?;
    match storage {
        QcModObjectiveStorage::Global(name) => Ok(QcObjectiveLocation {
            reference: None,
            on_globals: true,
            name: name.clone(),
        }),
        QcModObjectiveStorage::EntityField {
            global,
            indirections,
            field,
        } => {
            let mut reference = machine.global_int(global)?;
            for name in indirections {
                current(Some(reference))?;
                reference = machine.entity_int(reference, name)?;
            }
            current(Some(reference))?;
            Ok(QcObjectiveLocation {
                reference: Some(reference),
                on_globals: false,
                name: field.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use super::super::mod_provider::{QcApiKind, QcFunctionView};

    struct FakeProgram {
        fields: HashMap<String, QcValueType>,
        globals: HashMap<String, QcValueType>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            "abc"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.fields.get(name).copied()
        }

        fn global_type(&self, name: &str) -> Option<QcValueType> {
            self.globals.get(name).copied()
        }

        fn function_named(&self, _name: &str) -> Option<QcFunctionView> {
            None
        }

        fn function_at(&self, _index: i32) -> Option<QcFunctionView> {
            None
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            Vec::new()
        }
    }

    struct FakeMachine {
        global: i32,
        next: HashMap<(i32, String), i32>,
    }

    impl QcObjectiveMachine for FakeMachine {
        fn global_int(&self, _name: &str) -> Result<i32, GuestError> {
            Ok(self.global)
        }

        fn entity_int(&self, reference: i32, field: &str) -> Result<i32, GuestError> {
            self.next
                .get(&(reference, field.to_string()))
                .copied()
                .ok_or_else(|| GuestError::invalid("No such entity field"))
        }
    }

    fn program() -> FakeProgram {
        FakeProgram {
            fields: [
                ("state".to_string(), QcValueType::Float),
                ("owner".to_string(), QcValueType::Entity),
            ]
            .into_iter()
            .collect(),
            globals: [
                ("quest".to_string(), QcValueType::Float),
                ("hero".to_string(), QcValueType::Entity),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn validates_global_and_entity_storage() {
        let program = program();
        assert!(validate_qc_objective_storage(
            &program,
            &QcModObjectiveStorage::Global("quest".to_string()),
            QcValueType::Float
        )
        .is_ok());
        assert!(validate_qc_objective_storage(
            &program,
            &QcModObjectiveStorage::Global("quest".to_string()),
            QcValueType::Entity
        )
        .is_err());
        let entity = QcModObjectiveStorage::EntityField {
            global: "hero".to_string(),
            indirections: vec!["owner".to_string()],
            field: "state".to_string(),
        };
        assert!(validate_qc_objective_storage(&program, &entity, QcValueType::Float).is_ok());
        let bad = QcModObjectiveStorage::EntityField {
            global: "quest".to_string(),
            indirections: vec![],
            field: "state".to_string(),
        };
        assert!(validate_qc_objective_storage(&program, &bad, QcValueType::Float).is_err());
    }

    #[test]
    fn resolves_entity_path_with_currency_checks() {
        let machine = FakeMachine {
            global: 5,
            next: [((5, "owner".to_string()), 9)].into_iter().collect(),
        };
        let storage = QcModObjectiveStorage::EntityField {
            global: "hero".to_string(),
            indirections: vec!["owner".to_string()],
            field: "state".to_string(),
        };
        let mut seen = Vec::new();
        let location = resolve_qc_objective_storage(&machine, &storage, &|reference| {
            seen.push(reference);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            location,
            QcObjectiveLocation {
                reference: Some(9),
                on_globals: false,
                name: "state".to_string()
            }
        );
        assert_eq!(seen, vec![None, Some(5), Some(9)]);
    }

    #[test]
    fn stale_currency_aborts_resolution() {
        let machine = FakeMachine {
            global: 5,
            next: HashMap::new(),
        };
        let storage = QcModObjectiveStorage::Global("quest".to_string());
        let error =
            resolve_qc_objective_storage(&machine, &storage, &|_| Err(GuestError::invalid("retired"))).unwrap_err();
        assert_eq!(error, GuestError::invalid("retired"));
    }
}
