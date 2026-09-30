//! Qualified combat damage calls (`src/content/q1/quakec/damage-call.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/damage-call.ts`
//! (`qcDamageCallLayout`, `standardQcDamageCallLayout`,
//! `readQcDamageCall`, `projectQcDamageCall`).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::contract::{ModCallbackInput, ModCallbackValue, ModSourceCall};

use super::qc_view::{
    qc_source_value_type, validate_qc_source_call, QcFunctionExecution, QcMachineView, QcProgramView, QcValueType,
};
use super::{float_identical, fround, QcError};

/// Semantic damage role (donor `QcDamageRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QcDamageRole {
    /// Damage target.
    Slf,
    /// Inflicting entity.
    Inflictor,
    /// Attacking actor.
    Attacker,
    /// Damage amount.
    Amount,
}

/// Role source location (donor `QcDamageRoleLocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcDamageRoleLocation {
    /// Call argument.
    Argument {
        /// Argument index.
        index: usize,
        /// Callee frame word.
        frame_word: usize,
    },
    /// Source global.
    Global {
        /// Global word.
        word: usize,
    },
}

/// Damage call layout (donor `QcDamageCallLayout`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcDamageCallLayout {
    /// Called function index.
    pub function_index: usize,
    /// Parameter types.
    pub parameters: Vec<QcValueType>,
    /// Locations per role.
    pub roles: BTreeMap<QcDamageRole, Vec<QcDamageRoleLocation>>,
    /// Declaring source call.
    pub declaration: Option<ModSourceCall>,
}

/// Captured damage call values (donor `QcDamageCallValues`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QcDamageCallValues {
    /// Target reference.
    pub self_reference: i32,
    /// Inflictor reference.
    pub inflictor: i32,
    /// Attacker reference.
    pub attacker: i32,
    /// Damage amount.
    pub amount: f64,
}

/// Roles in donor order.
const ROLES: [QcDamageRole; 4] = [
    QcDamageRole::Slf,
    QcDamageRole::Inflictor,
    QcDamageRole::Attacker,
    QcDamageRole::Amount,
];

/// Donor spelling of a role for diagnostics.
fn role_name(role: QcDamageRole) -> &'static str {
    match role {
        QcDamageRole::Slf => "self",
        QcDamageRole::Inflictor => "inflictor",
        QcDamageRole::Attacker => "attacker",
        QcDamageRole::Amount => "amount",
    }
}

/// An original source call supplies semantic roles; their positions are
/// not a game-wide ABI (donor `qcDamageCallLayout`).
pub fn qc_damage_call_layout(
    program: &QcProgramView,
    declaration: Option<&ModSourceCall>,
) -> Result<QcDamageCallLayout, QcError> {
    let reject =
        |reason: &str| -> QcError { QcError::program(format!("Unsupported QC damage call: {reason}"), program.source) };
    let function = program.function_named(
        declaration
            .map(|declaration| declaration.function.as_str())
            .unwrap_or("T_Damage"),
    )?;
    if function.first_statement <= 0 || function.named_builtin {
        return Err(reject("requires an original bytecode function"));
    }
    if let Some(declaration) = declaration {
        let inputs: HashSet<ModCallbackInput> = [
            ModCallbackInput::Slf,
            ModCallbackInput::Inflictor,
            ModCallbackInput::Attacker,
            ModCallbackInput::Amount,
            ModCallbackInput::Knockback,
            ModCallbackInput::Point,
            ModCallbackInput::Direction,
            ModCallbackInput::Normal,
            ModCallbackInput::Time,
        ]
        .into_iter()
        .collect();
        validate_qc_source_call(program, declaration, &inputs, "combat damage")?;
    }
    let mut parameters = Vec::new();
    let mut locations: BTreeMap<QcDamageRole, Vec<QcDamageRoleLocation>> = BTreeMap::new();
    for role in ROLES {
        locations.insert(role, Vec::new());
    }
    let mut word = function.parameter_start;
    for (index, size) in function.parameter_sizes.iter().enumerate() {
        let param_type = program
            .globals
            .iter()
            .find(|global| global.offset == word && global.def_type != QcValueType::Void)
            .map(|global| global.def_type);
        let Some(param_type) = param_type else {
            return Err(reject("untyped original parameter"));
        };
        if !matches!(
            param_type,
            QcValueType::Entity | QcValueType::Float | QcValueType::Vector | QcValueType::Str | QcValueType::Function
        ) {
            return Err(reject("untyped original parameter"));
        }
        if *size != usize::from(param_type == QcValueType::Vector) * 2 + 1 {
            return Err(reject("original parameter width"));
        }
        let value = declaration.and_then(|declaration| declaration.arguments.get(index));
        let role = match (declaration, value) {
            (None, _) => ROLES.get(index).copied(),
            (Some(_), Some(ModCallbackValue::Input(name))) => match name {
                ModCallbackInput::Slf => Some(QcDamageRole::Slf),
                ModCallbackInput::Inflictor => Some(QcDamageRole::Inflictor),
                ModCallbackInput::Attacker => Some(QcDamageRole::Attacker),
                ModCallbackInput::Amount => Some(QcDamageRole::Amount),
                _ => None,
            },
            _ => None,
        };
        if let Some(value) = value {
            if qc_source_value_type(value) != param_type {
                return Err(reject("declaration differs from original parameter types"));
            }
        }
        if let Some(role) = role {
            let expected = if role == QcDamageRole::Amount {
                QcValueType::Float
            } else {
                QcValueType::Entity
            };
            if param_type != expected {
                return Err(reject(&format!("invalid {} parameter type", role_name(role))));
            }
            if let Some(entries) = locations.get_mut(&role) {
                entries.push(QcDamageRoleLocation::Argument {
                    index,
                    frame_word: word,
                });
            }
        }
        parameters.push(param_type);
        word = word.saturating_add(*size);
    }
    if let Some(declaration) = declaration {
        for global in &declaration.globals {
            let Some(definition) = program.global_named(&global.name) else {
                return Err(reject("missing declared global"));
            };
            let width = usize::from(definition.def_type == QcValueType::Vector) * 2 + 1;
            if definition.offset < 28
                || definition.offset < function.parameter_start.saturating_add(function.local_words)
                    && definition.offset.saturating_add(width) > function.parameter_start
            {
                return Err(reject("declared global overlaps call staging or the callee frame"));
            }
            if let ModCallbackValue::Input(name) = &global.value {
                let role = match name {
                    ModCallbackInput::Slf => Some(QcDamageRole::Slf),
                    ModCallbackInput::Inflictor => Some(QcDamageRole::Inflictor),
                    ModCallbackInput::Attacker => Some(QcDamageRole::Attacker),
                    ModCallbackInput::Amount => Some(QcDamageRole::Amount),
                    _ => None,
                };
                if let Some(role) = role {
                    if let Some(entries) = locations.get_mut(&role) {
                        entries.push(QcDamageRoleLocation::Global {
                            word: definition.offset,
                        });
                    }
                }
            }
        }
    }
    let mut globals: HashMap<usize, QcDamageRole> = HashMap::new();
    for role in ROLES {
        let entries = locations.get(&role).cloned().unwrap_or_default();
        if entries.is_empty() {
            return Err(reject(&format!("missing {} input", role_name(role))));
        }
        for location in &entries {
            if let QcDamageRoleLocation::Global { word } = location {
                if let Some(previous) = globals.get(word) {
                    if *previous != role {
                        return Err(reject("distinct damage roles overlap one source global"));
                    }
                }
                globals.insert(*word, role);
            }
        }
    }
    Ok(QcDamageCallLayout {
        function_index: function.index,
        parameters,
        roles: locations,
        declaration: declaration.cloned(),
    })
}

/// Exact positional ABI of the two pinned original programs (donor
/// `standardQcDamageCallLayout`).
#[must_use]
pub fn standard_qc_damage_call_layout(function_index: usize, parameter_start: usize) -> QcDamageCallLayout {
    let argument = |index: usize| {
        vec![QcDamageRoleLocation::Argument {
            index,
            frame_word: parameter_start.saturating_add(index),
        }]
    };
    let mut roles = BTreeMap::new();
    roles.insert(QcDamageRole::Slf, argument(0));
    roles.insert(QcDamageRole::Inflictor, argument(1));
    roles.insert(QcDamageRole::Attacker, argument(2));
    roles.insert(QcDamageRole::Amount, argument(3));
    QcDamageCallLayout {
        function_index,
        parameters: vec![
            QcValueType::Entity,
            QcValueType::Entity,
            QcValueType::Entity,
            QcValueType::Float,
        ],
        roles,
        declaration: None,
    }
}

/// Staging word for a role location (donor `stagingWord`).
fn staging_word(location: &QcDamageRoleLocation) -> usize {
    match location {
        QcDamageRoleLocation::Argument { index, .. } => 4 + index * 3,
        QcDamageRoleLocation::Global { word } => *word,
    }
}

/// Read staged damage values, requiring aliases to agree (donor
/// `readQcDamageCall`).
pub fn read_qc_damage_call(
    machine: &dyn QcMachineView,
    layout: &QcDamageCallLayout,
) -> Result<QcDamageCallValues, QcError> {
    let read_reference = |role: QcDamageRole| -> Result<i32, QcError> {
        let mut captured: Option<i32> = None;
        for location in layout.roles.get(&role).cloned().unwrap_or_default() {
            let value = machine.global_int(staging_word(&location))?;
            if captured.is_some_and(|captured| captured != value) {
                return Err(QcError::program(
                    format!("Original QC {} aliases disagree", role_name(role)),
                    machine.program_source(),
                ));
            }
            captured = Some(value);
        }
        captured.ok_or_else(|| {
            QcError::program(
                format!("Missing qualified QC {}", role_name(role)),
                machine.program_source(),
            )
        })
    };
    let read_amount = || -> Result<f64, QcError> {
        let mut captured: Option<f64> = None;
        for location in layout.roles.get(&QcDamageRole::Amount).cloned().unwrap_or_default() {
            let value = machine.global_float(staging_word(&location))?;
            if captured.is_some_and(|captured| !float_identical(captured, value)) {
                return Err(QcError::program(
                    "Original QC amount aliases disagree",
                    machine.program_source(),
                ));
            }
            captured = Some(value);
        }
        captured.ok_or_else(|| QcError::program("Missing qualified QC amount", machine.program_source()))
    };
    Ok(QcDamageCallValues {
        self_reference: read_reference(QcDamageRole::Slf)?,
        inflictor: read_reference(QcDamageRole::Inflictor)?,
        attacker: read_reference(QcDamageRole::Attacker)?,
        amount: read_amount()?,
    })
}

/// Change only transformed semantic inputs; keep original extra
/// arguments and source context intact (donor `projectQcDamageCall`).
pub fn project_qc_damage_call(
    machine: &dyn QcMachineView,
    layout: &QcDamageCallLayout,
    captured: &QcDamageCallValues,
    effective: &QcDamageCallValues,
    execute: &dyn QcFunctionExecution,
) -> Result<(), QcError> {
    let mut saved: HashMap<usize, i32> = HashMap::new();
    let result = execute.run(Some(&mut |vm: &dyn QcMachineView| -> Result<(), QcError> {
        for role in ROLES {
            let locations = layout.roles.get(&role).cloned().unwrap_or_default();
            if role == QcDamageRole::Amount {
                let value = fround(effective.amount);
                for location in locations {
                    let word = staging_word(&location);
                    let current = match &location {
                        QcDamageRoleLocation::Argument { .. } => captured.amount,
                        QcDamageRoleLocation::Global { .. } => vm.global_float(word)?,
                    };
                    if float_identical(current, value) {
                        continue;
                    }
                    if matches!(location, QcDamageRoleLocation::Global { .. }) && !saved.contains_key(&word) {
                        saved.insert(word, vm.global_int(word)?);
                    }
                    vm.set_global_float(word, value)?;
                }
                continue;
            }
            let value = match role {
                QcDamageRole::Slf => effective.self_reference,
                QcDamageRole::Inflictor => effective.inflictor,
                QcDamageRole::Attacker => effective.attacker,
                QcDamageRole::Amount => continue,
            };
            let captured_value = match role {
                QcDamageRole::Slf => captured.self_reference,
                QcDamageRole::Inflictor => captured.inflictor,
                QcDamageRole::Attacker => captured.attacker,
                QcDamageRole::Amount => continue,
            };
            for location in locations {
                let word = staging_word(&location);
                let current = match &location {
                    QcDamageRoleLocation::Argument { .. } => captured_value,
                    QcDamageRoleLocation::Global { .. } => vm.global_int(word)?,
                };
                if current == value {
                    continue;
                }
                if matches!(location, QcDamageRoleLocation::Global { .. }) && !saved.contains_key(&word) {
                    saved.insert(word, vm.global_int(word)?);
                }
                vm.set_global_int(word, value)?;
            }
        }
        Ok(())
    }));
    let mut words: Vec<usize> = saved.keys().copied().collect();
    words.sort_unstable();
    for word in words {
        if let Some(value) = saved.get(&word) {
            machine.set_global_int(word, *value)?;
        }
    }
    result
}
