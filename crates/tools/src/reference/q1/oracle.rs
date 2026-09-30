//! Q1 source-derived oracle (donor `tools/reference/q1/oracle.ts`).
//!
//! Evaluates source-derived equations: binary32 scalar stores, one
//! `SV_RunThink` invocation, the mg1 hub sigil gate, and the mg3 rune
//! counter. Float semantics are bit-exact with the donor: `as f32`
//! round-trips implement `Math.fround`, and all arithmetic stays in `f64`.

use crate::error::ToolsError;
use crate::json::Json;

/// Scalar operators from `pr_exec.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarOperator {
    /// Floating addition.
    Add,
    /// Floating subtraction.
    Subtract,
    /// Floating multiplication.
    Multiply,
    /// Floating division.
    Divide,
    /// Signed integer bitwise and after truncation.
    BitAnd,
    /// Signed integer bitwise or after truncation.
    BitOr,
}

impl ScalarOperator {
    fn parse(value: &str) -> Result<Self, ToolsError> {
        match value {
            "add" => Ok(Self::Add),
            "subtract" => Ok(Self::Subtract),
            "multiply" => Ok(Self::Multiply),
            "divide" => Ok(Self::Divide),
            "bit-and" => Ok(Self::BitAnd),
            "bit-or" => Ok(Self::BitOr),
            _ => Err(ToolsError::parse("Unknown scalar operator")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Subtract => "subtract",
            Self::Multiply => "multiply",
            Self::Divide => "divide",
            Self::BitAnd => "bit-and",
            Self::BitOr => "bit-or",
        }
    }
}

/// QuakeC callback globals: float time plus entity identifiers.
#[derive(Debug, Clone, Copy)]
pub struct CallbackGlobals {
    /// Global time (binary32 stored).
    pub time: f64,
    /// `self` entity identifier.
    pub self_entity: i64,
    /// `other` entity identifier.
    pub other_entity: i64,
}

impl CallbackGlobals {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("time".to_owned(), Json::float(self.time)),
            ("self".to_owned(), Json::int(self.self_entity)),
            ("other".to_owned(), Json::int(self.other_entity)),
        ])
    }
}

/// Prescribed think-callback effect.
#[derive(Debug, Clone, Copy)]
pub enum ThinkEffect {
    /// Entity survives with cleared schedule.
    Retain,
    /// Entity is removed.
    Remove,
    /// Entity reschedules.
    Reschedule {
        /// New schedule (binary32 stored).
        next_think: f64,
    },
}

/// One scalar-program operation.
#[derive(Debug, Clone, Copy)]
pub struct ScalarOperation {
    /// Operator.
    pub operator: ScalarOperator,
    /// Operand (binary32 stored).
    pub operand: f64,
}

/// Oracle input.
#[derive(Debug, Clone)]
pub enum Q1Input {
    /// Binary32 scalar program.
    ScalarProgram {
        /// Initial value (binary32 stored).
        initial: f64,
        /// Operations, at most 1024.
        operations: Vec<ScalarOperation>,
    },
    /// One `SV_RunThink` invocation.
    RunThink {
        /// Server time (binary64).
        server_time: f64,
        /// Frame length (binary64).
        frame_time: f64,
        /// Entity schedule (binary32 stored).
        next_think: f64,
        /// Entity identifier.
        entity: i64,
        /// Incoming globals.
        globals: CallbackGlobals,
        /// Prescribed callback effect.
        effect: ThinkEffect,
    },
    /// mg1 hub sigil gate.
    Mg1Hub {
        /// Server flags.
        server_flags: i32,
    },
    /// mg3 rune counter spawn plus one use call.
    Mg3Counter {
        /// Server flags.
        server_flags: i32,
        /// Spawn count (binary32 stored).
        count: f64,
        /// Cooperative mode.
        coop: bool,
        /// Spawn flags.
        spawn_flags: i32,
        /// Activator entity.
        activator: i64,
        /// Counter entity.
        entity: i64,
    },
}

impl Q1Input {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::ScalarProgram { initial, operations } => Json::object(vec![
                ("kind".to_owned(), Json::string("scalar-program")),
                ("initial".to_owned(), Json::float(*initial)),
                (
                    "operations".to_owned(),
                    Json::array(
                        operations
                            .iter()
                            .map(|op| {
                                Json::object(vec![
                                    ("operator".to_owned(), Json::string(op.operator.as_str())),
                                    ("operand".to_owned(), Json::float(op.operand)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
            Self::RunThink {
                server_time,
                frame_time,
                next_think,
                entity,
                globals,
                effect,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("run-think")),
                ("serverTime".to_owned(), Json::float(*server_time)),
                ("frameTime".to_owned(), Json::float(*frame_time)),
                ("nextThink".to_owned(), Json::float(*next_think)),
                ("entity".to_owned(), Json::int(*entity)),
                ("globals".to_owned(), globals.to_json()),
                (
                    "effect".to_owned(),
                    match effect {
                        ThinkEffect::Retain => Json::object(vec![("kind".to_owned(), Json::string("retain"))]),
                        ThinkEffect::Remove => Json::object(vec![("kind".to_owned(), Json::string("remove"))]),
                        ThinkEffect::Reschedule { next_think } => Json::object(vec![
                            ("kind".to_owned(), Json::string("reschedule")),
                            ("nextThink".to_owned(), Json::float(*next_think)),
                        ]),
                    },
                ),
            ]),
            Self::Mg1Hub { server_flags } => Json::object(vec![
                ("kind".to_owned(), Json::string("mg1-hub")),
                ("serverFlags".to_owned(), Json::int(i64::from(*server_flags))),
            ]),
            Self::Mg3Counter {
                server_flags,
                count,
                coop,
                spawn_flags,
                activator,
                entity,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("mg3-counter")),
                ("serverFlags".to_owned(), Json::int(i64::from(*server_flags))),
                ("count".to_owned(), Json::float(*count)),
                ("coop".to_owned(), Json::boolean(*coop)),
                ("spawnFlags".to_owned(), Json::int(i64::from(*spawn_flags))),
                ("activator".to_owned(), Json::int(*activator)),
                ("entity".to_owned(), Json::int(*entity)),
            ]),
        }
    }
}

/// Oracle output.
#[derive(Debug, Clone)]
pub enum Q1Output {
    /// Scalar values plus bit patterns after each store.
    ScalarProgram {
        /// Value after each store.
        values: Vec<f64>,
        /// Bit patterns as eight hex digits.
        bits: Vec<String>,
    },
    /// `SV_RunThink` result.
    RunThink {
        /// Whether the callback ran.
        ran: bool,
        /// Whether physics continues.
        continue_physics: bool,
        /// Resulting schedule.
        next_think: f64,
        /// Whether the entity was freed.
        free: bool,
        /// Outgoing globals.
        globals: CallbackGlobals,
        /// Callback entry state, when the callback ran.
        callback_entry: Option<CallbackEntry>,
    },
    /// mg1 gate result.
    Mg1Hub {
        /// Mask of present sigils.
        present_mask: i32,
        /// Invoked calls.
        calls: Vec<String>,
    },
    /// mg3 counter result.
    Mg3Counter {
        /// Whether spawn filtering removed the counter.
        removed: bool,
        /// Effective count.
        count: f64,
        /// Installed use callback.
        use_callback: Option<String>,
        /// Counted runes.
        runes: Option<i32>,
        /// `SUB_UseTargets` invocation, when the threshold is reached.
        callback: Option<TargetCallback>,
    },
}

/// Callback entry state: globals plus cleared schedule.
#[derive(Debug, Clone, Copy)]
pub struct CallbackEntry {
    /// Entry time.
    pub time: f64,
    /// Entry `self`.
    pub self_entity: i64,
    /// Entry `other` (world).
    pub other_entity: i64,
}

impl CallbackEntry {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("time".to_owned(), Json::float(self.time)),
            ("self".to_owned(), Json::int(self.self_entity)),
            ("other".to_owned(), Json::int(self.other_entity)),
            ("nextThink".to_owned(), Json::int(0)),
        ])
    }
}

/// A `SUB_UseTargets` invocation.
#[derive(Debug, Clone, Copy)]
pub struct TargetCallback {
    /// Invoking entity.
    pub self_entity: i64,
    /// Activating entity.
    pub activator: i64,
}

impl TargetCallback {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("name".to_owned(), Json::string("SUB_UseTargets")),
            ("self".to_owned(), Json::int(self.self_entity)),
            ("activator".to_owned(), Json::int(self.activator)),
        ])
    }
}

impl Q1Output {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::ScalarProgram { values, bits } => Json::object(vec![
                ("kind".to_owned(), Json::string("scalar-program")),
                (
                    "values".to_owned(),
                    Json::array(values.iter().map(|value| Json::float(*value)).collect()),
                ),
                ("bits".to_owned(), Json::array(bits.iter().map(Json::string).collect())),
            ]),
            Self::RunThink {
                ran,
                continue_physics,
                next_think,
                free,
                globals,
                callback_entry,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("run-think")),
                ("ran".to_owned(), Json::boolean(*ran)),
                ("continuePhysics".to_owned(), Json::boolean(*continue_physics)),
                ("nextThink".to_owned(), Json::float(*next_think)),
                ("free".to_owned(), Json::boolean(*free)),
                ("globals".to_owned(), globals.to_json()),
                (
                    "callbackEntry".to_owned(),
                    callback_entry.as_ref().map_or(Json::Null, CallbackEntry::to_json),
                ),
            ]),
            Self::Mg1Hub { present_mask, calls } => Json::object(vec![
                ("kind".to_owned(), Json::string("mg1-hub")),
                ("requiredMask".to_owned(), Json::int(31)),
                ("presentMask".to_owned(), Json::int(i64::from(*present_mask))),
                (
                    "calls".to_owned(),
                    Json::array(calls.iter().map(Json::string).collect()),
                ),
            ]),
            Self::Mg3Counter {
                removed,
                count,
                use_callback,
                runes,
                callback,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("mg3-counter")),
                ("removed".to_owned(), Json::boolean(*removed)),
                ("count".to_owned(), Json::float(*count)),
                ("use".to_owned(), use_callback.as_ref().map_or(Json::Null, Json::string)),
                (
                    "runes".to_owned(),
                    runes.map_or(Json::Null, |runes| Json::int(i64::from(runes))),
                ),
                (
                    "callback".to_owned(),
                    callback.as_ref().map_or(Json::Null, TargetCallback::to_json),
                ),
            ]),
        }
    }
}

fn object(value: &Json) -> Result<&[(String, Json)], ToolsError> {
    value.as_object().ok_or_else(|| ToolsError::parse("Expected an object"))
}

fn fields(value: &[(String, Json)], expected: &[&str]) -> Result<(), ToolsError> {
    for (key, _) in value {
        if !expected.contains(&key.as_str()) {
            return Err(ToolsError::parse(format!("Unexpected field {key}")));
        }
    }
    for key in expected {
        if !value.iter().any(|(present, _)| present == key) {
            return Err(ToolsError::parse(format!("Missing field {key}")));
        }
    }
    Ok(())
}

fn field<'a>(value: &'a [(String, Json)], key: &str) -> &'a Json {
    value
        .iter()
        .find(|(present, _)| present == key)
        .map(|(_, found)| found)
        .expect("checked field")
}

fn kind_of(value: &[(String, Json)]) -> Option<&str> {
    value
        .iter()
        .find(|(present, _)| present == "kind")
        .and_then(|(_, found)| found.as_str())
}

fn finite(value: &Json) -> Result<f64, ToolsError> {
    match value.as_f64() {
        Some(number) if number.is_finite() => Ok(number),
        _ => Err(ToolsError::parse("Expected a finite number")),
    }
}

fn stored_float(value: &Json) -> Result<f64, ToolsError> {
    let result = fround(finite(value)?);
    if !result.is_finite() {
        return Err(ToolsError::parse("Value exceeds finite binary32 scope"));
    }
    Ok(result)
}

/// Binary32 store: `Math.fround` round-to-nearest ties-to-even.
fn fround(value: f64) -> f64 {
    f64::from(value as f32)
}

fn int_operand(value: &Json) -> Result<i32, ToolsError> {
    let result = stored_float(value)?;
    if result < -2_147_483_648.0 || result >= 2_147_483_648.0 {
        return Err(ToolsError::parse("Float-to-int conversion exceeds signed int32 scope"));
    }
    Ok(result.trunc() as i32)
}

fn entity_id(value: &Json) -> Result<i64, ToolsError> {
    let result = finite(value)?;
    if result.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&result) {
        return Err(ToolsError::parse("Expected a nonnegative entity identifier"));
    }
    Ok(result as i64)
}

fn parse_globals(value: &Json) -> Result<CallbackGlobals, ToolsError> {
    let item = object(value)?;
    fields(item, &["time", "self", "other"])?;
    Ok(CallbackGlobals {
        time: stored_float(field(item, "time"))?,
        self_entity: entity_id(field(item, "self"))?,
        other_entity: entity_id(field(item, "other"))?,
    })
}

fn parse_effect(value: &Json) -> Result<ThinkEffect, ToolsError> {
    let item = object(value)?;
    match kind_of(item) {
        Some("retain") => {
            fields(item, &["kind"])?;
            Ok(ThinkEffect::Retain)
        }
        Some("remove") => {
            fields(item, &["kind"])?;
            Ok(ThinkEffect::Remove)
        }
        Some("reschedule") => {
            fields(item, &["kind", "nextThink"])?;
            Ok(ThinkEffect::Reschedule {
                next_think: stored_float(field(item, "nextThink"))?,
            })
        }
        _ => Err(ToolsError::parse("Unknown think callback effect")),
    }
}

fn parse_operation(value: &Json) -> Result<ScalarOperation, ToolsError> {
    let item = object(value)?;
    fields(item, &["operator", "operand"])?;
    let operator = field(item, "operator")
        .as_str()
        .ok_or_else(|| ToolsError::parse("Unknown scalar operator"))?;
    let operator = ScalarOperator::parse(operator)?;
    let operand = stored_float(field(item, "operand"))?;
    if operator == ScalarOperator::Divide && operand == 0.0 {
        return Err(ToolsError::parse("Division by zero exceeds finite oracle scope"));
    }
    Ok(ScalarOperation { operator, operand })
}

/// Parse and validate an oracle input document.
pub fn parse_q1_input(value: &Json) -> Result<Q1Input, ToolsError> {
    let item = object(value)?;
    match kind_of(item) {
        Some("scalar-program") => {
            fields(item, &["kind", "initial", "operations"])?;
            let raw = field(item, "operations")
                .as_array()
                .ok_or_else(|| ToolsError::parse("Expected scalar operations array"))?;
            if raw.len() > 1024 {
                return Err(ToolsError::parse("Scalar program exceeds 1024-operation capture scope"));
            }
            let mut operations = Vec::with_capacity(raw.len());
            for entry in raw {
                operations.push(parse_operation(entry)?);
            }
            Ok(Q1Input::ScalarProgram {
                initial: stored_float(field(item, "initial"))?,
                operations,
            })
        }
        Some("run-think") => {
            fields(
                item,
                &[
                    "kind",
                    "serverTime",
                    "frameTime",
                    "nextThink",
                    "entity",
                    "globals",
                    "effect",
                ],
            )?;
            let server_time = finite(field(item, "serverTime"))?;
            let frame_time = finite(field(item, "frameTime"))?;
            if server_time < 0.0 || frame_time < 0.0 || !(server_time + frame_time).is_finite() {
                return Err(ToolsError::parse("Invalid server frame interval"));
            }
            stored_float(field(item, "serverTime"))?;
            Ok(Q1Input::RunThink {
                server_time,
                frame_time,
                next_think: stored_float(field(item, "nextThink"))?,
                entity: entity_id(field(item, "entity"))?,
                globals: parse_globals(field(item, "globals"))?,
                effect: parse_effect(field(item, "effect"))?,
            })
        }
        Some("mg1-hub") => {
            fields(item, &["kind", "serverFlags"])?;
            Ok(Q1Input::Mg1Hub {
                server_flags: int_operand(field(item, "serverFlags"))?,
            })
        }
        Some("mg3-counter") => {
            fields(
                item,
                &[
                    "kind",
                    "serverFlags",
                    "count",
                    "coop",
                    "spawnFlags",
                    "activator",
                    "entity",
                ],
            )?;
            let coop = field(item, "coop")
                .as_bool()
                .ok_or_else(|| ToolsError::parse("Expected coop boolean"))?;
            Ok(Q1Input::Mg3Counter {
                server_flags: int_operand(field(item, "serverFlags"))?,
                count: stored_float(field(item, "count"))?,
                coop,
                spawn_flags: int_operand(field(item, "spawnFlags"))?,
                activator: entity_id(field(item, "activator"))?,
                entity: entity_id(field(item, "entity"))?,
            })
        }
        _ => Err(ToolsError::parse("Unknown Q1 oracle input kind")),
    }
}

/// Big-endian binary32 bit pattern as eight hex digits.
fn float_bits(value: f64) -> String {
    format!("{:08x}", (value as f32).to_bits())
}

fn scalar(initial: f64, operations: &[ScalarOperation]) -> Result<Q1Output, ToolsError> {
    let mut value = initial;
    let mut values = vec![value];
    let mut bits = vec![float_bits(value)];
    for operation in operations {
        match operation.operator {
            ScalarOperator::Add => value += operation.operand,
            ScalarOperator::Subtract => value -= operation.operand,
            ScalarOperator::Multiply => value *= operation.operand,
            ScalarOperator::Divide => value /= operation.operand,
            ScalarOperator::BitAnd => {
                value = f64::from(int_trunc(value)? & int_trunc(operation.operand)?);
            }
            ScalarOperator::BitOr => {
                value = f64::from(int_trunc(value)? | int_trunc(operation.operand)?);
            }
        }
        value = store_result(value)?;
        values.push(value);
        bits.push(float_bits(value));
    }
    Ok(Q1Output::ScalarProgram { values, bits })
}

/// Truncate an already-stored float to `i32`, matching `intOperand` on values
/// that passed validation (bitwise inputs are always in scope here).
fn int_trunc(value: f64) -> Result<i32, ToolsError> {
    if value < -2_147_483_648.0 || value >= 2_147_483_648.0 {
        return Err(ToolsError::parse("Float-to-int conversion exceeds signed int32 scope"));
    }
    Ok(value.trunc() as i32)
}

fn store_result(value: f64) -> Result<f64, ToolsError> {
    let result = fround(value);
    if !result.is_finite() {
        return Err(ToolsError::parse("Value exceeds finite binary32 scope"));
    }
    Ok(result)
}

fn run_think(
    server_time: f64,
    frame_time: f64,
    next_think: f64,
    entity: i64,
    globals: CallbackGlobals,
    effect: ThinkEffect,
) -> Q1Output {
    if next_think <= 0.0 || next_think > server_time + frame_time {
        return Q1Output::RunThink {
            ran: false,
            continue_physics: true,
            next_think,
            free: false,
            globals,
            callback_entry: None,
        };
    }
    let time = if next_think < server_time {
        fround(server_time)
    } else {
        next_think
    };
    let entry = CallbackEntry {
        time,
        self_entity: entity,
        other_entity: 0,
    };
    let callback_globals = CallbackGlobals {
        time,
        self_entity: entity,
        other_entity: 0,
    };
    let (next_think, free, continue_physics) = match effect {
        ThinkEffect::Retain => (0.0, false, true),
        ThinkEffect::Remove => (-1.0, true, false),
        ThinkEffect::Reschedule { next_think } => (next_think, false, true),
    };
    Q1Output::RunThink {
        ran: true,
        continue_physics,
        next_think,
        free,
        globals: callback_globals,
        callback_entry: Some(entry),
    }
}

/// Evaluate an oracle input document.
pub fn run_q1_oracle(value: &Json) -> Result<Q1Output, ToolsError> {
    match parse_q1_input(value)? {
        Q1Input::ScalarProgram { initial, operations } => scalar(initial, &operations),
        Q1Input::RunThink {
            server_time,
            frame_time,
            next_think,
            entity,
            globals,
            effect,
        } => Ok(run_think(server_time, frame_time, next_think, entity, globals, effect)),
        Q1Input::Mg1Hub { server_flags } => {
            let present_mask = server_flags & 31;
            let calls = vec![if present_mask == 31 {
                "trigger_changelevel()".to_owned()
            } else {
                "remove(self)".to_owned()
            }];
            Ok(Q1Output::Mg1Hub { present_mask, calls })
        }
        Q1Input::Mg3Counter {
            server_flags,
            count,
            coop,
            spawn_flags,
            activator,
            entity,
        } => {
            if coop && spawn_flags & 131_072 != 0 || !coop && spawn_flags & 32768 != 0 {
                return Ok(Q1Output::Mg3Counter {
                    removed: true,
                    count,
                    use_callback: None,
                    runes: None,
                    callback: None,
                });
            }
            let count = if count == 0.0 { 2.0 } else { count };
            let mut runes = 0;
            for bit in [1, 2, 4, 8] {
                if server_flags & bit != 0 {
                    runes += 1;
                }
            }
            let callback = if f64::from(runes) >= count {
                Some(TargetCallback {
                    self_entity: entity,
                    activator,
                })
            } else {
                None
            };
            Ok(Q1Output::Mg3Counter {
                removed: false,
                count,
                use_callback: Some("rune_counter_use".to_owned()),
                runes: Some(runes),
                callback,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    // Covers the oracle cases of tools/reference/q1/oracle.test.ts (pinned cases,
    // mg1/mg3 sweeps, signed zero, int truncation, malformed boundary); the
    // capture-backed cases live in q1/capture.rs tests.
    use crate::json::{deep_strict_equal, parse_json};
    use crate::reference::q1::cases::q1_cases;

    use super::*;

    #[test]
    fn pinned_cases_agree_with_the_oracle() {
        for item in q1_cases() {
            let observed = run_q1_oracle(&item.input.to_json()).expect(&format!("evaluate {}", item.id));
            assert!(
                deep_strict_equal(&observed.to_json(), &item.expected.to_json()),
                "{}",
                item.id
            );
        }
    }

    #[test]
    fn mg1_exhausts_five_required_bits() {
        for flags in 0..128 {
            let input = parse_json(&format!(r#"{{"kind": "mg1-hub", "serverFlags": {flags}}}"#)).expect("input");
            let Q1Output::Mg1Hub { calls, .. } = run_q1_oracle(&input).expect("evaluate") else {
                panic!("wrong oracle result kind for flags {flags}");
            };
            let expected = if flags % 32 == 31 {
                "trigger_changelevel()"
            } else {
                "remove(self)"
            };
            assert_eq!(calls, vec![expected.to_owned()], "flags {flags}");
        }
    }

    #[test]
    fn mg3_exhausts_rune_masks_and_thresholds() {
        let rune_counts = [0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4];
        for flags in 0..64 {
            let runes = rune_counts[flags % 16];
            for threshold in [-1.0, 0.0, 1.0, 2.0, 2.5, 3.0, 4.0, 5.0] {
                let input = parse_json(&format!(
                    r#"{{"kind": "mg3-counter", "serverFlags": {flags}, "count": {threshold}, "coop": false, "spawnFlags": 0, "entity": 40, "activator": 2}}"#,
                ))
                .expect("input");
                let Q1Output::Mg3Counter {
                    runes: observed,
                    callback,
                    ..
                } = run_q1_oracle(&input).expect("evaluate")
                else {
                    panic!("wrong oracle result kind for flags {flags}");
                };
                assert_eq!(observed, Some(runes), "flags {flags} threshold {threshold}");
                let count = if threshold == 0.0 { 2.0 } else { threshold };
                assert_eq!(
                    callback.is_some(),
                    f64::from(runes) >= count,
                    "flags {flags} threshold {threshold}"
                );
            }
        }
    }

    #[test]
    fn mg3_permits_matching_spawn_flags() {
        for (coop, spawn_flags) in [(true, 32768), (false, 131072)] {
            let input = parse_json(&format!(
                r#"{{"kind": "mg3-counter", "serverFlags": 3, "count": 0, "coop": {coop}, "spawnFlags": {spawn_flags}, "entity": 40, "activator": 2}}"#,
            ))
            .expect("input");
            let observed = run_q1_oracle(&input).expect("evaluate");
            let expected = parse_json(
                r#"{"kind": "mg3-counter", "removed": false, "count": 2, "use": "rune_counter_use", "runes": 2, "callback": {"name": "SUB_UseTargets", "self": 40, "activator": 2}}"#,
            )
            .expect("expected");
            assert!(deep_strict_equal(&observed.to_json(), &expected), "coop {coop}");
        }
    }

    #[test]
    fn numeric_records_distinguish_signed_zero() {
        let input = parse_json(
            r#"{"kind": "scalar-program", "initial": -0, "operations": [{"operator": "multiply", "operand": 2}]}"#,
        )
        .expect("input");
        let observed = run_q1_oracle(&input).expect("evaluate");
        let expected =
            parse_json(r#"{"kind": "scalar-program", "values": [-0, -0], "bits": ["80000000", "80000000"]}"#)
                .expect("expected");
        assert!(
            deep_strict_equal(&observed.to_json(), &expected),
            "{}",
            observed.to_json().render()
        );
    }

    #[test]
    fn signed_int_conversion_truncates_toward_zero() {
        let input = parse_json(
            r#"{"kind": "scalar-program", "initial": -1.75, "operations": [{"operator": "bit-and", "operand": 15}]}"#,
        )
        .expect("input");
        let observed = run_q1_oracle(&input).expect("evaluate");
        let expected =
            parse_json(r#"{"kind": "scalar-program", "values": [-1.75, 15], "bits": ["bfe00000", "41700000"]}"#)
                .expect("expected");
        assert!(
            deep_strict_equal(&observed.to_json(), &expected),
            "{}",
            observed.to_json().render()
        );
    }

    #[test]
    fn malformed_inputs_fail_at_the_boundary() {
        let documents = [
            "null",
            "[]",
            "\"mg1-hub\"",
            "{}",
            r#"{"kind": "other"}"#,
            r#"{"kind": "mg1-hub"}"#,
            r#"{"kind": "mg1-hub", "serverFlags": "31"}"#,
            r#"{"kind": "mg1-hub", "serverFlags": 2147483648}"#,
            r#"{"kind": "mg1-hub", "serverFlags": -2147483904}"#,
            r#"{"kind": "mg1-hub", "serverFlags": 31, "ignored": true}"#,
            r#"{"kind": "scalar-program", "initial": 1e100, "operations": []}"#,
            r#"{"kind": "scalar-program", "initial": 1, "operations": "add"}"#,
            r#"{"kind": "scalar-program", "initial": 1, "operations": [null]}"#,
            r#"{"kind": "scalar-program", "initial": 1, "operations": [{"operator": "power", "operand": 2}]}"#,
            r#"{"kind": "scalar-program", "initial": 1, "operations": [{"operator": "divide", "operand": 0}]}"#,
            r#"{"kind": "scalar-program", "initial": 3e38, "operations": [{"operator": "multiply", "operand": 2}]}"#,
            r#"{"kind": "scalar-program", "initial": 2147483648, "operations": [{"operator": "bit-or", "operand": 1}]}"#,
            r#"{"kind": "run-think", "serverTime": 10, "frameTime": -1, "nextThink": 10, "entity": 3, "globals": {"time": 7, "self": 99, "other": 98}, "effect": {"kind": "retain"}}"#,
            r#"{"kind": "run-think", "serverTime": 10, "frameTime": 1, "nextThink": 10, "entity": 0.5, "globals": {"time": 7, "self": 99, "other": 98}, "effect": {"kind": "retain"}}"#,
            r#"{"kind": "run-think", "serverTime": 10, "frameTime": 1, "nextThink": 10, "entity": 3, "globals": {"time": 7, "self": 99, "other": 98}, "effect": {"kind": "arbitrary-code"}}"#,
            r#"{"kind": "mg3-counter", "serverFlags": 3, "count": 2, "coop": 1, "spawnFlags": 0, "entity": 40, "activator": 2}"#,
        ];
        for document in documents {
            let value = parse_json(document).expect("test input parses");
            assert!(run_q1_oracle(&value).is_err(), "{document}");
        }
        for value in [
            Json::object(vec![
                ("kind".to_owned(), Json::string("mg1-hub")),
                ("serverFlags".to_owned(), Json::float(f64::NAN)),
            ]),
            Json::object(vec![
                ("kind".to_owned(), Json::string("scalar-program")),
                ("initial".to_owned(), Json::float(f64::INFINITY)),
                ("operations".to_owned(), Json::array(Vec::new())),
            ]),
        ] {
            assert!(run_q1_oracle(&value).is_err(), "{}", value.render());
        }
    }
}
