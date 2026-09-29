//! Port of `src/compat/qc/source-call.ts`: validated mod source calls
//! that stage the declared original ABI for one invocation only.
//!
//! Contracts absorption: [`ModSourceCall`], [`ModCallbackValue`],
//! [`ModRuntimeValue`], and [`ModCallbackInput`] mirror
//! `src/contracts/mod-callbacks.ts` minimally for this module (plus the
//! weapon-stage client calls in [`super::profile`]). JSON readers for these
//! values live in [`super::compatibility`], next to the document parser.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::machine::QcMachine;
use super::program::{QcProgram, QcValueType};
use crate::error::GuestError;

/// Named gameplay input lowerable into a source call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackInput {
    /// View angles vector.
    ViewAngles,
    /// Attack button.
    Attack,
    /// Jump button.
    Jump,
    /// Impulse number.
    Impulse,
    /// Forward move.
    ForwardMove,
    /// Side move.
    SideMove,
    /// Up move.
    UpMove,
    /// Calling entity.
    Self_,
    /// Other entity.
    Other,
    /// Activating entity.
    Activator,
    /// Attacking entity.
    Attacker,
    /// Inflicting entity.
    Inflictor,
    /// Damage amount.
    Amount,
    /// Damage flags.
    DamageFlags,
    /// Regular protection scale.
    RegularProtectionScale,
    /// Knockback.
    Knockback,
    /// A point in space.
    Point,
    /// A direction vector.
    Direction,
    /// A surface normal.
    Normal,
    /// An item id string.
    Item,
    /// Source time.
    Time,
    /// Frame elapsed time.
    Elapsed,
    /// Transform result.
    Result,
    /// Pickup count.
    PickupCount,
    /// Whether the pickup has a count.
    PickupHasCount,
    /// Whether the pickup was dropped.
    PickupDropped,
}

impl ModCallbackInput {
    /// Source spelling of the input name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ViewAngles => "view-angles",
            Self::Attack => "attack",
            Self::Jump => "jump",
            Self::Impulse => "impulse",
            Self::ForwardMove => "forward-move",
            Self::SideMove => "side-move",
            Self::UpMove => "up-move",
            Self::Self_ => "self",
            Self::Other => "other",
            Self::Activator => "activator",
            Self::Attacker => "attacker",
            Self::Inflictor => "inflictor",
            Self::Amount => "amount",
            Self::DamageFlags => "damage-flags",
            Self::RegularProtectionScale => "regular-protection-scale",
            Self::Knockback => "knockback",
            Self::Point => "point",
            Self::Direction => "direction",
            Self::Normal => "normal",
            Self::Item => "item",
            Self::Time => "time",
            Self::Elapsed => "elapsed",
            Self::Result => "result",
            Self::PickupCount => "pickup-count",
            Self::PickupHasCount => "pickup-has-count",
            Self::PickupDropped => "pickup-dropped",
        }
    }

    /// Parse a source input name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "view-angles" => Self::ViewAngles,
            "attack" => Self::Attack,
            "jump" => Self::Jump,
            "impulse" => Self::Impulse,
            "forward-move" => Self::ForwardMove,
            "side-move" => Self::SideMove,
            "up-move" => Self::UpMove,
            "self" => Self::Self_,
            "other" => Self::Other,
            "activator" => Self::Activator,
            "attacker" => Self::Attacker,
            "inflictor" => Self::Inflictor,
            "amount" => Self::Amount,
            "damage-flags" => Self::DamageFlags,
            "regular-protection-scale" => Self::RegularProtectionScale,
            "knockback" => Self::Knockback,
            "point" => Self::Point,
            "direction" => Self::Direction,
            "normal" => Self::Normal,
            "item" => Self::Item,
            "time" => Self::Time,
            "elapsed" => Self::Elapsed,
            "result" => Self::Result,
            "pickup-count" => Self::PickupCount,
            "pickup-has-count" => Self::PickupHasCount,
            "pickup-dropped" => Self::PickupDropped,
            _ => return None,
        })
    }
}

/// A declared source-call value: gameplay input or constant.
#[derive(Debug, Clone, PartialEq)]
pub enum ModCallbackValue {
    /// Gameplay input resolved per invocation.
    Input(ModCallbackInput),
    /// Constant float.
    Float(f64),
    /// Constant string.
    String(String),
    /// Constant vector.
    Vector(Vec3),
}

/// A resolved runtime value: constants plus lowered actors.
#[derive(Debug, Clone, PartialEq)]
pub enum ModRuntimeValue {
    /// Float constant.
    Float(f64),
    /// Vector constant.
    Vector(Vec3),
    /// String constant.
    String(String),
    /// Lowered actor reference (null keeps QC world semantics).
    Actor(Option<ActorId>),
}

/// One staged source global.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSourceGlobal {
    /// Global name.
    pub name: String,
    /// Staged value.
    pub value: ModCallbackValue,
}

/// A declared call into one original compiled function.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSourceCall {
    /// Source function name.
    pub function: String,
    /// Positional arguments.
    pub arguments: Vec<ModCallbackValue>,
    /// Staged globals.
    pub globals: Vec<ModSourceGlobal>,
}

/// QC word type of a declared value.
#[must_use]
pub fn qc_source_value_type(value: &ModCallbackValue) -> QcValueType {
    match value {
        ModCallbackValue::Float(_) => QcValueType::Float,
        ModCallbackValue::String(_) => QcValueType::String,
        ModCallbackValue::Vector(_) => QcValueType::Vector,
        ModCallbackValue::Input(input) => match input {
            ModCallbackInput::Self_
            | ModCallbackInput::Other
            | ModCallbackInput::Activator
            | ModCallbackInput::Attacker
            | ModCallbackInput::Inflictor => QcValueType::Entity,
            ModCallbackInput::Point
            | ModCallbackInput::Direction
            | ModCallbackInput::Normal
            | ModCallbackInput::ViewAngles => QcValueType::Vector,
            ModCallbackInput::Item => QcValueType::String,
            _ => QcValueType::Float,
        },
    }
}

/// Validate a source call against its program: readable inputs, compatible
/// signature, and uniquely-typed globals.
pub fn validate_qc_source_call(
    program: &QcProgram,
    call: &ModSourceCall,
    available: &HashSet<ModCallbackInput>,
    label: &str,
) -> Result<(), GuestError> {
    for value in call
        .arguments
        .iter()
        .chain(call.globals.iter().map(|global| &global.value))
    {
        if let ModCallbackValue::Input(name) = value {
            if !available.contains(name) {
                return Err(GuestError::callback(format!(
                    "Mod {label} cannot read {}",
                    name.name()
                )));
            }
        }
    }
    let function = program
        .function_named(&call.function)
        .map_err(|_| GuestError::callback(format!("Mod callback {} has an incompatible source signature", call.function)))?;
    let signature_ok = function.index != 0
        && function.first_statement > 0
        && function.parameter_sizes.len() == call.arguments.len()
        && function.parameter_sizes.iter().zip(call.arguments.iter()).all(|(size, value)| {
            usize::from(*size) == qc_source_value_type(value).words()
        });
    if !signature_ok {
        return Err(GuestError::callback(format!(
            "Mod callback {} has an incompatible source signature",
            call.function
        )));
    }
    let mut seen = HashSet::new();
    for global in &call.globals {
        let matches = program
            .global_named(&global.name)
            .is_some_and(|definition| definition.value_type == qc_source_value_type(&global.value));
        if !seen.insert(global.name.clone()) || !matches {
            return Err(GuestError::callback(format!(
                "Mod callback global {} is duplicated or has an incompatible type",
                global.name
            )));
        }
    }
    Ok(())
}

/// Actor-to-reference lowering for source values.
pub type QcActorReference = Rc<dyn Fn(Option<&ActorId>) -> i32>;

/// Write one resolved value to a global word.
pub fn write_qc_source_value(
    machine: &mut QcMachine,
    offset: usize,
    value: &ModRuntimeValue,
    reference: &QcActorReference,
) -> Result<(), GuestError> {
    match value {
        ModRuntimeValue::Float(value) => {
            if !(*value as f32).is_finite() {
                return Err(machine.fail("Mod callback number exceeds binary32 range"));
            }
            machine
                .globals_mut()
                .set_float(offset, *value as f32)
                .map_err(|error| machine.fail(error.to_string()))
        }
        ModRuntimeValue::Vector(value) => {
            if ![value.x, value.y, value.z].iter().all(|component| component.is_finite()) {
                return Err(machine.fail("Mod callback vector exceeds binary32 range"));
            }
            machine
                .globals_mut()
                .set_vector(offset, *value)
                .map_err(|error| machine.fail(error.to_string()))
        }
        ModRuntimeValue::String(value) => {
            let capacity = 128.max(value.len() + 1);
            let stored = machine
                .strings_mut()
                .set_engine(&format!("mod-value:{value}"), value, capacity)
                .map_err(|error| machine.fail(error.to_string()))?;
            machine
                .globals_mut()
                .set_int(offset, stored)
                .map_err(|error| machine.fail(error.to_string()))
        }
        ModRuntimeValue::Actor(actor) => {
            let reference = reference(actor.as_ref());
            machine
                .globals_mut()
                .set_int(offset, reference)
                .map_err(|error| machine.fail(error.to_string()))
        }
    }
}

/// Stage the declared original ABI only for this invocation (arguments at
/// words `4 + index * 3`, declared globals), run `execute`, then restore
/// the staging area and globals — including on faults.
pub fn with_qc_source_call(
    machine: &mut QcMachine,
    call: &ModSourceCall,
    inputs: &HashMap<ModCallbackInput, ModRuntimeValue>,
    reference: &QcActorReference,
    execute: impl FnOnce(&mut QcMachine, usize) -> Result<i32, GuestError>,
) -> Result<i32, GuestError> {
    let resolve = |value: &ModCallbackValue| -> Result<ModRuntimeValue, GuestError> {
        match value {
            ModCallbackValue::Float(value) => Ok(ModRuntimeValue::Float(*value)),
            ModCallbackValue::String(value) => Ok(ModRuntimeValue::String(value.clone())),
            ModCallbackValue::Vector(value) => Ok(ModRuntimeValue::Vector(*value)),
            ModCallbackValue::Input(name) => inputs.get(name).cloned().ok_or_else(|| {
                GuestError::callback(format!("Gameplay callback {} has no {} input", call.function, name.name()))
            }),
        }
    };
    let mut args = Vec::with_capacity(call.arguments.len());
    for value in &call.arguments {
        args.push(resolve(value)?);
    }
    let mut globals = Vec::with_capacity(call.globals.len());
    for global in &call.globals {
        let definition = machine
            .program()
            .global_named(&global.name)
            .map(|definition| (definition.offset, definition.value_type.words()))
            .ok_or_else(|| GuestError::callback("Missing validated callback global"))?;
        globals.push((definition, resolve(&global.value)?));
    }
    if machine.globals().bytes().len() < 112 {
        return Err(machine.fail("Missing validated callback staging area"));
    }
    let staging = machine.globals().bytes()[4..112].to_vec();
    let mut saved = Vec::with_capacity(globals.len());
    for ((offset, words), _) in &globals {
        let start = offset * 4;
        saved.push((*offset, machine.globals().bytes()[start..start + words * 4].to_vec()));
    }
    let outcome = (|| -> Result<i32, GuestError> {
        for (index, value) in args.iter().enumerate() {
            write_qc_source_value(machine, 4 + index * 3, value, reference)?;
        }
        for ((offset, _), value) in &globals {
            write_qc_source_value(machine, *offset, value, reference)?;
        }
        execute(machine, args.len())
    })();
    machine.globals_mut().bytes_mut()[4..112].copy_from_slice(&staging);
    for (offset, bytes) in &saved {
        let start = offset * 4;
        machine.globals_mut().bytes_mut()[start..start + bytes.len()].copy_from_slice(bytes);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::machine::{QcBuiltinRegistry, QcMachineOptions};
    use super::super::memory::{QcEntityLayout, QcEntityMemory};
    use super::super::program::{test_program, QcDefinition, QcFunction, QcOpcode, QcStatement, QcValueType};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::numeric::NumericOps;

    fn fixture_machine() -> QcMachine {
        let statements = vec![
            QcStatement { opcode: QcOpcode::Done, a: 0, b: 0, c: 0 },
            QcStatement { opcode: QcOpcode::Return, a: 1, b: 0, c: 0 },
        ];
        let globals = vec![
            QcDefinition { value_type: QcValueType::Entity, native_type: 4, save: false, offset: 28, name: "self".to_string() },
            QcDefinition { value_type: QcValueType::Float, native_type: 2, save: false, offset: 29, name: "damage".to_string() },
            QcDefinition { value_type: QcValueType::Vector, native_type: 3, save: false, offset: 30, name: "spot".to_string() },
        ];
        let functions = vec![
            QcFunction {
                index: 0, first_statement: 0, parameter_start: 0, local_words: 0,
                name: "<null>".to_string(), file: String::new(), parameter_sizes: vec![], named_builtin: false,
            },
            QcFunction {
                index: 1, first_statement: 1, parameter_start: 33, local_words: 0,
                name: "hurt".to_string(), file: "test.qc".to_string(), parameter_sizes: vec![1, 3],
                named_builtin: false,
            },
        ];
        let program = test_program(statements, globals, vec![], functions, b"\0".to_vec(), vec![0; 36 * 4], 1);
        let layout = QcEntityLayout { stride_bytes: 100, variables_offset_bytes: 96, field_words: 1 };
        let entities = QcEntityMemory::new(layout, 2, 1).unwrap();
        QcMachine::new(QcMachineOptions::new(
            program,
            NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap(),
            entities,
            QcBuiltinRegistry::default(),
            Rc::new(|| true),
        ))
        .unwrap()
    }

    fn hurt_call() -> ModSourceCall {
        ModSourceCall {
            function: "hurt".to_string(),
            arguments: vec![
                ModCallbackValue::Input(ModCallbackInput::Amount),
                ModCallbackValue::Input(ModCallbackInput::Point),
            ],
            globals: vec![ModSourceGlobal {
                name: "damage".to_string(),
                value: ModCallbackValue::Input(ModCallbackInput::Amount),
            }],
        }
    }

    #[test]
    fn value_types_follow_donor_table() {
        assert_eq!(qc_source_value_type(&ModCallbackValue::Input(ModCallbackInput::Self_)), QcValueType::Entity);
        assert_eq!(qc_source_value_type(&ModCallbackValue::Input(ModCallbackInput::Point)), QcValueType::Vector);
        assert_eq!(qc_source_value_type(&ModCallbackValue::Input(ModCallbackInput::Item)), QcValueType::String);
        assert_eq!(qc_source_value_type(&ModCallbackValue::Input(ModCallbackInput::Amount)), QcValueType::Float);
        assert_eq!(qc_source_value_type(&ModCallbackValue::Float(1.0)), QcValueType::Float);
    }

    #[test]
    fn validation_accepts_compatible_calls() {
        let machine = fixture_machine();
        let available: HashSet<ModCallbackInput> =
            [ModCallbackInput::Amount, ModCallbackInput::Point].into_iter().collect();
        validate_qc_source_call(machine.program(), &hurt_call(), &available, "test").unwrap();
    }

    #[test]
    fn validation_rejects_bad_calls() {
        let machine = fixture_machine();
        let available: HashSet<ModCallbackInput> = [ModCallbackInput::Amount].into_iter().collect();
        assert!(validate_qc_source_call(machine.program(), &hurt_call(), &available, "test").is_err());
        let mut wrong = hurt_call();
        wrong.arguments.pop();
        let available: HashSet<ModCallbackInput> =
            [ModCallbackInput::Amount, ModCallbackInput::Point].into_iter().collect();
        assert!(validate_qc_source_call(machine.program(), &wrong, &available, "test").is_err());
        let mut missing = hurt_call();
        missing.function = "nope".to_string();
        assert!(validate_qc_source_call(machine.program(), &missing, &available, "test").is_err());
        let mut dup = hurt_call();
        dup.globals.push(dup.globals[0].clone());
        assert!(validate_qc_source_call(machine.program(), &dup, &available, "test").is_err());
    }

    #[test]
    fn staging_restores_after_execute() {
        let mut machine = fixture_machine();
        machine.globals_mut().set_float(4, 111.0).unwrap();
        let mut inputs = HashMap::new();
        inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(9.0));
        inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vector(vec3(1.0, 2.0, 3.0)));
        let reference: QcActorReference = Rc::new(|_| 0);
        let seen = with_qc_source_call(&mut machine, &hurt_call(), &inputs, &reference, |machine, argc| {
            assert_eq!(argc, 2);
            assert_eq!(machine.globals().float(4).unwrap(), 9.0);
            assert_eq!(machine.globals().vector(7).unwrap(), vec3(1.0, 2.0, 3.0));
            assert_eq!(machine.globals().float(29).unwrap(), 9.0);
            Ok(7)
        })
        .unwrap();
        assert_eq!(seen, 7);
        assert_eq!(machine.globals().float(4).unwrap(), 111.0);
        assert_eq!(machine.globals().float(29).unwrap(), 0.0);
    }

    #[test]
    fn staging_restores_on_fault() {
        let mut machine = fixture_machine();
        machine.globals_mut().set_float(4, 5.0).unwrap();
        let mut inputs = HashMap::new();
        inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(f64::INFINITY));
        inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vector(vec3(0.0, 0.0, 0.0)));
        let reference: QcActorReference = Rc::new(|_| 0);
        let error = with_qc_source_call(&mut machine, &hurt_call(), &inputs, &reference, |_, _| Ok(0)).unwrap_err();
        assert!(error.to_string().contains("binary32"));
        assert_eq!(machine.globals().float(4).unwrap(), 5.0);
    }

    #[test]
    fn actor_and_string_values_lower() {
        let mut machine = fixture_machine();
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(2, 1);
        let reference: QcActorReference = Rc::new(|actor| actor.map_or(0, |_| 99));
        write_qc_source_value(&mut machine, 4, &ModRuntimeValue::Actor(Some(actor)), &reference).unwrap();
        assert_eq!(machine.globals().int(4).unwrap(), 99);
        write_qc_source_value(&mut machine, 4, &ModRuntimeValue::Actor(None), &reference).unwrap();
        assert_eq!(machine.globals().int(4).unwrap(), 0);
        write_qc_source_value(&mut machine, 4, &ModRuntimeValue::String("hi".to_string()), &reference).unwrap();
        let stored = machine.globals().int(4).unwrap();
        assert_eq!(machine.strings().get(stored).unwrap(), "hi");
    }
}
