//! Port of `src/compat/qc/compatibility.ts`: the `quakec-compatibility.json`
//! document reader binding original source interfaces to program bytes.
//!
//! Also ports the declaration readers this document needs:
//! `content/q1/quakec/weapon-stage-declaration.ts`,
//! `content/q1/quakec/pickup-callers.ts` (`readQcPickupCaller`),
//! `content/mods/source-call.ts`, the `readQcCombat`/`armorStage` subset of
//! `content/mods/callbacks.ts`, and `readSourceTeamAliases` from
//! `content/mods/match.ts`. Declaration TYPES live in [`super::profile`]
//! and [`super::source_call`]; only team/combat mirrors (owned by the
//! mods domain) are defined here.
//!
//! JSON comes from [`qa_world::save::json`]; the typed accessors below
//! reproduce the donor `SaveReader` subset the document needs.

use std::collections::HashSet;

use qa_core::math::vec3;
use qa_world::save::json::{parse_source_json, SourceJson};

use super::profile::{
    QcClientCall, QcPickupCallerDeclaration, QcPickupDescriptor, QcPickupItemValue, QcPickupOperation, QcPickupRegion,
    QcPickupResource, QcPickupScalar, QcPrimaryWeaponStage, QcProofStatement, QcProtectionChannel, QcWeaponClient,
    QcWeaponObjectives, QcWeaponRepeat, QcWeaponResult,
};
use super::source_call::{ModCallbackInput, ModCallbackValue, ModSourceCall, ModSourceGlobal};
use crate::error::GuestError;

/// Original-to-shared team alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTeamAlias {
    /// Original identity, if any.
    pub source: Option<String>,
    /// Shared identity, if any.
    pub team: Option<String>,
}

/// Rerelease message dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcMessageDialect {
    /// Known retail messages.
    KnownRetail,
    /// Private rerelease TypeScript messages.
    ReTsPrivate,
}

/// Damage-scale region kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcDamageScaleKind {
    /// Multiplier stage.
    Multiplier,
    /// Identity stage.
    Identity,
    /// Transform stage.
    Transform,
}

/// Declared damage-scale region.
#[derive(Debug, Clone, PartialEq)]
pub struct QcDamageScale {
    /// Stage kind, if declared.
    pub kind: Option<QcDamageScaleKind>,
    /// Owning function.
    pub function: String,
    /// Region entry.
    pub entry: usize,
    /// Join statement.
    pub exit: usize,
    /// Damage global word.
    pub damage: usize,
    /// Proven instructions.
    pub statements: Vec<QcProofStatement>,
}

/// Regular-scale caller site inside an armor stage.
#[derive(Debug, Clone, PartialEq)]
pub struct QcArmorRegularScale {
    /// Calling function.
    pub caller: String,
    /// Call statement.
    pub statement: usize,
    /// Applied scale.
    pub scale: f64,
}

/// Armor-stage flag source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcArmorFlags {
    /// No flags.
    None,
    /// Bit flags in a global word.
    Bits {
        /// Flag word.
        word: usize,
        /// No-armor bit.
        no_armor: i64,
        /// No-power-armor bit.
        no_power_armor: i64,
        /// No-regular-armor bit.
        no_regular_armor: i64,
        /// Energy bit.
        energy: i64,
    },
}

/// Declared armor stage.
#[derive(Debug, Clone, PartialEq)]
pub struct QcArmorStage {
    /// Owning function.
    pub function: String,
    /// Region entry.
    pub entry: usize,
    /// Join statement.
    pub exit: usize,
    /// Target word.
    pub target: usize,
    /// Damage word.
    pub damage: usize,
    /// Saved-result word.
    pub saved: usize,
    /// Temporary caller scales.
    pub regular_scale: Vec<QcArmorRegularScale>,
    /// Flag source.
    pub flags: QcArmorFlags,
    /// Proven instructions.
    pub statements: Vec<QcProofStatement>,
}

/// Empty-armor fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct QcEmptyArmor {
    /// Armor item id.
    pub item: String,
    /// Absorption.
    pub absorption: f64,
}

/// Declared combat lowering (damage ABI plus optional stages).
#[derive(Debug, Clone, PartialEq)]
pub struct QcCombatDeclaration {
    /// Damage source call.
    pub damage: ModSourceCall,
    /// Optional damage-scale stage.
    pub damage_scale: Option<QcDamageScale>,
    /// Optional armor stage.
    pub armor_stage: Option<QcArmorStage>,
    /// Optional empty-armor fallback.
    pub empty_armor: Option<QcEmptyArmor>,
}

/// Full QuakeC compatibility document.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCCompatibility {
    /// Team aliases, if declared.
    pub teams: Option<Vec<SourceTeamAlias>>,
    /// Combat lowering, if declared.
    pub combat: Option<QcCombatDeclaration>,
    /// Primary weapon stage, if declared.
    pub weapon_stage: Option<QcPrimaryWeaponStage>,
    /// Message dialect.
    pub message_dialect: QcMessageDialect,
    /// Declared pickup callers.
    pub pickup_callers: Vec<QcPickupCallerDeclaration>,
}

const DOCUMENT: &str = "quakec-compatibility.json";

fn fail(path: &str, message: &str) -> GuestError {
    GuestError::BadSave(format!("{DOCUMENT}:{path}: {message}"))
}

fn field<'a>(value: &'a SourceJson, path: &str, name: &str) -> Result<&'a SourceJson, GuestError> {
    if !matches!(value, SourceJson::Object(_)) {
        return Err(fail(path, "expected a record"));
    }
    value
        .get(name)
        .ok_or_else(|| fail(&format!("{path}.{name}"), "missing field"))
}

fn opt<'a>(value: &'a SourceJson, name: &str) -> Option<&'a SourceJson> {
    value.get(name)
}

fn as_string(value: &SourceJson, path: &str) -> Result<String, GuestError> {
    match value {
        SourceJson::String(text) => Ok(text.clone()),
        _ => Err(fail(path, "expected a string")),
    }
}

fn as_number(value: &SourceJson, path: &str) -> Result<f64, GuestError> {
    match value {
        SourceJson::Number(number) => Ok(number.number()),
        _ => Err(fail(path, "expected a number")),
    }
}

fn as_finite(value: &SourceJson, path: &str) -> Result<f64, GuestError> {
    let number = as_number(value, path)?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(fail(path, "expected a finite number"))
    }
}

fn as_integer(value: &SourceJson, path: &str, minimum: i64) -> Result<i64, GuestError> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    let number = as_number(value, path)?;
    if number.fract() != 0.0 || number.abs() > MAX_SAFE_INTEGER || number < minimum as f64 {
        return Err(fail(path, "expected an integer in range"));
    }
    Ok(number as i64)
}

fn as_operand(value: &SourceJson, path: &str, minimum: i64) -> Result<i32, GuestError> {
    let integer = as_integer(value, path, minimum)?;
    i32::try_from(integer).map_err(|_| fail(path, "expected an integer in range"))
}

fn as_usize(value: &SourceJson, path: &str, minimum: i64) -> Result<usize, GuestError> {
    let integer = as_integer(value, path, minimum)?;
    usize::try_from(integer).map_err(|_| fail(path, "expected an integer in range"))
}

fn as_choice<'a>(value: &SourceJson, path: &str, choices: &[&'a str]) -> Result<&'a str, GuestError> {
    let text = as_string(value, path)?;
    choices
        .iter()
        .find(|choice| **choice == text)
        .copied()
        .ok_or_else(|| fail(path, "unexpected choice"))
}

fn as_list<T>(
    value: &SourceJson,
    path: &str,
    read: impl Fn(&SourceJson, &str) -> Result<T, GuestError>,
) -> Result<Vec<T>, GuestError> {
    match value {
        SourceJson::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| read(item, &format!("{path}[{index}]")))
            .collect(),
        _ => Err(fail(path, "expected a list")),
    }
}

fn as_nullable<T>(
    value: &SourceJson,
    path: &str,
    read: impl FnOnce(&SourceJson, &str) -> Result<T, GuestError>,
) -> Result<Option<T>, GuestError> {
    match value {
        SourceJson::Null => Ok(None),
        _ => read(value, path).map(Some),
    }
}

fn as_namespaced(value: &SourceJson, path: &str) -> Result<String, GuestError> {
    let text = as_string(value, path)?;
    let mut parts = text.splitn(2, ':');
    let (Some(head), Some(tail)) = (parts.next(), parts.next()) else {
        return Err(fail(path, "expected a namespaced id"));
    };
    if head.is_empty() || tail.is_empty() {
        return Err(fail(path, "expected a namespaced id"));
    }
    Ok(text)
}

fn read_proof_statement(value: &SourceJson, path: &str) -> Result<QcProofStatement, GuestError> {
    read_proof_statement_with(value, path, 0)
}

/// Proof reader with a configurable operand minimum: pickup/armor proofs
/// require `integer(0)` operands while weapon/damage proofs allow the
/// donor's unbounded `integer()` range (negative proofs simply never bind).
fn read_proof_statement_with(
    value: &SourceJson,
    path: &str,
    operands_minimum: i64,
) -> Result<QcProofStatement, GuestError> {
    Ok(QcProofStatement {
        opcode: as_operand(field(value, path, "opcode")?, &format!("{path}.opcode"), 0)?,
        a: as_operand(field(value, path, "a")?, &format!("{path}.a"), operands_minimum)?,
        b: as_operand(field(value, path, "b")?, &format!("{path}.b"), operands_minimum)?,
        c: as_operand(field(value, path, "c")?, &format!("{path}.c"), operands_minimum)?,
    })
}

fn read_vector(value: &SourceJson, path: &str) -> Result<qa_core::math::Vec3, GuestError> {
    let x = as_finite(field(value, path, "x")?, &format!("{path}.x"))?;
    let y = as_finite(field(value, path, "y")?, &format!("{path}.y"))?;
    let z = as_finite(field(value, path, "z")?, &format!("{path}.z"))?;
    if ![x, y, z].iter().all(|component| (*component as f32).is_finite()) {
        return Err(fail(path, "vector exceeds binary32 range"));
    }
    Ok(vec3(x as f32, y as f32, z as f32))
}

/// Read a mod source value (`content/mods/source-call.ts`).
pub fn read_mod_source_value(value: &SourceJson, path: &str) -> Result<ModCallbackValue, GuestError> {
    match as_choice(
        field(value, path, "kind")?,
        &format!("{path}.kind"),
        &["input", "float", "string", "vector"],
    )? {
        "input" => {
            let name = as_string(field(value, path, "name")?, &format!("{path}.name"))?;
            ModCallbackInput::parse(&name)
                .map(ModCallbackValue::Input)
                .ok_or_else(|| fail(&format!("{path}.name"), "unknown callback input"))
        }
        "float" => Ok(ModCallbackValue::Float(as_number(
            field(value, path, "value")?,
            &format!("{path}.value"),
        )?)),
        "string" => Ok(ModCallbackValue::String(as_string(
            field(value, path, "value")?,
            &format!("{path}.value"),
        )?)),
        _ => Ok(ModCallbackValue::Vector(read_vector(
            field(value, path, "value")?,
            &format!("{path}.value"),
        )?)),
    }
}

/// Read a mod source call (`content/mods/source-call.ts`).
pub fn read_mod_source_call(value: &SourceJson, path: &str) -> Result<ModSourceCall, GuestError> {
    Ok(ModSourceCall {
        function: as_string(field(value, path, "function")?, &format!("{path}.function"))?,
        arguments: as_list(
            field(value, path, "arguments")?,
            &format!("{path}.arguments"),
            read_mod_source_value,
        )?,
        globals: as_list(
            field(value, path, "globals")?,
            &format!("{path}.globals"),
            |entry, entry_path| {
                Ok(ModSourceGlobal {
                    name: as_string(field(entry, entry_path, "name")?, &format!("{entry_path}.name"))?,
                    value: read_mod_source_value(field(entry, entry_path, "value")?, &format!("{entry_path}.value"))?,
                })
            },
        )?,
    })
}

fn read_client_call(value: &SourceJson, path: &str) -> Result<QcClientCall, GuestError> {
    match value {
        SourceJson::String(_) => Ok(QcClientCall::Function(as_string(value, path)?)),
        _ => Ok(QcClientCall::Call(read_mod_source_call(value, path)?)),
    }
}

fn read_weapon_repeats(value: &SourceJson, path: &str) -> Result<Vec<QcWeaponRepeat>, GuestError> {
    as_list(value, path, |entry, entry_path| {
        let result = field(entry, entry_path, "result")?;
        let result_path = format!("{entry_path}.result");
        let accepted = as_integer(
            field(result, &result_path, "value")?,
            &format!("{result_path}.value"),
            0,
        )?;
        if accepted != 0 && accepted != 1 {
            return Err(fail(&format!("{result_path}.value"), "unexpected choice"));
        }
        Ok(QcWeaponRepeat {
            function: as_string(field(entry, entry_path, "function")?, &format!("{entry_path}.function"))?,
            entry: as_usize(field(entry, entry_path, "entry")?, &format!("{entry_path}.entry"), 0)?,
            exit: as_usize(field(entry, entry_path, "exit")?, &format!("{entry_path}.exit"), 0)?,
            result: QcWeaponResult {
                word: as_usize(field(result, &result_path, "word")?, &format!("{result_path}.word"), 28)?,
                value: accepted == 1,
            },
            statements: as_list(
                field(entry, entry_path, "statements")?,
                &format!("{entry_path}.statements"),
                |statement, statement_path| read_proof_statement_with(statement, statement_path, i64::MIN),
            )?,
        })
    })
}

/// Read a primary weapon stage (`content/q1/quakec/weapon-stage-declaration.ts`).
pub fn read_primary_weapon_stage(value: &SourceJson, path: &str) -> Result<QcPrimaryWeaponStage, GuestError> {
    let client = field(value, path, "client")?;
    let client_path = format!("{path}.client");
    let objectives = field(client, &client_path, "objectives")?;
    let objectives_path = format!("{client_path}.objectives");
    let kind = as_choice(
        field(objectives, &objectives_path, "kind")?,
        &format!("{objectives_path}.kind"),
        &["none", "call"],
    )?;
    if kind == "call" && opt(objectives, "call").is_some() && opt(objectives, "function").is_some() {
        return Err(fail(
            &objectives_path,
            "client objectives must declare one original call",
        ));
    }
    let objectives = match kind {
        "none" => QcWeaponObjectives::None,
        _ if opt(objectives, "call").is_some() => QcWeaponObjectives::Call(read_mod_source_call(
            field(objectives, &objectives_path, "call")?,
            &format!("{objectives_path}.call"),
        )?),
        _ => QcWeaponObjectives::Function(as_string(
            field(objectives, &objectives_path, "function")?,
            &format!("{objectives_path}.function"),
        )?),
    };
    Ok(QcPrimaryWeaponStage {
        dispatcher: as_string(field(value, path, "dispatcher")?, &format!("{path}.dispatcher"))?,
        continuations: as_list(
            field(value, path, "continuations")?,
            &format!("{path}.continuations"),
            |entry, entry_path| as_string(entry, entry_path),
        )?,
        repeats: read_weapon_repeats(field(value, path, "repeats")?, &format!("{path}.repeats"))?,
        client: QcWeaponClient {
            spawn: read_client_call(field(client, &client_path, "spawn")?, &format!("{client_path}.spawn"))?,
            select_spawn: read_client_call(
                field(client, &client_path, "selectSpawn")?,
                &format!("{client_path}.selectSpawn"),
            )?,
            objectives,
        },
    })
}

fn read_pickup_scalar(value: &SourceJson, path: &str) -> Result<QcPickupScalar, GuestError> {
    match as_choice(
        field(value, path, "kind")?,
        &format!("{path}.kind"),
        &["field", "global"],
    )? {
        "field" => Ok(QcPickupScalar::Field {
            name: as_string(field(value, path, "name")?, &format!("{path}.name"))?,
        }),
        _ => Ok(QcPickupScalar::Global {
            word: as_usize(field(value, path, "word")?, &format!("{path}.word"), 0)?,
        }),
    }
}

fn read_pickup_resource(value: &SourceJson, path: &str) -> Result<QcPickupResource, GuestError> {
    match as_choice(
        field(value, path, "kind")?,
        &format!("{path}.kind"),
        &["protection", "inventory"],
    )? {
        "protection" => {
            let channel = as_choice(
                field(value, path, "channel")?,
                &format!("{path}.channel"),
                &["regular", "powered"],
            )?;
            Ok(QcPickupResource::Protection {
                channel: if channel == "regular" {
                    QcProtectionChannel::Regular
                } else {
                    QcProtectionChannel::Powered
                },
            })
        }
        _ => Ok(QcPickupResource::Inventory {
            item: as_namespaced(field(value, path, "item")?, &format!("{path}.item"))?,
        }),
    }
}

fn read_pickup_item(
    value: &SourceJson,
    path: &str,
) -> Result<(String, Option<QcPickupResource>, Option<QcPickupScalar>), GuestError> {
    let resource = match opt(value, "resource") {
        None => return Err(fail(&format!("{path}.resource"), "missing field")),
        Some(resource) => as_nullable(resource, &format!("{path}.resource"), read_pickup_resource)?,
    };
    let count = match opt(value, "count") {
        None => None,
        Some(count) => Some(read_pickup_scalar(count, &format!("{path}.count"))?),
    };
    Ok((
        as_namespaced(field(value, path, "item")?, &format!("{path}.item"))?,
        resource,
        count,
    ))
}

/// Read a pickup caller (`content/q1/quakec/pickup-callers.ts`).
pub fn read_pickup_caller(value: &SourceJson, path: &str) -> Result<QcPickupCallerDeclaration, GuestError> {
    let descriptor = field(value, path, "descriptor")?;
    let descriptor_path = format!("{path}.descriptor");
    let kind = as_choice(
        field(descriptor, &descriptor_path, "kind")?,
        &format!("{descriptor_path}.kind"),
        &["constant", "string", "float"],
    )?;
    let descriptor = match kind {
        "constant" => {
            let (item, resource, count) = read_pickup_item(descriptor, &descriptor_path)?;
            super::profile::QcPickupDescriptor::Constant(super::profile::QcPickupItem { item, resource, count })
        }
        "string" => {
            let field_name = as_string(
                field(descriptor, &descriptor_path, "field")?,
                &format!("{descriptor_path}.field"),
            )?;
            let values = as_list(
                field(descriptor, &descriptor_path, "values")?,
                &format!("{descriptor_path}.values"),
                |entry, entry_path| {
                    let (item, resource, count) = read_pickup_item(entry, entry_path)?;
                    Ok(QcPickupItemValue {
                        item,
                        resource,
                        count,
                        value: as_string(field(entry, entry_path, "value")?, &format!("{entry_path}.value"))?,
                    })
                },
            )?;
            QcPickupDescriptor::String {
                field: field_name,
                values,
            }
        }
        _ => {
            let field_name = as_string(
                field(descriptor, &descriptor_path, "field")?,
                &format!("{descriptor_path}.field"),
            )?;
            let values = as_list(
                field(descriptor, &descriptor_path, "values")?,
                &format!("{descriptor_path}.values"),
                |entry, entry_path| {
                    let (item, resource, count) = read_pickup_item(entry, entry_path)?;
                    Ok(QcPickupItemValue {
                        item,
                        resource,
                        count,
                        value: as_finite(field(entry, entry_path, "value")?, &format!("{entry_path}.value"))?,
                    })
                },
            )?;
            QcPickupDescriptor::Float {
                field: field_name,
                values,
            }
        }
    };
    let dropped = match opt(value, "dropped") {
        None => None,
        Some(dropped) => Some(read_pickup_scalar(dropped, &format!("{path}.dropped"))?),
    };
    let regions = as_list(
        field(value, path, "regions")?,
        &format!("{path}.regions"),
        |region, region_path| {
            let operation = field(region, region_path, "operation")?;
            let operation_path = format!("{region_path}.operation");
            let kind = as_choice(
                field(operation, &operation_path, "kind")?,
                &format!("{operation_path}.kind"),
                &["decision", "admission", "grant", "consume"],
            )?;
            Ok(QcPickupRegion {
                entry: as_usize(field(region, region_path, "entry")?, &format!("{region_path}.entry"), 0)?,
                exit: as_usize(field(region, region_path, "exit")?, &format!("{region_path}.exit"), 0)?,
                statements: as_list(
                    field(region, region_path, "statements")?,
                    &format!("{region_path}.statements"),
                    read_proof_statement,
                )?,
                operation: match kind {
                    "decision" => QcPickupOperation::Decision {
                        word: as_usize(
                            field(operation, &operation_path, "word")?,
                            &format!("{operation_path}.word"),
                            0,
                        )?,
                        accepted: as_finite(
                            field(operation, &operation_path, "accepted")?,
                            &format!("{operation_path}.accepted"),
                        )?,
                    },
                    "admission" => QcPickupOperation::Admission,
                    "grant" => QcPickupOperation::Grant,
                    _ => QcPickupOperation::Consume,
                },
            })
        },
    )?;
    Ok(QcPickupCallerDeclaration {
        function: as_string(field(value, path, "function")?, &format!("{path}.function"))?,
        descriptor,
        dropped,
        regions,
    })
}

fn read_team_aliases(value: &SourceJson, path: &str) -> Result<Vec<SourceTeamAlias>, GuestError> {
    let values = as_list(value, path, |entry, entry_path| {
        Ok(SourceTeamAlias {
            source: match opt(entry, "source") {
                None => return Err(fail(&format!("{entry_path}.source"), "missing field")),
                Some(source) => as_nullable(source, &format!("{entry_path}.source"), as_string)?,
            },
            team: match opt(entry, "team") {
                None => return Err(fail(&format!("{entry_path}.team"), "missing field")),
                Some(team) => as_nullable(team, &format!("{entry_path}.team"), as_string)?,
            },
        })
    })?;
    let sources: HashSet<Option<&String>> = values.iter().map(|alias| alias.source.as_ref()).collect();
    let teams: HashSet<Option<&String>> = values.iter().map(|alias| alias.team.as_ref()).collect();
    if sources.len() != values.len()
        || teams.len() != values.len()
        || values
            .iter()
            .any(|alias| alias.source.as_deref() == Some("") || alias.team.as_deref() == Some(""))
    {
        return Err(fail(
            path,
            "team aliases require distinct original and shared identities",
        ));
    }
    Ok(values)
}

fn read_damage_scale(value: &SourceJson, path: &str) -> Result<QcDamageScale, GuestError> {
    let kind = match opt(value, "kind") {
        None => None,
        Some(kind) => Some(
            match as_choice(kind, &format!("{path}.kind"), &["multiplier", "identity", "transform"])? {
                "multiplier" => QcDamageScaleKind::Multiplier,
                "identity" => QcDamageScaleKind::Identity,
                _ => QcDamageScaleKind::Transform,
            },
        ),
    };
    Ok(QcDamageScale {
        kind,
        function: as_string(field(value, path, "function")?, &format!("{path}.function"))?,
        entry: as_usize(field(value, path, "entry")?, &format!("{path}.entry"), 0)?,
        exit: as_usize(field(value, path, "exit")?, &format!("{path}.exit"), 0)?,
        damage: as_usize(field(value, path, "damage")?, &format!("{path}.damage"), 28)?,
        statements: as_list(
            field(value, path, "statements")?,
            &format!("{path}.statements"),
            |entry, entry_path| read_proof_statement_with(entry, entry_path, i64::MIN),
        )?,
    })
}

fn read_armor_stage(value: &SourceJson, path: &str) -> Result<QcArmorStage, GuestError> {
    let flags = field(value, path, "flags")?;
    let flags_path = format!("{path}.flags");
    let flag_kind = as_choice(
        field(flags, &flags_path, "kind")?,
        &format!("{flags_path}.kind"),
        &["none", "bits"],
    )?;
    Ok(QcArmorStage {
        function: as_string(field(value, path, "function")?, &format!("{path}.function"))?,
        entry: as_usize(field(value, path, "entry")?, &format!("{path}.entry"), 0)?,
        exit: as_usize(field(value, path, "exit")?, &format!("{path}.exit"), 0)?,
        target: as_usize(field(value, path, "target")?, &format!("{path}.target"), 0)?,
        damage: as_usize(field(value, path, "damage")?, &format!("{path}.damage"), 0)?,
        saved: as_usize(field(value, path, "saved")?, &format!("{path}.saved"), 0)?,
        regular_scale: match opt(value, "regularScale") {
            None => Vec::new(),
            Some(scale) => as_list(scale, &format!("{path}.regularScale"), |site, site_path| {
                Ok(QcArmorRegularScale {
                    caller: as_string(field(site, site_path, "caller")?, &format!("{site_path}.caller"))?,
                    statement: as_usize(
                        field(site, site_path, "statement")?,
                        &format!("{site_path}.statement"),
                        0,
                    )?,
                    scale: as_number(field(site, site_path, "scale")?, &format!("{site_path}.scale"))?,
                })
            })?,
        },
        flags: if flag_kind == "none" {
            QcArmorFlags::None
        } else {
            QcArmorFlags::Bits {
                word: as_usize(field(flags, &flags_path, "word")?, &format!("{flags_path}.word"), 0)?,
                no_armor: as_integer(
                    field(flags, &flags_path, "noArmor")?,
                    &format!("{flags_path}.noArmor"),
                    0,
                )?,
                no_power_armor: as_integer(
                    field(flags, &flags_path, "noPowerArmor")?,
                    &format!("{flags_path}.noPowerArmor"),
                    0,
                )?,
                no_regular_armor: as_integer(
                    field(flags, &flags_path, "noRegularArmor")?,
                    &format!("{flags_path}.noRegularArmor"),
                    0,
                )?,
                energy: as_integer(field(flags, &flags_path, "energy")?, &format!("{flags_path}.energy"), 0)?,
            }
        },
        statements: as_list(
            field(value, path, "statements")?,
            &format!("{path}.statements"),
            read_proof_statement,
        )?,
    })
}

fn read_combat(value: &SourceJson, path: &str) -> Result<QcCombatDeclaration, GuestError> {
    Ok(QcCombatDeclaration {
        damage: read_mod_source_call(field(value, path, "damage")?, &format!("{path}.damage"))?,
        damage_scale: match opt(value, "damageScale") {
            None => None,
            Some(scale) => Some(read_damage_scale(scale, &format!("{path}.damageScale"))?),
        },
        armor_stage: match opt(value, "armorStage") {
            None => None,
            Some(stage) => Some(read_armor_stage(stage, &format!("{path}.armorStage"))?),
        },
        empty_armor: match opt(value, "emptyArmor") {
            None => None,
            Some(armor) => {
                let armor_path = format!("{path}.emptyArmor");
                let item = as_choice(
                    field(armor, &armor_path, "item")?,
                    &format!("{armor_path}.item"),
                    &["q1:item_armor1", "q1:item_armor2", "q1:item_armorInv"],
                )?;
                Some(QcEmptyArmor {
                    item: item.to_string(),
                    absorption: as_finite(
                        field(armor, &armor_path, "absorption")?,
                        &format!("{armor_path}.absorption"),
                    )?,
                })
            }
        },
    })
}

/// Read the compatibility document bound to `artifact_digest`. A missing
/// document means retail messages and no pickup callers.
pub fn read_quake_c_compatibility(
    bytes: Option<&[u8]>,
    artifact_digest: &str,
) -> Result<QuakeCCompatibility, GuestError> {
    let Some(bytes) = bytes else {
        return Ok(QuakeCCompatibility {
            teams: None,
            combat: None,
            weapon_stage: None,
            message_dialect: QcMessageDialect::KnownRetail,
            pickup_callers: Vec::new(),
        });
    };
    let text = std::str::from_utf8(bytes).map_err(|_| GuestError::BadSave(format!("{DOCUMENT}: invalid UTF-8")))?;
    let document = parse_source_json(text).map_err(GuestError::from)?;
    if as_integer(
        field(&document, DOCUMENT, "version")?,
        &format!("{DOCUMENT}.version"),
        0,
    )? != 1
    {
        return Err(fail(&format!("{DOCUMENT}.version"), "unsupported version"));
    }
    let digest = as_string(
        field(&document, DOCUMENT, "artifactDigest")?,
        &format!("{DOCUMENT}.artifactDigest"),
    )?;
    if digest != artifact_digest {
        return Err(fail(&format!("{DOCUMENT}.artifactDigest"), "artifact digest mismatch"));
    }
    Ok(QuakeCCompatibility {
        teams: match opt(&document, "teams") {
            None => None,
            Some(teams) => Some(read_team_aliases(teams, &format!("{DOCUMENT}.teams"))?),
        },
        combat: match opt(&document, "combat") {
            None => None,
            Some(combat) => Some(read_combat(combat, &format!("{DOCUMENT}.combat"))?),
        },
        weapon_stage: match opt(&document, "weaponStage") {
            None => None,
            Some(stage) => Some(read_primary_weapon_stage(stage, &format!("{DOCUMENT}.weaponStage"))?),
        },
        message_dialect: match opt(&document, "messageDialect") {
            None => QcMessageDialect::KnownRetail,
            Some(dialect) => match as_choice(
                dialect,
                &format!("{DOCUMENT}.messageDialect"),
                &["known-retail", "quake-1-re-ts-private"],
            )? {
                "known-retail" => QcMessageDialect::KnownRetail,
                _ => QcMessageDialect::ReTsPrivate,
            },
        },
        pickup_callers: match opt(&document, "pickupCallers") {
            None => Vec::new(),
            Some(callers) => as_list(callers, &format!("{DOCUMENT}.pickupCallers"), read_pickup_caller)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn document(body: &str) -> Vec<u8> {
        format!("{{\"version\":1,\"artifactDigest\":\"{DIGEST}\",{body}}}").into_bytes()
    }

    #[test]
    fn missing_document_means_retail() {
        let compatibility = read_quake_c_compatibility(None, DIGEST).unwrap();
        assert_eq!(compatibility.message_dialect, QcMessageDialect::KnownRetail);
        assert!(compatibility.pickup_callers.is_empty());
        assert!(compatibility.teams.is_none());
    }

    #[test]
    fn minimal_document_parses() {
        let compatibility =
            read_quake_c_compatibility(Some(&document("\"messageDialect\":\"known-retail\"")), DIGEST).unwrap();
        assert_eq!(compatibility.message_dialect, QcMessageDialect::KnownRetail);
    }

    #[test]
    fn version_and_digest_are_pinned() {
        let bad_version = format!("{{\"version\":2,\"artifactDigest\":\"{DIGEST}\"}}").into_bytes();
        assert!(read_quake_c_compatibility(Some(&bad_version), DIGEST).is_err());
        let bad_digest = document("\"messageDialect\":\"known-retail\"");
        assert!(read_quake_c_compatibility(Some(&bad_digest), "sha256:ffff").is_err());
        assert!(read_quake_c_compatibility(Some(b"\xff\xfe"), DIGEST).is_err());
    }

    #[test]
    fn teams_require_distinct_identities() {
        let body = "\"teams\":[{\"source\":\"a\",\"team\":\"x\"},{\"source\":null,\"team\":null}]";
        let compatibility = read_quake_c_compatibility(Some(&document(body)), DIGEST).unwrap();
        assert_eq!(compatibility.teams.unwrap().len(), 2);
        let dupe = "\"teams\":[{\"source\":\"a\",\"team\":\"x\"},{\"source\":\"a\",\"team\":\"y\"}]";
        assert!(read_quake_c_compatibility(Some(&document(dupe)), DIGEST).is_err());
    }

    #[test]
    fn pickup_caller_parses() {
        let body = "\"pickupCallers\":[{\"function\":\"touch\",\"descriptor\":{\"kind\":\"constant\",\"item\":\"q1:item_health\",\"resource\":null},\"regions\":[{\"entry\":1,\"exit\":2,\"statements\":[{\"opcode\":31,\"a\":4,\"b\":30,\"c\":0}],\"operation\":{\"kind\":\"admission\"}}]}]";
        let compatibility = read_quake_c_compatibility(Some(&document(body)), DIGEST).unwrap();
        assert_eq!(compatibility.pickup_callers.len(), 1);
        assert!(matches!(
            compatibility.pickup_callers[0].descriptor,
            QcPickupDescriptor::Constant(_)
        ));
    }

    #[test]
    fn weapon_stage_parses() {
        let body = "\"weaponStage\":{\"dispatcher\":\"W_Attack\",\"continuations\":[],\"repeats\":[],\"client\":{\"spawn\":\"spawn\",\"selectSpawn\":{\"function\":\"sel\",\"arguments\":[],\"globals\":[]},\"objectives\":{\"kind\":\"none\"}}}";
        let compatibility = read_quake_c_compatibility(Some(&document(body)), DIGEST).unwrap();
        let stage = compatibility.weapon_stage.unwrap();
        assert_eq!(stage.dispatcher, "W_Attack");
        assert!(matches!(stage.client.spawn, QcClientCall::Function(_)));
        assert!(matches!(stage.client.select_spawn, QcClientCall::Call(_)));
    }

    #[test]
    fn combat_parses() {
        let body = "\"combat\":{\"damage\":{\"function\":\"T_Damage\",\"arguments\":[],\"globals\":[]},\"emptyArmor\":{\"item\":\"q1:item_armor2\",\"absorption\":0.6}}";
        let compatibility = read_quake_c_compatibility(Some(&document(body)), DIGEST).unwrap();
        let combat = compatibility.combat.unwrap();
        assert_eq!(combat.damage.function, "T_Damage");
        assert_eq!(combat.empty_armor.unwrap().item, "q1:item_armor2");
    }

    #[test]
    fn source_values_cover_all_kinds() {
        let parsed = parse_source_json("{\"kind\":\"input\",\"name\":\"amount\"}").unwrap();
        assert!(matches!(
            read_mod_source_value(&parsed, "test").unwrap(),
            ModCallbackValue::Input(ModCallbackInput::Amount)
        ));
        let parsed = parse_source_json("{\"kind\":\"vector\",\"value\":{\"x\":1,\"y\":2,\"z\":3}}").unwrap();
        assert!(matches!(
            read_mod_source_value(&parsed, "test").unwrap(),
            ModCallbackValue::Vector(_)
        ));
        let parsed = parse_source_json("{\"kind\":\"input\",\"name\":\"bogus\"}").unwrap();
        assert!(read_mod_source_value(&parsed, "test").is_err());
    }
}
