//! Qualified damage scales (`src/content/q1/quakec/damage-scale.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/damage-scale.ts`
//! (`qcDamageScale`, `evaluateQcDamageScale`, `evaluateQcDamageAmount`).

use std::collections::{BTreeMap, HashMap, HashSet};

use qa_core::identity::ActorId;

use crate::contract::{
    ModCallbackInput, ModCallbackValue, ModQcDamageScale, ModQcDamageScaleKind, ModRuntimeValue, ModSourceCall,
};

use super::armor_stage::{qc_region_private_writes_are_dead, qc_statement_access};
use super::damage_call::{qc_damage_call_layout, QcDamageCallLayout, QcDamageRole, QcDamageRoleLocation};
use super::qc_view::{
    signed_qc_branch, validate_qc_source_call, with_qc_source_call, InlineStandalone, QcInlineRegion, QcMachineView,
    QcOpcode, QcProgramView, QcValueType, StandaloneScope,
};
use super::{fround, QcError};

/// Qualified damage scale (donor `QcDamageScale`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcDamageScale {
    /// Scale kind.
    pub kind: ModQcDamageScaleKind,
    /// Inline region.
    pub region: QcInlineRegion,
    /// Borrowed frame and scratch words.
    pub scratch: Vec<usize>,
    /// Qualified scaling constants.
    pub constants: Vec<QcScaleConstant>,
    /// Scale call.
    pub call: ModSourceCall,
}

/// Qualified scaling constant (donor `QcDamageScale.constants` entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcScaleConstant {
    /// Constant word.
    pub word: usize,
    /// Required bits.
    pub bits: i32,
}

/// Abstract value (donor `Value`).
#[derive(Debug, Clone, PartialEq)]
enum Value {
    /// Attacker entity.
    Actor,
    /// Independent scalar.
    Scalar {
        /// Known constant.
        constant: Option<f64>,
        /// Constant origin words.
        origins: Vec<usize>,
    },
    /// Scaled damage.
    Scaled {
        /// Multiplicative operations applied.
        operations: usize,
        /// Whether the value is still identical to the input.
        identity: bool,
        /// Whether all scales were exact powers of two.
        powers_of_two: bool,
    },
}

/// Word range helper (donor `words`).
fn words(start: usize, length: usize) -> Vec<usize> {
    (start..start.saturating_add(length)).collect()
}

/// Scalar binary opcodes (donor `binary`).
fn is_binary(opcode: QcOpcode) -> bool {
    matches!(
        opcode,
        QcOpcode::AddF
            | QcOpcode::SubF
            | QcOpcode::MulF
            | QcOpcode::DivF
            | QcOpcode::EqF
            | QcOpcode::NeF
            | QcOpcode::EqS
            | QcOpcode::NeS
            | QcOpcode::Le
            | QcOpcode::Ge
            | QcOpcode::Lt
            | QcOpcode::Gt
            | QcOpcode::And
            | QcOpcode::Or
            | QcOpcode::BitAnd
            | QcOpcode::BitOr
    )
}

/// Qualify the exact pure attacker policy and its original suppression
/// boundary (donor `qcDamageScale`).
pub fn qc_damage_scale(
    program: &QcProgramView,
    call: &ModSourceCall,
    source: Option<&ModQcDamageScale>,
) -> Result<Option<QcDamageScale>, QcError> {
    let Some(source) = source else {
        return Ok(None);
    };
    let reject = |reason: &str| -> QcError {
        QcError::program(format!("Unsupported QC damage scale: {reason}"), program.source)
    };
    let function = program.function_named(&source.function)?;
    let end = program.function_end(function.first_statement);
    let entry = source.entry as usize;
    let exit = source.exit as usize;
    let first = usize::try_from(function.first_statement).unwrap_or(usize::MAX);
    if source.function != call.function
        || function.first_statement <= 0
        || function.named_builtin
        || entry < first
        || exit <= entry
        || exit >= end
    {
        return Err(reject("original damage region bounds"));
    }
    let layout: QcDamageCallLayout = qc_damage_call_layout(program, Some(call))?;
    let amount = layout.roles.get(&QcDamageRole::Amount).cloned().unwrap_or_default();
    if amount.len() != 1 {
        return Err(reject("scale result requires one original float input"));
    }
    let Some(location) = amount.first() else {
        return Err(reject("scale result requires one original float input"));
    };
    let damage = match location {
        QcDamageRoleLocation::Argument { frame_word, .. } => *frame_word,
        QcDamageRoleLocation::Global { word } => *word,
    };
    let parameter_end = function
        .parameter_start
        .saturating_add(function.parameter_sizes.iter().sum::<usize>());
    if source.damage as usize != damage || function.local_words < parameter_end.saturating_sub(function.parameter_start)
    {
        return Err(reject("original damage parameter"));
    }
    let available: HashSet<ModCallbackInput> = [
        ModCallbackInput::Slf,
        ModCallbackInput::Attacker,
        ModCallbackInput::Inflictor,
        ModCallbackInput::Amount,
        ModCallbackInput::Time,
    ]
    .into_iter()
    .collect();
    validate_qc_source_call(program, call, &available, "damage scale")?;
    // Declared context values per word (`Some(None)` is a non-attacker
    // declared entity; missing words are undeclared).
    let mut context: HashMap<usize, Option<Value>> = HashMap::new();
    for global in &call.globals {
        let Some(definition) = program.global_named(&global.name) else {
            return Err(reject("missing declared context global"));
        };
        match &global.value {
            ModCallbackValue::Input(name) => {
                context.insert(
                    definition.offset,
                    match name {
                        ModCallbackInput::Attacker => Some(Value::Actor),
                        ModCallbackInput::Amount => Some(Value::Scaled {
                            operations: 0,
                            identity: true,
                            powers_of_two: true,
                        }),
                        ModCallbackInput::Time => Some(Value::Scalar {
                            constant: None,
                            origins: Vec::new(),
                        }),
                        _ => None,
                    },
                );
            }
            ModCallbackValue::Float(value) => {
                context.insert(
                    definition.offset,
                    Some(Value::Scalar {
                        constant: Some(fround(*value)),
                        origins: Vec::new(),
                    }),
                );
            }
            _ => {
                for word in words(
                    definition.offset,
                    usize::from(definition.def_type == QcValueType::Vector) * 2 + 1,
                ) {
                    context.insert(
                        word,
                        Some(Value::Scalar {
                            constant: None,
                            origins: Vec::new(),
                        }),
                    );
                }
            }
        }
    }
    let mut prefix_writes: HashSet<usize> = HashSet::new();
    let mut amount_inputs: HashSet<usize> = HashSet::from([damage]);
    amount_inputs.extend(
        context
            .iter()
            .filter_map(|(word, value)| matches!(value, Some(Value::Scaled { .. })).then_some(*word)),
    );
    for statement in program.statements.get(first..entry).unwrap_or(&[]) {
        let access = qc_statement_access(statement);
        if access.read.iter().any(|word| amount_inputs.contains(word)) {
            return Err(reject("damage-dependent policy precedes the scaling region"));
        }
        if statement.opcode >= QcOpcode::StorePF && statement.opcode <= QcOpcode::StorePFn
            || statement.opcode.is_call()
            || statement.opcode == QcOpcode::State
        {
            return Err(reject("source side effects precede the scale query"));
        }
        if !(statement.opcode >= QcOpcode::StoreF
            && statement.opcode <= QcOpcode::StoreFn
            && statement.a == statement.b)
        {
            prefix_writes.extend(access.write);
        }
    }
    let attacker_touched = layout
        .roles
        .get(&QcDamageRole::Attacker)
        .cloned()
        .unwrap_or_default()
        .iter()
        .any(|location| {
            prefix_writes.contains(&match location {
                QcDamageRoleLocation::Argument { frame_word, .. } => *frame_word,
                QcDamageRoleLocation::Global { word } => *word,
            })
        });
    if prefix_writes.contains(&damage) || attacker_touched {
        return Err(reject("source changes scale arguments before the region"));
    }
    if source.statements.len() != exit.saturating_sub(entry).saturating_add(1) {
        return Err(reject("incomplete original instructions"));
    }
    for (offset, expected) in source.statements.iter().enumerate() {
        let actual = program.statements.get(entry + offset);
        if actual.is_none_or(|actual| {
            actual.opcode.as_u32() != expected.opcode
                || u32::from(actual.a) != expected.a
                || u32::from(actual.b) != expected.b
                || u32::from(actual.c) != expected.c
        }) {
            return Err(reject("instructions differ from artifact"));
        }
    }
    let mut named: HashSet<usize> = HashSet::new();
    for global in program.globals.iter().filter(|global| !global.name.is_empty()) {
        named.extend(words(
            global.offset,
            usize::from(global.def_type == QcValueType::Vector) * 2 + 1,
        ));
    }
    let mut mutable: HashSet<usize> = HashSet::new();
    for statement in program.statements {
        mutable.extend(qc_statement_access(statement).write);
    }
    let initial_len = program.initial_globals.len();
    let mut scratch: HashSet<usize> = HashSet::new();
    let mut constants: HashSet<usize> = HashSet::new();
    let kind = source.kind;
    let merge = |left: &Value, right: &Value| -> Result<Value, QcError> {
        let same_kind = matches!(
            (left, right),
            (Value::Actor, Value::Actor)
                | (Value::Scalar { .. }, Value::Scalar { .. })
                | (Value::Scaled { .. }, Value::Scaled { .. })
        );
        if !same_kind {
            if kind == ModQcDamageScaleKind::Transform
                && !matches!(left, Value::Actor)
                && !matches!(right, Value::Actor)
            {
                return Ok(Value::Scaled {
                    operations: 0,
                    identity: false,
                    powers_of_two: false,
                });
            }
            return Err(reject("control flow replaces damage with an independent value"));
        }
        match (left, right) {
            (Value::Actor, Value::Actor) => Ok(Value::Actor),
            (
                Value::Scalar {
                    constant: left_constant,
                    origins: left_origins,
                },
                Value::Scalar {
                    constant: right_constant,
                    origins: right_origins,
                },
            ) => {
                if left_constant != right_constant {
                    return Ok(Value::Scalar {
                        constant: None,
                        origins: Vec::new(),
                    });
                }
                let mut origins = left_origins.clone();
                origins.extend(right_origins.iter().copied());
                Ok(Value::Scalar {
                    constant: *left_constant,
                    origins,
                })
            }
            (
                Value::Scaled {
                    operations: left_operations,
                    identity: left_identity,
                    powers_of_two: left_powers,
                },
                Value::Scaled {
                    operations: right_operations,
                    identity: right_identity,
                    powers_of_two: right_powers,
                },
            ) => Ok(Value::Scaled {
                operations: (*left_operations).max(*right_operations),
                identity: *left_identity && *right_identity,
                powers_of_two: *left_powers && *right_powers,
            }),
            _ => Err(reject("inconsistent source value")),
        }
    };
    let mut initial_values: HashMap<usize, Value> = HashMap::from([(
        damage,
        Value::Scaled {
            operations: 0,
            identity: true,
            powers_of_two: true,
        },
    )]);
    for location in layout.roles.get(&QcDamageRole::Attacker).cloned().unwrap_or_default() {
        if let QcDamageRoleLocation::Argument { frame_word, .. } = location {
            initial_values.insert(frame_word, Value::Actor);
        }
    }
    let mut pending: BTreeMap<usize, HashMap<usize, Value>> = BTreeMap::from([(entry, initial_values)]);
    let mut joined = false;
    let mut multiplied = false;
    while let Some(pc) = pending.keys().next().copied() {
        let Some(mut values) = pending.remove(&pc) else {
            return Err(reject("missing source path"));
        };
        if pc == exit {
            let result = values.get(&damage);
            let retains_scalar = match result {
                Some(Value::Scalar { .. }) => kind == ModQcDamageScaleKind::Transform,
                Some(Value::Scaled { .. }) => true,
                _ => false,
            };
            if !retains_scalar {
                return Err(reject("join does not retain scalar damage"));
            }
            if kind == ModQcDamageScaleKind::Identity && !matches!(result, Some(Value::Scaled { identity: true, .. })) {
                return Err(reject("identity region changes its original damage input"));
            }
            joined = true;
            continue;
        }
        let Some(statement) = program.statements.get(pc) else {
            return Err(reject("missing source instruction"));
        };
        let opcode = statement.opcode;
        let a = usize::from(statement.a);
        let b = usize::from(statement.b);
        let c = usize::from(statement.c);
        let read = |word: usize, values: &HashMap<usize, Value>| -> Result<Value, QcError> {
            if let Some(value) = values.get(&word) {
                return Ok(value.clone());
            }
            if word < 28 || word.saturating_mul(4).saturating_add(4) > initial_len {
                return Err(reject("unavailable original word"));
            }
            let frame_end = function.parameter_start.saturating_add(function.local_words);
            if word >= function.parameter_start && word < frame_end || !named.contains(&word) && mutable.contains(&word)
            {
                return Err(reject("undeclared local or target/inflictor dependency"));
            }
            match context.get(&word) {
                Some(None) => return Err(reject("non-attacker declared entity context")),
                Some(Some(supplied)) => return Ok(supplied.clone()),
                None => {}
            }
            if prefix_writes.contains(&word) {
                return Err(reject("unexecuted source context assignment"));
            }
            if program.global_named("time").is_some_and(|time| time.offset == word) {
                return Err(reject("current source time must be declared"));
            }
            if program
                .globals
                .iter()
                .any(|global| global.offset == word && global.def_type == QcValueType::Entity)
            {
                return Err(reject("non-attacker entity context"));
            }
            if mutable.contains(&word) {
                return Ok(Value::Scalar {
                    constant: None,
                    origins: Vec::new(),
                });
            }
            Ok(Value::Scalar {
                constant: Some(f64::from(program.initial_f32(word)?)),
                origins: vec![word],
            })
        };
        let independent = |word: usize, values: &HashMap<usize, Value>| -> Result<Value, QcError> {
            let value = read(word, values)?;
            if matches!(value, Value::Scaled { .. }) {
                return Err(reject("nonlinear damage-dependent predicate or address"));
            }
            Ok(value)
        };
        let mut write = |word: usize, value: Value, values: &mut HashMap<usize, Value>| -> Result<(), QcError> {
            let frame_end = function.parameter_start.saturating_add(function.local_words);
            if word < 28
                || word.saturating_mul(4).saturating_add(4) > initial_len
                || word != damage
                    && (word >= function.parameter_start && word < parameter_end
                        || named.contains(&word) && !(word >= parameter_end && word < frame_end))
            {
                return Err(reject("source write outside private frame"));
            }
            if word == damage
                && (matches!(value, Value::Actor)
                    || kind != ModQcDamageScaleKind::Transform && !matches!(value, Value::Scaled { .. }))
            {
                return Err(reject("damage overwritten by independent value"));
            }
            scratch.insert(word);
            values.insert(word, value);
            Ok(())
        };
        let mut edge = |from: usize, to: i64, values: &HashMap<usize, Value>| -> Result<(), QcError> {
            if to <= from as i64 || to < entry as i64 || to > exit as i64 {
                return Err(reject("escaping or backward branch"));
            }
            let to = to as usize;
            match pending.remove(&to) {
                None => {
                    pending.insert(to, values.clone());
                }
                Some(previous) => {
                    let mut merged = HashMap::new();
                    for (word, value) in &previous {
                        if let Some(next) = values.get(word) {
                            merged.insert(*word, merge(value, next)?);
                        }
                    }
                    pending.insert(to, merged);
                }
            }
            Ok(())
        };
        if opcode == QcOpcode::If || opcode == QcOpcode::IfNot {
            if kind == ModQcDamageScaleKind::Transform {
                read(a, &values)?;
            } else {
                independent(a, &values)?;
            }
            edge(pc, pc as i64 + i64::from(signed_qc_branch(statement.b)), &values)?;
        } else if opcode == QcOpcode::Goto {
            edge(pc, pc as i64 + i64::from(signed_qc_branch(statement.a)), &values)?;
            continue;
        } else if opcode == QcOpcode::LoadF || opcode == QcOpcode::LoadS {
            if !matches!(read(a, &values)?, Value::Actor) {
                return Err(reject("entity reads must belong to the original attacker"));
            }
            independent(b, &values)?;
            write(
                c,
                Value::Scalar {
                    constant: None,
                    origins: Vec::new(),
                },
                &mut values,
            )?;
        } else if opcode == QcOpcode::StoreF || opcode == QcOpcode::StoreEnt || opcode == QcOpcode::StoreS {
            let value = read(a, &values)?;
            write(b, value, &mut values)?;
        } else if opcode == QcOpcode::NotF || opcode == QcOpcode::NotS || opcode == QcOpcode::NotEnt {
            let value = if kind == ModQcDamageScaleKind::Transform {
                read(a, &values)?
            } else {
                independent(a, &values)?
            };
            write(
                c,
                match value {
                    Value::Scaled {
                        operations,
                        powers_of_two,
                        ..
                    } => Value::Scaled {
                        operations,
                        identity: false,
                        powers_of_two,
                    },
                    _ => Value::Scalar {
                        constant: None,
                        origins: Vec::new(),
                    },
                },
                &mut values,
            )?;
        } else if is_binary(opcode) {
            let left = read(a, &values)?;
            let right = read(b, &values)?;
            if kind == ModQcDamageScaleKind::Transform {
                if matches!(left, Value::Actor) || matches!(right, Value::Actor) {
                    return Err(reject("damage arithmetic cannot consume entity references"));
                }
                write(
                    c,
                    if matches!(left, Value::Scaled { .. }) || matches!(right, Value::Scaled { .. }) {
                        Value::Scaled {
                            operations: 0,
                            identity: false,
                            powers_of_two: false,
                        }
                    } else {
                        Value::Scalar {
                            constant: None,
                            origins: Vec::new(),
                        }
                    },
                    &mut values,
                )?;
            } else if !matches!(left, Value::Scaled { .. }) && !matches!(right, Value::Scaled { .. }) {
                write(
                    c,
                    Value::Scalar {
                        constant: None,
                        origins: Vec::new(),
                    },
                    &mut values,
                )?;
            } else {
                let scaled = if matches!(left, Value::Scaled { .. }) {
                    &left
                } else {
                    &right
                };
                let factor = if matches!(left, Value::Scalar { .. }) {
                    &left
                } else {
                    &right
                };
                let (
                    Value::Scaled {
                        operations,
                        identity,
                        powers_of_two,
                    },
                    Value::Scalar { constant, origins },
                ) = (scaled, factor)
                else {
                    return Err(reject("nonmultiplicative damage transformation"));
                };
                if opcode != QcOpcode::MulF && opcode != QcOpcode::DivF
                    || opcode == QcOpcode::DivF && !matches!(left, Value::Scaled { .. })
                {
                    return Err(reject("nonmultiplicative damage transformation"));
                }
                let power = constant.is_some_and(|constant| {
                    constant > 0.0 && {
                        let log = constant.log2();
                        log.is_finite() && log.fract() == 0.0
                    }
                });
                if opcode == QcOpcode::DivF && !power {
                    return Err(reject("division requires an exactly representable reciprocal"));
                }
                if power {
                    constants.extend(origins.iter().copied());
                }
                let increasing = power
                    && constant.is_some_and(|constant| {
                        if opcode == QcOpcode::DivF {
                            constant <= 1.0
                        } else {
                            constant >= 1.0
                        }
                    });
                if *operations > 0 && !(*powers_of_two && increasing) {
                    return Err(reject("multiple rounded scales cannot be represented by one factor"));
                }
                multiplied = true;
                write(
                    c,
                    Value::Scaled {
                        operations: operations + 1,
                        identity: *identity && *constant == Some(1.0),
                        powers_of_two: *powers_of_two && increasing,
                    },
                    &mut values,
                )?;
            }
        } else {
            return Err(reject(
                "region must contain only source reads and scalar frame operations",
            ));
        }
        edge(pc, pc as i64 + 1, &values)?;
    }
    if !joined || kind != ModQcDamageScaleKind::Identity && kind != ModQcDamageScaleKind::Transform && !multiplied {
        return Err(reject("region has no original multiplicative result"));
    }
    for pc in first..end {
        if pc >= entry && pc < exit {
            continue;
        }
        let Some(statement) = program.statements.get(pc) else {
            continue;
        };
        let destination = if statement.opcode == QcOpcode::Goto {
            pc as i64 + i64::from(signed_qc_branch(statement.a))
        } else if statement.opcode == QcOpcode::If || statement.opcode == QcOpcode::IfNot {
            pc as i64 + i64::from(signed_qc_branch(statement.b))
        } else {
            -1
        };
        if destination > entry as i64 && destination < exit as i64 {
            return Err(reject("incoming interior branch"));
        }
        if (pc as i64) < entry as i64 && destination >= exit as i64 {
            return Err(reject("source path bypasses scaling before continuing damage"));
        }
        if pc >= exit && destination >= first as i64 && destination <= entry as i64 {
            return Err(reject("source path repeats scaling"));
        }
    }
    let mut private_writes = scratch.clone();
    private_writes.remove(&damage);
    if !qc_region_private_writes_are_dead(program, exit, end, &private_writes)? {
        return Err(reject("private scale outputs remain live after suppression"));
    }
    let mut scratch: Vec<usize> = scratch.into_iter().collect();
    scratch.sort_unstable();
    let mut constants: Vec<QcScaleConstant> = constants
        .into_iter()
        .map(|word| program.initial_i32(word).map(|bits| QcScaleConstant { word, bits }))
        .collect::<Result<Vec<_>, _>>()?;
    constants.sort_by_key(|constant| constant.word);
    Ok(Some(QcDamageScale {
        kind,
        call: call.clone(),
        scratch,
        constants,
        region: QcInlineRegion {
            function_index: function.index,
            entry,
            exit,
            replaceable: true,
            standalone: Some(InlineStandalone {
                saved: damage,
                scope: match location {
                    QcDamageRoleLocation::Argument { .. } => StandaloneScope::Frame,
                    QcDamageRoleLocation::Global { .. } => StandaloneScope::Global,
                },
            }),
        },
    }))
}

/// Borrow only the qualified original frame and scratch words, restoring
/// them before returning (donor `evaluateQcDamageScale`).
pub fn evaluate_qc_damage_scale(
    machine: &dyn QcMachineView,
    scale: &QcDamageScale,
    owner: &ActorId,
    actor_reference: i32,
    seconds: f64,
) -> Result<f64, QcError> {
    if scale.kind == ModQcDamageScaleKind::Transform {
        return Err(QcError::program(
            "Original QC amount transform cannot be queried as a multiplier",
            machine.program_source(),
        ));
    }
    evaluate_qc_damage_amount(machine, scale, owner, actor_reference, seconds, 1.0)
}

/// Evaluate the qualified damage amount (donor `evaluateQcDamageAmount`).
pub fn evaluate_qc_damage_amount(
    machine: &dyn QcMachineView,
    scale: &QcDamageScale,
    owner: &ActorId,
    actor_reference: i32,
    seconds: f64,
    amount: f64,
) -> Result<f64, QcError> {
    let actor = ModRuntimeValue::Actor(Some(owner.clone()));
    let inputs: HashMap<ModCallbackInput, ModRuntimeValue> = [
        (ModCallbackInput::Slf, actor.clone()),
        (ModCallbackInput::Attacker, actor.clone()),
        (ModCallbackInput::Inflictor, actor),
        (ModCallbackInput::Amount, ModRuntimeValue::Float(amount)),
        (ModCallbackInput::Time, ModRuntimeValue::Float(seconds)),
    ]
    .into_iter()
    .collect();
    for constant in &scale.constants {
        if machine.global_int(constant.word)? != constant.bits {
            return Err(QcError::program(
                "Original QC scaling constant changed after qualification",
                machine.program_source(),
            ));
        }
    }
    let mut saved = Vec::with_capacity(scale.scratch.len());
    for word in &scale.scratch {
        saved.push((*word, machine.global_int(*word)?));
    }
    let result = with_qc_source_call(
        machine,
        &scale.call,
        &inputs,
        &|actor: Option<&ActorId>| -> Result<i32, QcError> {
            match actor {
                None => machine.entity_reference(0),
                Some(actor) => {
                    if actor != owner {
                        return Err(QcError::program(
                            "QC damage scale changed its source attacker",
                            machine.program_source(),
                        ));
                    }
                    Ok(actor_reference)
                }
            }
        },
        &mut |count| machine.execute_region(&scale.region, count),
    );
    let result = match result {
        Ok(result) => {
            if !result.is_finite() || result < 0.0 {
                Err(QcError::program(
                    "Original QC damage amount is not finite and nonnegative",
                    machine.program_source(),
                ))
            } else {
                Ok(result)
            }
        }
        Err(error) => Err(error),
    };
    for (word, value) in &saved {
        machine.set_global_int(*word, *value)?;
    }
    result
}
