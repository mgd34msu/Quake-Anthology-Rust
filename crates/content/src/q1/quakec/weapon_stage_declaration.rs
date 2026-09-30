//! Declared weapon stages (`src/content/q1/quakec/weapon-stage-declaration.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/weapon-stage-declaration.ts`
//! (`readQcWeaponStageDeclaration`, `readQcPrimaryWeaponStage`).

use crate::contract::{ModSourceCall, QcStatement, QcWeaponStageDeclaration, QcWeaponStageRepeat};
use crate::value::SaveReader;

use super::QcError;

/// Client stage target (donor `string | ModSourceCall`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcClientStageTarget {
    /// Source function name.
    Function(String),
    /// Declared source call.
    Call(ModSourceCall),
}

/// Primary weapon objectives (donor `client.objectives`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcPrimaryObjectives {
    /// No objectives.
    None,
    /// Objective function name.
    Function(String),
    /// Declared objective call.
    Call(ModSourceCall),
}

/// Primary weapon client stages (donor `client`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPrimaryWeaponClient {
    /// Client spawn stage.
    pub spawn: QcClientStageTarget,
    /// Spawn selection stage.
    pub select_spawn: QcClientStageTarget,
    /// Objectives.
    pub objectives: QcPrimaryObjectives,
}

/// Primary weapon stage declaration (donor
/// `QcPrimaryWeaponStageDeclaration`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPrimaryWeaponStageDeclaration {
    /// Dispatcher function name.
    pub dispatcher: String,
    /// Continuation function names.
    pub continuations: Vec<String>,
    /// Repeat declarations.
    pub repeats: Vec<QcWeaponStageRepeat>,
    /// Client stages.
    pub client: QcPrimaryWeaponClient,
}

impl QcPrimaryWeaponStageDeclaration {
    /// Base stage declaration without the client stages.
    #[must_use]
    pub fn stage(&self) -> QcWeaponStageDeclaration {
        QcWeaponStageDeclaration {
            dispatcher: self.dispatcher.clone(),
            continuations: self.continuations.clone(),
            repeats: self.repeats.clone(),
        }
    }
}

/// Read a bounded integer field as `u32`.
fn u32_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<u32, QcError> {
    let field = reader.field(name);
    let value = field.integer(minimum)?;
    u32::try_from(value).map_err(|_| QcError::from(field.fail("expected an integer in range")))
}

/// Read a declared weapon stage (donor `readQcWeaponStageDeclaration`).
pub fn read_qc_weapon_stage_declaration(reader: SaveReader) -> Result<QcWeaponStageDeclaration, QcError> {
    Ok(QcWeaponStageDeclaration {
        dispatcher: reader.field("dispatcher").string()?,
        continuations: reader.field("continuations").list(|value| value.string())?,
        repeats: reader.field("repeats").list(|repeat| {
            let result = repeat.field("result");
            Ok::<_, QcError>(QcWeaponStageRepeat {
                function: repeat.field("function").string()?,
                entry: u32_field(&repeat, "entry", 0)?,
                exit: u32_field(&repeat, "exit", 0)?,
                result: crate::contract::QcWeaponStageResult {
                    word: u32_field(&result, "word", 28)?,
                    value: result.field("value").choice_i64(&[0, 1])? as u8,
                },
                statements: repeat.field("statements").list(|value| {
                    Ok::<_, QcError>(QcStatement {
                        opcode: u32_field(&value, "opcode", 0)?,
                        // The donor accepts signed operands but compares
                        // them against unsigned file words, so negative
                        // declarations fail qualification there; reject
                        // them at read time here instead.
                        a: u32_field(&value, "a", 0)?,
                        b: u32_field(&value, "b", 0)?,
                        c: u32_field(&value, "c", 0)?,
                    })
                })?,
            })
        })?,
    })
}

/// Read a spawn target (donor `spawn`).
fn read_spawn(reader: SaveReader) -> Result<QcClientStageTarget, QcError> {
    let scope = reader.string()?;
    if scope.is_empty() {
        Ok(QcClientStageTarget::Call(crate::mods::read_mod_source_call(reader)?))
    } else {
        Ok(QcClientStageTarget::Function(scope))
    }
}

/// Read a primary weapon stage declaration (donor
/// `readQcPrimaryWeaponStage`).
pub fn read_qc_primary_weapon_stage(reader: SaveReader) -> Result<QcPrimaryWeaponStageDeclaration, QcError> {
    let stage = read_qc_weapon_stage_declaration(reader.clone())?;
    let client = reader.field("client");
    let objectives = client.field("objectives");
    Ok(QcPrimaryWeaponStageDeclaration {
        dispatcher: stage.dispatcher,
        continuations: stage.continuations,
        repeats: stage.repeats,
        client: QcPrimaryWeaponClient {
            spawn: read_spawn(client.field("spawn"))?,
            select_spawn: read_spawn(client.field("selectSpawn"))?,
            objectives: if objectives.field("kind").choice_str(&["none", "call"])? == "none" {
                QcPrimaryObjectives::None
            } else if objectives.field("call").is_missing() {
                QcPrimaryObjectives::Function(objectives.field("function").string()?)
            } else {
                QcPrimaryObjectives::Call(crate::mods::read_mod_source_call(objectives.field("call"))?)
            },
        },
    })
}
