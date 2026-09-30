//! Weapon stages (`src/content/q1/quakec/weapon-stage.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/weapon-stage.ts`
//! (`qcWeaponStage`, `invokeQcClientStage`, `qcClientStageSelf`,
//! `qcDeclaredWeaponStage`, `QcWeaponStageBinding`).

use std::collections::{HashMap, HashSet};

use qa_core::identity::ActorId;

use crate::contract::{ModCallbackInput, ModRuntimeValue, ModSourceCall, QcWeaponStageDeclaration};

use super::qc_view::{
    signed_qc_branch, validate_qc_source_call, with_qc_source_call, MachineFn, QcFunctionBoundary, QcFunctionView,
    QcInlineBoundary, QcInlineRegion, QcMachineView, QcOpcode, QcProgramView, QcValueType,
};
use super::weapon_stage_declaration::{QcClientStageTarget, QcPrimaryObjectives, QcPrimaryWeaponStageDeclaration};
use super::QcError;

/// Client stage call (donor `QcClientStageCall`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcClientStageCall {
    /// Function index.
    pub function_index: usize,
    /// Source call.
    pub call: ModSourceCall,
}

/// Weapon stage objectives (donor `client.objectives`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcWeaponObjectives {
    /// No objectives.
    None,
    /// Objective call.
    Call(QcClientStageCall),
}

/// Qualified weapon stage client (donor `QcWeaponStage.client`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponStageClient {
    /// Client spawn stage.
    pub spawn: QcClientStageCall,
    /// Spawn selection stage.
    pub select_spawn: QcClientStageCall,
    /// Objectives.
    pub objectives: QcWeaponObjectives,
}

/// Weapon stage repeat gate (donor `RepeatGate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcWeaponStageRepeat {
    /// Inline region.
    pub region: QcInlineRegion,
    /// Released result word.
    pub released: usize,
    /// Released value.
    pub value: u8,
}

/// Qualified weapon stage (donor `QcWeaponStage`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponStage {
    /// Dispatcher function index.
    pub dispatcher: usize,
    /// Client stages.
    pub client: Option<QcWeaponStageClient>,
    /// Continuation function indices.
    pub continuations: HashSet<usize>,
    /// Repeat gates.
    pub repeats: Vec<QcWeaponStageRepeat>,
}

/// Artifact-qualified weapons.qc dispatchers and player.qc next-shot
/// release branches (donor `qcWeaponStage`).
pub fn qc_weapon_stage(
    program: &QcProgramView,
    declared: Option<&QcPrimaryWeaponStageDeclaration>,
) -> Result<Option<QcWeaponStage>, QcError> {
    if let Some(declared) = declared {
        let stage = qc_declared_weapon_stage(program, &declared.stage())?;
        let source = |declaration: &QcClientStageTarget, result: &str| -> Result<QcClientStageCall, QcError> {
            let call = match declaration {
                QcClientStageTarget::Function(name) => ModSourceCall {
                    function: name.clone(),
                    arguments: Vec::new(),
                    globals: Vec::new(),
                },
                QcClientStageTarget::Call(call) => call.clone(),
            };
            let available: HashSet<ModCallbackInput> =
                [ModCallbackInput::Slf, ModCallbackInput::Time].into_iter().collect();
            validate_qc_source_call(program, &call, &available, "client stage")?;
            let function = program.function_named(&call.function)?;
            if function.first_statement <= 0 || function.named_builtin {
                return Err(QcError::program(
                    "QC client stage requires an original source function",
                    program.source,
                ));
            }
            let end = program.function_end(function.first_statement);
            let first = usize::try_from(function.first_statement).unwrap_or(usize::MAX);
            let returns: Vec<_> = program
                .statements
                .get(first..end)
                .unwrap_or(&[])
                .iter()
                .filter(|statement| {
                    (statement.opcode == QcOpcode::Return || statement.opcode == QcOpcode::Done) && statement.a != 0
                })
                .collect();
            let bad = if result == "void" {
                !returns.is_empty()
            } else {
                returns.is_empty()
                    || returns.iter().any(|statement| {
                        !program.globals.iter().any(|global| {
                            global.offset == usize::from(statement.a) && global.def_type == QcValueType::Entity
                        })
                    })
            };
            if bad {
                return Err(QcError::program(
                    format!("QC client stage {} does not return {result}", call.function),
                    program.source,
                ));
            }
            Ok(QcClientStageCall {
                function_index: function.index,
                call,
            })
        };
        return Ok(Some(QcWeaponStage {
            dispatcher: stage.dispatcher,
            continuations: stage.continuations,
            repeats: stage.repeats,
            client: Some(QcWeaponStageClient {
                spawn: source(&declared.client.spawn, "void")?,
                select_spawn: source(&declared.client.select_spawn, "entity")?,
                objectives: match &declared.client.objectives {
                    QcPrimaryObjectives::None => QcWeaponObjectives::None,
                    QcPrimaryObjectives::Function(name) => {
                        QcWeaponObjectives::Call(source(&QcClientStageTarget::Function(name.clone()), "void")?)
                    }
                    QcPrimaryObjectives::Call(call) => {
                        QcWeaponObjectives::Call(source(&QcClientStageTarget::Call(call.clone()), "void")?)
                    }
                },
            }),
        }));
    }
    let qw = program.digest == "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830";
    if !qw && program.digest != "sha256:f2619787f9aa0f057246eea1665b622b4691b5c5a800b1a46133d1fe8b771580" {
        return Ok(None);
    }
    let check_function = |name: &str, index: usize, first: i32| -> Result<usize, QcError> {
        let original = program.function_named(name)?;
        if original.index != index
            || original.first_statement != first
            || original.local_words != 0
            || !original.parameter_sizes.is_empty()
            || original.named_builtin
        {
            return Err(QcError::program(
                format!("Unsupported original weapon stage {name}"),
                program.source,
            ));
        }
        Ok(index)
    };
    let check_statement = |index: usize, opcode: QcOpcode, a: u16, b: u16| -> Result<(), QcError> {
        let actual = program.statements.get(index);
        if actual.is_none_or(|actual| actual.opcode != opcode || actual.a != a || actual.b != b) {
            return Err(QcError::program(
                format!("Original weapon stage statement {index} differs from its qualified artifact"),
                program.source,
            ));
        }
        Ok(())
    };
    let client_function = |name: &str,
                           index: usize,
                           first: i32,
                           parameters: usize,
                           locals: usize|
     -> Result<QcClientStageCall, QcError> {
        let original = program.function_named(name)?;
        if original.index != index
            || original.first_statement != first
            || original.parameter_start != parameters
            || original.local_words != locals
            || !original.parameter_sizes.is_empty()
            || original.named_builtin
        {
            return Err(QcError::program(
                format!("Unsupported original client stage {name}"),
                program.source,
            ));
        }
        Ok(QcClientStageCall {
            function_index: index,
            call: ModSourceCall {
                function: name.to_string(),
                arguments: Vec::new(),
                globals: Vec::new(),
            },
        })
    };
    if qw {
        let client = QcWeaponStageClient {
            spawn: client_function("PutClientInServer", 193, 5554, 3621, 2)?,
            select_spawn: client_function("SelectSpawnPoint", 191, 5465, 3585, 9)?,
            objectives: QcWeaponObjectives::None,
        };
        let dispatcher = check_function("W_WeaponFrame", 170, 4618)?;
        check_function("player_run", 215, 7185)?;
        let mut continuations = HashSet::new();
        let families: &[(&str, usize, &[usize])] = &[
            ("shot", 217, &[7237, 7242, 7246, 7250, 7254, 7258]),
            ("axe", 223, &[7262, 7266, 7270, 7275]),
            ("axeb", 227, &[7279, 7283, 7287, 7292]),
            ("axec", 231, &[7296, 7300, 7304, 7309]),
            ("axed", 235, &[7313, 7317, 7321, 7326]),
            ("nail", 239, &[7330, 7356]),
            ("light", 241, &[7382, 7405]),
            ("rocket", 243, &[7428, 7433, 7437, 7441, 7445, 7449]),
        ];
        for (family, first_index, starts) in families {
            for (ordinal, first) in starts.iter().enumerate() {
                continuations.insert(check_function(
                    &format!("player_{family}{}", ordinal + 1),
                    first_index + ordinal,
                    *first as i32,
                )?);
            }
        }
        let mut repeats = Vec::new();
        for (function_index, entry, held, impulse) in [
            (239, 7332, 4574, true),
            (240, 7358, 4588, true),
            (241, 7384, 4603, false),
            (242, 7407, 4615, false),
        ] {
            let exact = |offset: usize, opcode: QcOpcode, a: u16, b: u16, c: u16| -> Result<(), QcError> {
                check_statement(entry + offset, opcode, a, b)?;
                if program
                    .statements
                    .get(entry + offset)
                    .is_none_or(|actual| actual.c != c)
                {
                    return Err(QcError::program("QW weapon release temporary differs", program.source));
                }
                Ok(())
            };
            exact(0, QcOpcode::LoadF, 28, 165, held)?;
            exact(1, QcOpcode::NotF, held, 0, held + 1)?;
            exact(2, QcOpcode::Or, held + 1, 3457, held + 2)?;
            if impulse {
                exact(3, QcOpcode::LoadF, 28, 168, held + 3)?;
                exact(4, QcOpcode::Or, held + 2, held + 3, held + 4)?;
            }
            let exit = entry + if impulse { 5 } else { 3 };
            let released = held + if impulse { 4 } else { 2 };
            check_statement(exit, QcOpcode::IfNot, released, 3)?;
            check_statement(exit + 1, QcOpcode::Call0, 2112, 0)?;
            check_statement(exit + 2, QcOpcode::Return, 0, 0)?;
            repeats.push(QcWeaponStageRepeat {
                region: QcInlineRegion {
                    function_index,
                    entry,
                    exit,
                    replaceable: true,
                    standalone: None,
                },
                released: released as usize,
                value: 1,
            });
        }
        return Ok(Some(QcWeaponStage {
            dispatcher,
            client: Some(client),
            continuations,
            repeats,
        }));
    }
    let client = QcWeaponStageClient {
        spawn: client_function("PutClientInServer", 229, 6083, 4114, 1)?,
        select_spawn: client_function("SelectSpawnPoint", 228, 6007, 4093, 3)?,
        objectives: QcWeaponObjectives::None,
    };
    let dispatcher = check_function("W_WeaponFrame", 206, 5047)?;
    check_function("player_run", 248, 7380)?;
    let mut continuations = HashSet::new();
    let families: &[(&str, usize, &[usize])] = &[
        ("shot", 249, &[7421, 7429, 7433, 7437, 7441, 7445]),
        ("axe", 255, &[7449, 7453, 7457, 7462]),
        ("axeb", 259, &[7466, 7470, 7474, 7479]),
        ("axec", 263, &[7483, 7487, 7491, 7496]),
        ("axed", 267, &[7500, 7504, 7508, 7513]),
        ("nail", 271, &[7517, 7543]),
        ("light", 273, &[7569, 7594]),
        ("rocket", 275, &[7619, 7627, 7631, 7635, 7639, 7643]),
    ];
    for (family, first_index, starts) in families {
        for (ordinal, first) in starts.iter().enumerate() {
            continuations.insert(check_function(
                &format!("player_{family}{}", ordinal + 1),
                first_index + ordinal,
                *first as i32,
            )?);
        }
    }
    let mut repeats = Vec::new();
    for (function_index, entry, held) in [
        (271, 7522, 4951),
        (272, 7548, 4965),
        (273, 7574, 4980),
        (274, 7599, 4994),
    ] {
        check_statement(entry, QcOpcode::LoadF, 28, 170)?;
        if program.statements.get(entry).is_none_or(|actual| actual.c != held) {
            return Err(QcError::program(
                "Original weapon release temporary differs",
                program.source,
            ));
        }
        check_statement(entry + 1, QcOpcode::NotF, held, 0)?;
        if program
            .statements
            .get(entry + 1)
            .is_none_or(|actual| actual.c != held + 1)
        {
            return Err(QcError::program(
                "Original weapon release result differs",
                program.source,
            ));
        }
        check_statement(entry + 2, QcOpcode::IfNot, held + 1, 3)?;
        check_statement(entry + 3, QcOpcode::Call0, 2597, 0)?;
        check_statement(entry + 4, QcOpcode::Return, 0, 0)?;
        repeats.push(QcWeaponStageRepeat {
            region: QcInlineRegion {
                function_index,
                entry,
                exit: entry + 2,
                replaceable: true,
                standalone: None,
            },
            released: (held + 1) as usize,
            value: 1,
        });
    }
    Ok(Some(QcWeaponStage {
        dispatcher,
        client: Some(client),
        continuations,
        repeats,
    }))
}

/// Invoke the declared source ABI inside the original engine client
/// context (donor `invokeQcClientStage`).
pub fn invoke_qc_client_stage(
    machine: &dyn QcMachineView,
    stage: &QcClientStageCall,
    actor: &ActorId,
    time: f64,
    reference: &dyn Fn(Option<&ActorId>) -> Result<i32, QcError>,
) -> Result<i32, QcError> {
    let self_offset = machine.global_offset("self")?;
    let other_offset = machine.global_offset("other")?;
    let saved_self = machine.global_int(self_offset)?;
    let saved_other = machine.global_int(other_offset)?;
    let mut inputs = HashMap::new();
    inputs.insert(ModCallbackInput::Slf, ModRuntimeValue::Actor(Some(actor.clone())));
    inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(time));
    let result = (|| -> Result<i32, QcError> {
        machine.set_global_int(self_offset, reference(Some(actor))?)?;
        machine.set_global_int(other_offset, reference(None)?)?;
        machine.set_global_float(machine.global_offset("time")?, time)?;
        with_qc_source_call(machine, &stage.call, &inputs, reference, &mut |count| {
            machine.execute(stage.function_index, count)?;
            machine.global_int(1)
        })
    })();
    machine.set_global_int(self_offset, saved_self)?;
    machine.set_global_int(other_offset, saved_other)?;
    result
}

/// Original source callers may pass the client explicitly instead of
/// using global self (donor `qcClientStageSelf`).
pub fn qc_client_stage_self(machine: &dyn QcMachineView, stage: &QcClientStageCall) -> Result<i32, QcError> {
    let mut client: Option<i32> = None;
    let mut admit = |reference: i32| -> Result<(), QcError> {
        if client.is_some_and(|client| client != reference) {
            return Err(QcError::program(
                "QC client call has conflicting source self inputs",
                machine.program_source(),
            ));
        }
        client = Some(reference);
        Ok(())
    };
    for (index, value) in stage.call.arguments.iter().enumerate() {
        if matches!(value, crate::contract::ModCallbackValue::Input(ModCallbackInput::Slf)) {
            admit(machine.arg_int(index)?)?;
        }
    }
    for global in &stage.call.globals {
        if matches!(
            global.value,
            crate::contract::ModCallbackValue::Input(ModCallbackInput::Slf)
        ) {
            admit(machine.global_int(machine.global_offset(&global.name)?)?)?;
        }
    }
    match client {
        Some(client) => Ok(client),
        None => machine.global_int(machine.global_offset("self")?),
    }
}

/// Qualify a declared weapon stage (donor `qcDeclaredWeaponStage`).
pub fn qc_declared_weapon_stage(
    program: &QcProgramView,
    declared: &QcWeaponStageDeclaration,
) -> Result<QcWeaponStage, QcError> {
    let check_function = |name: &str| -> Result<QcFunctionView, QcError> {
        let value = program.function_named(name)?;
        if value.first_statement <= 0 || value.named_builtin || !value.parameter_sizes.is_empty() {
            return Err(QcError::program(
                "QC weapon stage requires original parameterless source functions",
                program.source,
            ));
        }
        let end = program.function_end(value.first_statement);
        let first = usize::try_from(value.first_statement).unwrap_or(usize::MAX);
        if program
            .statements
            .get(first..end)
            .unwrap_or(&[])
            .iter()
            .any(|statement| {
                (statement.opcode == QcOpcode::Return || statement.opcode == QcOpcode::Done) && statement.a != 0
            })
        {
            return Err(QcError::program(
                "QC weapon stage function returns a source value",
                program.source,
            ));
        }
        Ok(value.clone())
    };
    let dispatcher = check_function(&declared.dispatcher)?;
    let mut continuations = HashSet::new();
    for name in &declared.continuations {
        continuations.insert(check_function(name)?.index);
    }
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    let mut named: HashSet<usize> = HashSet::new();
    for global in program
        .globals
        .iter()
        .filter(|global| !global.name.is_empty() && global.name != "IMMEDIATE")
    {
        for index in 0..usize::from(global.def_type == QcValueType::Vector) * 2 + 1 {
            named.insert(global.offset + index);
        }
    }
    let temporary = |word: u32, owner: &QcFunctionView| -> bool {
        u64::from(word) >= 28
            && u64::from(word) * 4 < program.initial_globals.len() as u64
            && (!named.contains(&(word as usize))
                || word as usize >= owner.parameter_start
                    && (word as usize) < owner.parameter_start.saturating_add(owner.local_words))
    };
    if continuations.is_empty()
        || continuations.len() != declared.continuations.len()
        || continuations.contains(&dispatcher.index)
    {
        return Err(QcError::program(
            "QC weapon continuation declarations overlap or are empty",
            program.source,
        ));
    }
    let pure = |opcode: QcOpcode| -> bool {
        matches!(
            opcode,
            QcOpcode::LoadF
                | QcOpcode::NotF
                | QcOpcode::EqF
                | QcOpcode::NeF
                | QcOpcode::Le
                | QcOpcode::Ge
                | QcOpcode::Lt
                | QcOpcode::Gt
                | QcOpcode::And
                | QcOpcode::Or
                | QcOpcode::BitAnd
                | QcOpcode::BitOr
                | QcOpcode::AddF
                | QcOpcode::SubF
                | QcOpcode::MulF
                | QcOpcode::DivF
        )
    };
    let mut repeats = Vec::with_capacity(declared.repeats.len());
    for source in &declared.repeats {
        let owner = check_function(&source.function)?;
        let end = program.function_end(owner.first_statement);
        let first = usize::try_from(owner.first_statement).unwrap_or(usize::MAX);
        if !continuations.contains(&owner.index)
            || ranges
                .iter()
                .any(|(entry, exit)| source.entry <= *exit && source.exit >= *entry)
            || (source.entry as usize) < first
            || source.exit <= source.entry
            || (source.exit as usize).saturating_add(2) >= end
            || !temporary(source.result.word, &owner)
            || source.result.value > 1
            || source.statements.len() != source.exit.saturating_sub(source.entry).saturating_add(1) as usize
        {
            return Err(QcError::program(
                "QC weapon repeat boundary is outside its original continuation",
                program.source,
            ));
        }
        ranges.push((source.entry, source.exit));
        let mut result_written = false;
        for (offset, expected) in source.statements.iter().enumerate() {
            let actual = program.statements.get(source.entry as usize + offset);
            let Some(actual) = actual else {
                return Err(QcError::program(
                    "QC weapon repeat boundary differs from its declared original instructions",
                    program.source,
                ));
            };
            if actual.opcode.as_u32() != expected.opcode
                || u32::from(actual.a) != expected.a
                || u32::from(actual.b) != expected.b
                || u32::from(actual.c) != expected.c
            {
                return Err(QcError::program(
                    "QC weapon repeat boundary differs from its declared original instructions",
                    program.source,
                ));
            }
            if offset == source.statements.len() - 1 {
                continue;
            }
            if !pure(actual.opcode) || !temporary(u32::from(actual.c), &owner) {
                return Err(QcError::program(
                    "QC weapon repeat must be a pure scalar predicate using source temporaries",
                    program.source,
                ));
            }
            result_written = result_written || u32::from(actual.c) == source.result.word;
        }
        let join = program.statements.get(source.exit as usize);
        let release = program.statements.get(source.exit as usize + 1);
        let returned = program.statements.get(source.exit as usize + 2);
        let expected_join = if source.result.value == 0 {
            QcOpcode::If
        } else {
            QcOpcode::IfNot
        };
        if !result_written
            || join.is_none_or(|join| u32::from(join.a) != source.result.word || join.opcode != expected_join)
            || join.is_none_or(|join| signed_qc_branch(join.b) != 3)
            || release.is_none_or(|release| release.opcode != QcOpcode::Call0)
            || returned.is_none_or(|returned| returned.opcode != QcOpcode::Return || returned.a != 0)
        {
            return Err(QcError::program(
                "QC weapon predicate does not join its original release-call and return branch",
                program.source,
            ));
        }
        let release_a = u32::from(release.map(|release| release.a).unwrap_or(0));
        if u64::from(release_a) * 4 + 4 > program.initial_globals.len() as u64 {
            return Err(QcError::program(
                "QC weapon release call is outside source globals",
                program.source,
            ));
        }
        let callee = program.initial_i32(release_a as usize)?;
        let callee = usize::try_from(callee).map_err(|_| QcError::program("invalid function", program.source))?;
        check_function(&program.function_at(callee)?.name.clone())?;
        repeats.push(QcWeaponStageRepeat {
            region: QcInlineRegion {
                function_index: owner.index,
                entry: source.entry as usize,
                exit: source.exit as usize,
                replaceable: true,
                standalone: None,
            },
            released: source.result.word as usize,
            value: source.result.value,
        });
    }
    Ok(QcWeaponStage {
        dispatcher: dispatcher.index,
        client: None,
        continuations,
        repeats,
    })
}

/// Selection gates new attacks; committed melee hits and source
/// animation finish normally (donor `QcWeaponStageBinding`).
pub struct QcWeaponStageBinding<'a> {
    /// Qualified stage.
    pub stage: QcWeaponStage,
    machine: MachineFn<'a>,
    selected: Box<dyn Fn(i32) -> bool + 'a>,
}

impl<'a> QcWeaponStageBinding<'a> {
    /// Bind a weapon stage (donor `QcWeaponStageBinding` constructor).
    pub fn new(stage: QcWeaponStage, machine: MachineFn<'a>, selected: Box<dyn Fn(i32) -> bool + 'a>) -> Self {
        Self {
            stage,
            machine,
            selected,
        }
    }

    /// Whether the actor left staged continuations (donor `settled`).
    pub fn settled(&self, reference: i32) -> Result<bool, QcError> {
        let vm = (self.machine)();
        let think = vm.entity_int(vm.entity_slot(reference)?, vm.field_offset("think")?)?;
        Ok(!self
            .stage
            .continuations
            .contains(&usize::try_from(think).unwrap_or(usize::MAX)))
    }

    /// Compose function boundaries (donor `composeFunctions`).
    pub fn compose_functions(&'a self, inner: QcFunctionBoundary<'a>) -> Result<QcFunctionBoundary<'a>, QcError> {
        if inner.functions.contains(&self.stage.dispatcher) {
            return Err(QcError::program(
                "Original weapon dispatcher already has a boundary owner",
                "progs.dat",
            ));
        }
        let mut functions = inner.functions.clone();
        functions.insert(self.stage.dispatcher);
        Ok(QcFunctionBoundary {
            functions,
            run: Box::new(move |call, execute| {
                if call.function_index != self.stage.dispatcher {
                    return (inner.run)(call, execute);
                }
                let vm = (self.machine)();
                if (self.selected)(vm.global_int(vm.global_offset("self")?)?) {
                    execute.run(None)
                } else {
                    execute.skip([0, 0, 0]);
                    Ok(())
                }
            }),
        })
    }

    /// Compose region boundaries (donor `composeRegions`).
    pub fn compose_regions(&'a self, inner: QcInlineBoundary<'a>) -> Result<QcInlineBoundary<'a>, QcError> {
        if self
            .stage
            .repeats
            .iter()
            .any(|gate| inner.regions.iter().any(|region| region.entry == gate.region.entry))
        {
            return Err(QcError::program(
                "Original weapon repeat gate already has a boundary owner",
                "progs.dat",
            ));
        }
        let mut regions = inner.regions.clone();
        regions.extend(self.stage.repeats.iter().map(|gate| gate.region));
        Ok(QcInlineBoundary {
            regions,
            run: Box::new(move |region, execute| {
                let gate = self.stage.repeats.iter().find(|gate| gate.region.entry == region.entry);
                let Some(gate) = gate else {
                    return (inner.run)(region, execute);
                };
                let vm = (self.machine)();
                if (self.selected)(vm.global_int(vm.global_offset("self")?)?) {
                    return execute.run();
                }
                execute.run()?;
                vm.set_global_float(gate.released, f64::from(gate.value))?;
                Ok(())
            }),
        })
    }
}
