//! Port of `src/compat/qc/save.ts` (Quake `pr_edict.c` entity/global save
//! fields, GPL-2.0-or-later): text parsing and saving of entity fields and
//! savable globals.
//!
//! Numbers use the shared [`qa_core::cvar`] spellings (`quake_atof` for
//! parsing, `cvar_value_text` for saving), matching the donor exactly.

use super::machine::QcMachine;
use super::program::{QcDefinition, QcValueType};
use crate::error::GuestError;

/// One text key/value pair in source order (duplicates preserved by the
/// caller-supplied slice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcTextPair {
    /// Key text.
    pub key: String,
    /// Value text.
    pub value: String,
}

/// Result of applying entity pairs to a slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcEntityParseResult {
    /// Whether the pair list was empty.
    pub empty: bool,
    /// Pairs with no matching field definition.
    pub unknown: Vec<QcTextPair>,
    /// Pairs retained after comment/alias filtering.
    pub retained: Vec<QcTextPair>,
}

/// Parse target for [`parse_qc_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcSaveTarget {
    /// Global words.
    Globals,
    /// Entity slot variables.
    Entity(u32),
}

fn write_target(machine: &mut QcMachine, target: QcSaveTarget, offset: usize, value: i32) -> Result<(), GuestError> {
    match target {
        QcSaveTarget::Globals => machine
            .globals_mut()
            .set_int(offset, value)
            .map_err(|error| machine.fail(error.to_string())),
        QcSaveTarget::Entity(slot) => machine
            .entities_mut()
            .set_slot_int(slot, offset, value)
            .map_err(|error| machine.fail(error.to_string())),
    }
}

/// Unescape `\\n` newlines; any other backslash collapses to one.
fn new_string(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        if char == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some(_) => result.push('\\'),
                None => result.push('\\'),
            }
        } else {
            result.push(char);
        }
    }
    result
}

/// Parse one text value into global or entity words.
pub fn parse_qc_value(
    machine: &mut QcMachine,
    target: QcSaveTarget,
    definition: &QcDefinition,
    text: &str,
) -> Result<(), GuestError> {
    let offset = definition.offset;
    match definition.value_type {
        QcValueType::String => {
            let stored = machine
                .strings_mut()
                .allocate(&new_string(text))
                .map_err(|error| machine.fail(error.to_string()))?;
            write_target(machine, target, offset, stored)
        }
        QcValueType::Float => {
            let value = qa_core::cvar::quake_atof(text) as f32;
            match target {
                QcSaveTarget::Globals => machine
                    .globals_mut()
                    .set_float(offset, value)
                    .map_err(|error| machine.fail(error.to_string())),
                QcSaveTarget::Entity(slot) => machine
                    .entities_mut()
                    .set_slot_float(slot, offset, value)
                    .map_err(|error| machine.fail(error.to_string())),
            }
        }
        QcValueType::Vector => {
            let mut start = 0;
            for component in 0..3 {
                let from = start.min(text.len());
                let end = text[from..].find(' ').map(|at| from + at).unwrap_or(text.len());
                let value = qa_core::cvar::quake_atof(&text[from..end]) as f32;
                match target {
                    QcSaveTarget::Globals => machine
                        .globals_mut()
                        .set_float(offset + component, value)
                        .map_err(|error| machine.fail(error.to_string()))?,
                    QcSaveTarget::Entity(slot) => machine
                        .entities_mut()
                        .set_slot_float(slot, offset + component, value)
                        .map_err(|error| machine.fail(error.to_string()))?,
                }
                start = end + 1;
            }
            Ok(())
        }
        QcValueType::Entity => {
            let slot = qa_core::cvar::quake_atof(text).trunc() as i64;
            let slot = u32::try_from(slot).map_err(|_| machine.fail(format!("invalid entity slot {text}")))?;
            let reference = machine
                .entities()
                .reference(slot)
                .map_err(|error| machine.fail(error.to_string()))?;
            write_target(machine, target, offset, reference)
        }
        QcValueType::Function => {
            let index = machine
                .program()
                .function_named(text)
                .map_err(|error| machine.fail(error.to_string()))?
                .index;
            write_target(machine, target, offset, index as i32)
        }
        QcValueType::Field => {
            // The original `ED_ParseEpair` reads the global word at this
            // field definition's offset.
            let field = machine
                .program()
                .field_named(text)
                .ok_or_else(|| machine.fail(format!("cannot find field {text}")))?
                .offset;
            let value = machine
                .globals()
                .int(field)
                .map_err(|error| machine.fail(error.to_string()))?;
            write_target(machine, target, offset, value)
        }
        QcValueType::Void => Ok(()),
        QcValueType::Pointer | QcValueType::Opaque => Err(machine.fail(format!(
            "text parsing is unavailable for QC type {} ({}); restore raw state instead",
            definition.native_type, definition.name
        ))),
    }
}

/// Apply source-order pairs to an entity slot (slot 0 keeps its words;
/// other slots are zeroed first). Handles `angle`/`light` aliases,
/// NetQuake trailing-space keys, and `_` comment keys.
pub fn apply_qc_entity_pairs(
    machine: &mut QcMachine,
    slot: u32,
    pairs: &[QcTextPair],
) -> Result<QcEntityParseResult, GuestError> {
    if slot != 0 {
        machine
            .entities_mut()
            .clear_slot(slot)
            .map_err(|error| machine.fail(error.to_string()))?;
    } else {
        machine
            .entities()
            .field_bytes(slot)
            .map_err(|error| machine.fail(error.to_string()))?;
    }
    let netquake = !machine.program().api.is_quakeworld();
    let mut unknown = Vec::new();
    let mut retained = Vec::new();
    for pair in pairs {
        let angle = pair.key == "angle";
        let mut key = if angle {
            "angles".to_string()
        } else if pair.key == "light" {
            "light_lev".to_string()
        } else {
            pair.key.clone()
        };
        if netquake {
            while key.ends_with(' ') {
                key.pop();
            }
        }
        if key.starts_with('_') {
            continue;
        }
        retained.push(QcTextPair {
            key: key.clone(),
            value: pair.value.clone(),
        });
        let definition = machine.program().field_named(&key).cloned();
        match definition {
            Some(definition) => {
                let text = if angle {
                    format!("0 {} 0", pair.value)
                } else {
                    pair.value.clone()
                };
                parse_qc_value(machine, QcSaveTarget::Entity(slot), &definition, &text)?;
            }
            None => unknown.push(QcTextPair {
                key,
                value: pair.value.clone(),
            }),
        }
    }
    Ok(QcEntityParseResult {
        empty: pairs.is_empty(),
        unknown,
        retained,
    })
}

/// Apply pairs to globals, returning the unknown ones.
pub fn apply_qc_global_pairs(machine: &mut QcMachine, pairs: &[QcTextPair]) -> Result<Vec<QcTextPair>, GuestError> {
    let mut unknown = Vec::new();
    for pair in pairs {
        let definition = machine.program().global_named(&pair.key).cloned();
        match definition {
            Some(definition) => parse_qc_value(machine, QcSaveTarget::Globals, &definition, &pair.value)?,
            None => unknown.push(pair.clone()),
        }
    }
    Ok(unknown)
}

fn saved_word(machine: &QcMachine, target: QcSaveTarget, offset: usize) -> Result<i32, GuestError> {
    match target {
        QcSaveTarget::Globals => machine
            .globals()
            .int(offset)
            .map_err(|error| machine.fail(error.to_string())),
        QcSaveTarget::Entity(slot) => machine
            .entities()
            .slot_int(slot, offset)
            .map_err(|error| machine.fail(error.to_string())),
    }
}

fn saved_float(machine: &QcMachine, target: QcSaveTarget, offset: usize) -> Result<f32, GuestError> {
    match target {
        QcSaveTarget::Globals => machine
            .globals()
            .float(offset)
            .map_err(|error| machine.fail(error.to_string())),
        QcSaveTarget::Entity(slot) => machine
            .entities()
            .slot_float(slot, offset)
            .map_err(|error| machine.fail(error.to_string())),
    }
}

fn saved_value(machine: &QcMachine, target: QcSaveTarget, definition: &QcDefinition) -> Result<String, GuestError> {
    let offset = definition.offset;
    match definition.value_type {
        QcValueType::String => {
            let reference = saved_word(machine, target, offset)?;
            machine
                .strings()
                .get(reference)
                .map_err(|error| machine.fail(error.to_string()))
        }
        QcValueType::Float => {
            let value = saved_float(machine, target, offset)?;
            qa_core::cvar::cvar_value_text(f64::from(value), false).map_err(|error| machine.fail(error.to_string()))
        }
        QcValueType::Vector => {
            let mut parts = Vec::with_capacity(3);
            for component in 0..3 {
                let value = saved_float(machine, target, offset + component)?;
                parts.push(
                    qa_core::cvar::cvar_value_text(f64::from(value), false)
                        .map_err(|error| machine.fail(error.to_string()))?,
                );
            }
            Ok(parts.join(" "))
        }
        QcValueType::Entity => {
            let reference = saved_word(machine, target, offset)?;
            let slot = machine
                .entities()
                .slot(reference)
                .map_err(|error| machine.fail(error.to_string()))?;
            Ok(slot.to_string())
        }
        QcValueType::Function => {
            let index = saved_word(machine, target, offset)?;
            let function = machine
                .program()
                .function_at(index as usize)
                .map_err(|error| machine.fail(error.to_string()))?;
            Ok(function.name.clone())
        }
        QcValueType::Field => {
            let word = saved_word(machine, target, offset)?;
            let field = machine
                .program()
                .fields
                .iter()
                .find(|candidate| candidate.offset as i32 == word)
                .ok_or_else(|| machine.fail(format!("cannot save field offset {word}")))?;
            Ok(field.name.clone())
        }
        QcValueType::Void => Ok("void".to_string()),
        QcValueType::Pointer | QcValueType::Opaque => Err(machine.fail(format!(
            "text saving is unavailable for QC type {} ({}); save raw state instead",
            definition.native_type, definition.name
        ))),
    }
}

/// Save savable string/float/entity globals as text pairs.
pub fn save_qc_global_pairs(machine: &QcMachine) -> Result<Vec<QcTextPair>, GuestError> {
    let mut pairs = Vec::new();
    for definition in machine.program().globals.clone() {
        if definition.save
            && matches!(
                definition.value_type,
                QcValueType::String | QcValueType::Float | QcValueType::Entity
            )
        {
            pairs.push(QcTextPair {
                key: definition.name.clone(),
                value: saved_value(machine, QcSaveTarget::Globals, &definition)?,
            });
        }
    }
    Ok(pairs)
}

/// Save nonzero entity fields as text pairs (free slots save nothing).
/// Fields after the first whose second-to-last name character is `_` are
/// compiler temporaries and are skipped, like the donor.
pub fn save_qc_entity_pairs(machine: &QcMachine, slot: u32, free: bool) -> Result<Vec<QcTextPair>, GuestError> {
    if free {
        return Ok(Vec::new());
    }
    machine
        .entities()
        .field_bytes(slot)
        .map_err(|error| machine.fail(error.to_string()))?;
    let mut pairs = Vec::new();
    for definition in machine.program().fields.clone().into_iter().skip(1) {
        let name = definition.name.as_bytes();
        if name.len() >= 2 && name[name.len() - 2] == b'_' {
            continue;
        }
        let count = definition.value_type.words();
        let mut nonzero = false;
        for component in 0..count {
            if saved_word(machine, QcSaveTarget::Entity(slot), definition.offset + component)? != 0 {
                nonzero = true;
            }
        }
        if nonzero {
            pairs.push(QcTextPair {
                key: definition.name.clone(),
                value: saved_value(machine, QcSaveTarget::Entity(slot), &definition)?,
            });
        }
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::super::machine::{QcBuiltinRegistry, QcMachineOptions};
    use super::super::memory::{QcEntityLayout, QcEntityMemory};
    use super::super::program::{test_program, QcFunction, QcOpcode, QcStatement, QcValueType};
    use super::*;
    use qa_core::numeric::NumericOps;
    use std::rc::Rc;

    fn fixture_machine() -> QcMachine {
        let statements = vec![QcStatement {
            opcode: QcOpcode::Done,
            a: 0,
            b: 0,
            c: 0,
        }];
        let globals = vec![
            QcDefinition {
                value_type: QcValueType::Float,
                native_type: 2,
                save: true,
                offset: 28,
                name: "skill".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::String,
                native_type: 1,
                save: true,
                offset: 29,
                name: "mapname".to_string(),
            },
        ];
        let fields = vec![
            QcDefinition {
                value_type: QcValueType::Void,
                native_type: 0,
                save: false,
                offset: 0,
                name: "<reserved>".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::Vector,
                native_type: 3,
                save: false,
                offset: 0,
                name: "angles".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::String,
                native_type: 1,
                save: false,
                offset: 3,
                name: "classname".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::Float,
                native_type: 2,
                save: false,
                offset: 4,
                name: "health".to_string(),
            },
            QcDefinition {
                value_type: QcValueType::Entity,
                native_type: 4,
                save: false,
                offset: 5,
                name: "enemy".to_string(),
            },
        ];
        let functions = vec![QcFunction {
            index: 0,
            first_statement: 0,
            parameter_start: 0,
            local_words: 0,
            name: "<null>".to_string(),
            file: String::new(),
            parameter_sizes: vec![],
            named_builtin: false,
        }];
        let program = test_program(
            statements,
            globals,
            fields,
            functions,
            b"\0".to_vec(),
            vec![0; 30 * 4],
            6,
        );
        let layout = QcEntityLayout {
            stride_bytes: 96 + 6 * 4,
            variables_offset_bytes: 96,
            field_words: 6,
        };
        let entities = QcEntityMemory::new(layout, 3, 2).unwrap();
        QcMachine::new(QcMachineOptions::new(
            program,
            NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap(),
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        ))
        .unwrap()
    }

    fn pair(key: &str, value: &str) -> QcTextPair {
        QcTextPair {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn entity_pairs_parse_and_alias() {
        let mut machine = fixture_machine();
        let result = apply_qc_entity_pairs(
            &mut machine,
            1,
            &[
                pair("angle", "90"),
                pair("classname", "monster"),
                pair("_comment", "x"),
                pair("nope", "1"),
            ],
        )
        .unwrap();
        assert!(!result.empty);
        assert_eq!(result.unknown, vec![pair("nope", "1")]);
        assert_eq!(
            machine
                .entity_vector(machine.entities().reference(1).unwrap(), "angles")
                .unwrap(),
            qa_core::math::vec3(0.0, 90.0, 0.0)
        );
        let reference = machine.entities().reference(1).unwrap();
        let classname = machine.entity_int(reference, "classname").unwrap();
        assert_eq!(machine.strings().get(classname).unwrap(), "monster");
    }

    #[test]
    fn entity_newlines_unescape() {
        let mut machine = fixture_machine();
        apply_qc_entity_pairs(&mut machine, 1, &[pair("classname", "a\\nb\\\\c")]).unwrap();
        let reference = machine.entities().reference(1).unwrap();
        let classname = machine.entity_int(reference, "classname").unwrap();
        assert_eq!(machine.strings().get(classname).unwrap(), "a\nb\\c");
    }

    #[test]
    fn global_pairs_round_trip() {
        let mut machine = fixture_machine();
        let unknown = apply_qc_global_pairs(&mut machine, &[pair("skill", "3"), pair("bogus", "0")]).unwrap();
        assert_eq!(unknown, vec![pair("bogus", "0")]);
        assert_eq!(machine.globals().float(28).unwrap(), 3.0);
        let saved = save_qc_global_pairs(&machine).unwrap();
        assert!(saved.iter().any(|pair| pair.key == "skill" && pair.value == "3.000000"));
    }

    #[test]
    fn entity_save_skips_zero_and_free() {
        let mut machine = fixture_machine();
        apply_qc_entity_pairs(&mut machine, 1, &[pair("health", "100")]).unwrap();
        let saved = save_qc_entity_pairs(&machine, 1, false).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].key, "health");
        assert!(save_qc_entity_pairs(&machine, 1, true).unwrap().is_empty());
    }

    #[test]
    fn opaque_types_fail_with_raw_state_hint() {
        let mut machine = fixture_machine();
        let definition = QcDefinition {
            value_type: QcValueType::Opaque,
            native_type: 9,
            save: false,
            offset: 4,
            name: "blob".to_string(),
        };
        let error = parse_qc_value(&mut machine, QcSaveTarget::Globals, &definition, "1").unwrap_err();
        assert!(error.to_string().contains("restore raw state"));
    }
}
