//! Qualified regular-armor stages (`src/content/q1/quakec/armor-stage.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/armor-stage.ts` (`qcArmorStage`,
//! `qcStatementAccess`, `qcRegionPrivateWritesAreDead`).

use std::collections::{HashMap, HashSet};

use crate::contract::{ModQcArmorStage, ModQcDamageFlags};

use super::qc_view::{
    signed_qc_branch, InlineStandalone, QcInlineRegion, QcOpcode, QcProgramView, QcStatementView, QcValueType,
    StandaloneScope,
};
use super::QcError;

/// Armor stage plus its inline region (donor `QcArmorStage`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcArmorStage {
    /// Declared stage.
    pub stage: ModQcArmorStage,
    /// Inline region.
    pub region: QcInlineRegion,
}

/// Opcode word access (donor `qcStatementAccess` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcStatementAccess {
    /// Read words.
    pub read: Vec<usize>,
    /// Written words.
    pub write: Vec<usize>,
}

/// Word range helper (donor `words`).
fn words(first: usize, count: usize) -> Vec<usize> {
    (first..first.saturating_add(count)).collect()
}

/// Opcode access contract (donor `access`).
fn access(statement: &QcStatementView) -> QcStatementAccess {
    let opcode = statement.opcode;
    let a = usize::from(statement.a);
    let b = usize::from(statement.b);
    let c = usize::from(statement.c);
    if opcode == QcOpcode::Done || opcode == QcOpcode::Return {
        return QcStatementAccess {
            read: words(a, 3),
            write: vec![1, 2, 3],
        };
    }
    if opcode == QcOpcode::Goto {
        return QcStatementAccess {
            read: Vec::new(),
            write: Vec::new(),
        };
    }
    if opcode == QcOpcode::If || opcode == QcOpcode::IfNot {
        return QcStatementAccess {
            read: vec![a],
            write: Vec::new(),
        };
    }
    if opcode.is_call() {
        let arity = opcode.call_arity().unwrap_or(0) as usize;
        let mut read = vec![a];
        read.extend(words(4, arity * 3));
        return QcStatementAccess {
            read,
            write: vec![1, 2, 3],
        };
    }
    if opcode == QcOpcode::State {
        return QcStatementAccess {
            read: vec![a, b],
            write: Vec::new(),
        };
    }
    if opcode >= QcOpcode::StoreF && opcode <= QcOpcode::StoreFn {
        let width = usize::from(opcode == QcOpcode::StoreV) * 2 + 1;
        return QcStatementAccess {
            read: words(a, width),
            write: words(b, width),
        };
    }
    if opcode >= QcOpcode::StorePF && opcode <= QcOpcode::StorePFn {
        let width = usize::from(opcode == QcOpcode::StorePV) * 2 + 1;
        let mut read = words(a, width);
        read.push(b);
        return QcStatementAccess {
            read,
            write: Vec::new(),
        };
    }
    if opcode >= QcOpcode::NotF && opcode <= QcOpcode::NotFn {
        return QcStatementAccess {
            read: words(a, usize::from(opcode == QcOpcode::NotV) * 2 + 1),
            write: vec![c],
        };
    }
    let vector_a = matches!(
        opcode,
        QcOpcode::MulV | QcOpcode::MulVF | QcOpcode::AddV | QcOpcode::SubV | QcOpcode::EqV | QcOpcode::NeV
    );
    let vector_b = matches!(
        opcode,
        QcOpcode::MulV | QcOpcode::MulFV | QcOpcode::AddV | QcOpcode::SubV | QcOpcode::EqV | QcOpcode::NeV
    );
    let vector_result = matches!(
        opcode,
        QcOpcode::MulFV | QcOpcode::MulVF | QcOpcode::AddV | QcOpcode::SubV | QcOpcode::LoadV
    );
    let mut read = words(a, usize::from(vector_a) * 2 + 1);
    read.extend(words(b, usize::from(vector_b) * 2 + 1));
    QcStatementAccess {
        read,
        write: words(c, usize::from(vector_result) * 2 + 1),
    }
}

/// Words written anywhere in the program (donor `writtenWords` cache,
///
/// computed once per qualification instead of memoized per program).
fn written_words(program: &QcProgramView) -> HashSet<usize> {
    let mut written = HashSet::new();
    for statement in program.statements {
        written.extend(access(statement).write);
    }
    written
}

/// Callee when the call word is never written (donor `constantCallee`).
fn constant_callee(
    program: &QcProgramView,
    written: &HashSet<usize>,
    statement: &QcStatementView,
) -> Result<Option<usize>, QcError> {
    if written.contains(&usize::from(statement.a)) {
        return Ok(None);
    }
    let index = program.initial_i32(usize::from(statement.a))?;
    let function = usize::try_from(index)
        .ok()
        .and_then(|index| program.functions.get(index));
    Ok(match function {
        Some(function)
            if program
                .global_named(&function.name)
                .is_some_and(|definition| definition.offset == usize::from(statement.a)) =>
        {
            Some(function.index)
        }
        _ => None,
    })
}

/// Call access narrowed to declared parameter words (donor
/// `sourceAccess`).
fn source_access(
    program: &QcProgramView,
    written: &HashSet<usize>,
    statement: &QcStatementView,
) -> Result<QcStatementAccess, QcError> {
    let result = access(statement);
    if !statement.opcode.is_call() {
        return Ok(result);
    }
    let Some(arity) = statement.opcode.call_arity() else {
        return Ok(result);
    };
    let callee = constant_callee(program, written, statement)?.and_then(|index| program.functions.get(index));
    let Some(function) = callee else {
        return Ok(result);
    };
    if function.parameter_sizes.len() != arity as usize {
        return Ok(result);
    }
    let mut read = vec![usize::from(statement.a)];
    for (index, size) in function.parameter_sizes.iter().enumerate() {
        read.extend(words(4 + index * 3, *size));
    }
    Ok(QcStatementAccess {
        read,
        write: result.write,
    })
}

/// Pending continuation-flow state (donor `safeFlow` queue entry).
struct FlowState {
    index: i64,
    remaining: HashSet<usize>,
    limit: usize,
    called: bool,
    local_start: usize,
    local_end: usize,
}

/// Prove that dirty words cannot escape (donor `safeFlow`).
fn safe_flow(
    program: &QcProgramView,
    written: &HashSet<usize>,
    start: usize,
    limit: usize,
    dirty: HashSet<usize>,
    output: Option<usize>,
) -> Result<bool, QcError> {
    let mut ends: HashMap<i32, usize> = HashMap::new();
    let mut entered: HashMap<usize, HashSet<usize>> = HashMap::new();
    let function_end = |start: i32, ends: &mut HashMap<i32, usize>| -> usize {
        if let Some(known) = ends.get(&start) {
            return *known;
        }
        let end = program.function_end(start);
        ends.insert(start, end);
        end
    };
    let mut pending = vec![FlowState {
        index: start as i64,
        remaining: dirty,
        limit,
        called: false,
        local_start: 0,
        local_end: 0,
    }];
    let mut seen: HashMap<i64, HashSet<usize>> = HashMap::new();
    while let Some(current) = pending.pop() {
        if current.remaining.is_empty() {
            continue;
        }
        let FlowState {
            index,
            remaining,
            limit,
            called,
            local_start,
            local_end,
        } = current;
        if !called && index == limit as i64 {
            if let Some(output) = output {
                if remaining.contains(&output) {
                    return Ok(false);
                }
                continue;
            }
        }
        if index < 0 || index >= limit as i64 {
            return Ok(false);
        }
        let key = index * 2 + i64::from(called);
        let previous = seen.get(&key);
        if previous.is_some_and(|previous| remaining.iter().all(|word| previous.contains(word))) {
            continue;
        }
        let mut merged = previous.cloned().unwrap_or_default();
        merged.extend(remaining.iter().copied());
        seen.insert(key, merged);
        let Some(statement) = program.statements.get(usize::try_from(index).unwrap_or(usize::MAX)) else {
            return Ok(false);
        };
        let QcStatementAccess { read, write } = source_access(program, written, statement)?;
        let tainted = read.iter().any(|word| remaining.contains(word));
        if tainted
            && (called
                && write
                    .iter()
                    .any(|word| *word >= 28 && (*word < local_start || *word >= local_end))
                || write.is_empty()
                || statement.opcode == QcOpcode::Return
                || statement.opcode == QcOpcode::Done
                || statement.opcode.is_call())
        {
            return Ok(false);
        }
        if statement.opcode.is_call() && remaining.iter().any(|word| *word >= 28) {
            let callee = constant_callee(program, written, statement)?;
            let mut candidates = Vec::new();
            match callee.and_then(|index| program.functions.get(index)) {
                Some(function) => candidates.push(function),
                None => candidates.extend(program.functions.iter()),
            }
            for callee in candidates {
                if callee.first_statement <= 0 || callee.named_builtin {
                    continue;
                }
                let parameter_end = callee
                    .parameter_start
                    .saturating_add(callee.parameter_sizes.iter().sum::<usize>());
                let incoming: HashSet<usize> = remaining
                    .iter()
                    .copied()
                    .filter(|word| *word < callee.parameter_start || *word >= parameter_end)
                    .collect();
                let previous = entered.get(&callee.index);
                if previous.is_some_and(|previous| incoming.iter().all(|word| previous.contains(word))) {
                    continue;
                }
                let mut merged = previous.cloned().unwrap_or_default();
                merged.extend(incoming.iter().copied());
                entered.insert(callee.index, merged);
                // Callees may carry dirty frame/ABI temporaries until overwritten,
                // but cannot publish them outside their restored frame. Keep caller
                // dirtiness when traversing the join.
                pending.push(FlowState {
                    index: i64::from(callee.first_statement),
                    limit: function_end(callee.first_statement, &mut ends),
                    called: true,
                    local_start: callee.parameter_start,
                    local_end: callee.parameter_start.saturating_add(callee.local_words),
                    remaining: incoming,
                });
            }
        }
        let mut next: HashSet<usize> = remaining.iter().copied().filter(|word| !write.contains(word)).collect();
        if statement.opcode >= QcOpcode::StoreF && statement.opcode <= QcOpcode::StoreFn {
            for (offset, _) in write.iter().enumerate() {
                if remaining.contains(&(usize::from(statement.a) + offset)) {
                    next.insert(usize::from(statement.b) + offset);
                }
            }
        } else if tainted {
            next.extend(write.iter().copied());
        }
        if statement.opcode == QcOpcode::Return || statement.opcode == QcOpcode::Done {
            continue;
        }
        let successors = if statement.opcode == QcOpcode::Goto {
            vec![index + i64::from(signed_qc_branch(statement.a))]
        } else if statement.opcode == QcOpcode::If || statement.opcode == QcOpcode::IfNot {
            vec![index + 1, index + i64::from(signed_qc_branch(statement.b))]
        } else {
            vec![index + 1]
        };
        for successor in successors {
            pending.push(FlowState {
                index: successor,
                remaining: next.clone(),
                limit,
                called,
                local_start,
                local_end,
            });
        }
    }
    Ok(true)
}

/// Prove that skipping the region leaves only its declared saved word
/// live (donor `replacementSafe`).
fn replacement_safe(
    program: &QcProgramView,
    written: &HashSet<usize>,
    source: &ModQcArmorStage,
    end: usize,
) -> Result<bool, QcError> {
    let mut writes: HashSet<usize> = HashSet::new();
    let mut visiting: HashSet<usize> = HashSet::new();
    // These standard ABI builtins write only the reserved return words.
    let return_only_builtins: HashSet<i32> = [-7, -9, -12, -13, -36, -37, -38, -43, -51].into_iter().collect();
    #[allow(clippy::too_many_arguments)]
    fn collect(
        program: &QcProgramView,
        written: &HashSet<usize>,
        return_only_builtins: &HashSet<i32>,
        writes: &mut HashSet<usize>,
        visiting: &mut HashSet<usize>,
        start: usize,
        limit: usize,
        local_start: usize,
        local_end: usize,
    ) -> Result<bool, QcError> {
        for statement in program.statements.get(start..limit).unwrap_or(&[]).iter() {
            for word in access(statement).write {
                if word < local_start || word >= local_end {
                    writes.insert(word);
                }
            }
            if !statement.opcode.is_call() {
                continue;
            }
            let Some(index) = constant_callee(program, written, statement)? else {
                return Ok(false);
            };
            let Some(function) = program.functions.get(index) else {
                return Ok(false);
            };
            if function.named_builtin || function.first_statement < 0 {
                if !return_only_builtins.contains(&function.first_statement) {
                    return Ok(false);
                }
                continue;
            }
            if visiting.contains(&function.index) {
                return Ok(false);
            }
            visiting.insert(function.index);
            let stop = program.function_end(function.first_statement);
            let ok = collect(
                program,
                written,
                return_only_builtins,
                writes,
                visiting,
                usize::try_from(function.first_statement).unwrap_or(usize::MAX),
                stop,
                function.parameter_start,
                function.parameter_start.saturating_add(function.local_words),
            )?;
            visiting.remove(&function.index);
            if !ok {
                return Ok(false);
            }
        }
        // A short slice means a missing statement (donor `statement ===
        // undefined`).
        if program.statements.len() < limit {
            return Ok(false);
        }
        Ok(true)
    }
    let entry = source.entry as usize;
    let exit = source.exit as usize;
    if !collect(
        program,
        written,
        &return_only_builtins,
        &mut writes,
        &mut visiting,
        entry,
        exit,
        0,
        0,
    )? {
        return Ok(false);
    }
    writes.remove(&(source.saved as usize));
    let owner = program.function_named(&source.function)?;
    if writes.iter().any(|word| {
        *word >= 28
            && (*word < owner.parameter_start || *word >= owner.parameter_start.saturating_add(owner.local_words))
            && program
                .globals
                .iter()
                .any(|global| global.offset == *word && !global.name.is_empty())
    }) {
        return Ok(false);
    }
    safe_flow(program, written, exit, end, writes, None)
}

/// Scalar-region qualification uses the same opcode access contract as
/// continuation analysis (donor `qcStatementAccess`).
#[must_use]
pub fn qc_statement_access(statement: &QcStatementView) -> QcStatementAccess {
    access(statement)
}

/// Check that skipping qualified private temporary writes cannot affect
/// the source continuation (donor `qcRegionPrivateWritesAreDead`).
pub fn qc_region_private_writes_are_dead(
    program: &QcProgramView,
    start: usize,
    end: usize,
    writes: &HashSet<usize>,
) -> Result<bool, QcError> {
    let written = written_words(program);
    safe_flow(program, &written, start, end, writes.clone(), None)
}

/// Prove the region needs no caller frame beyond its inputs (donor
/// `standaloneSafe`).
fn standalone_safe(program: &QcProgramView, source: &ModQcArmorStage) -> Result<bool, QcError> {
    let written = written_words(program);
    let function = program.function_named(&source.function)?;
    let parameters: usize = function.parameter_sizes.iter().sum();
    let mut named: HashSet<usize> = HashSet::new();
    for global in program.globals {
        if global.name.is_empty() {
            continue;
        }
        named.extend(words(
            global.offset,
            usize::from(global.def_type == QcValueType::Vector) * 2 + 1,
        ));
    }
    let mut missing: HashSet<usize> = HashSet::new();
    missing.extend(words(
        function.parameter_start.saturating_add(parameters),
        function.local_words.saturating_sub(parameters),
    ));
    let entry = source.entry as usize;
    let exit = source.exit as usize;
    for statement in program.statements.get(entry..exit).unwrap_or(&[]) {
        let QcStatementAccess { read, .. } = source_access(program, &written, statement)?;
        missing.extend(read.into_iter().filter(|word| *word >= 28 && !named.contains(word)));
    }
    for word in words(function.parameter_start, parameters) {
        missing.remove(&word);
    }
    safe_flow(program, &written, entry, exit, missing, Some(source.saved as usize))
}

/// Source-reviewed entry, join and frame words for one original
/// regular-armor region (donor `originals`).
fn original_armor_region(digest: &str) -> Option<[u32; 5]> {
    match digest {
        "sha256:f2619787f9aa0f057246eea1665b622b4691b5c5a800b1a46133d1fe8b771580" => {
            Some([1431, 1455, 1580, 1583, 1588])
        }
        "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830" => Some([377, 401, 855, 858, 863]),
        "sha256:35a2fdc3acb04bdafe8d0269f5327cd1d5b47971f1ef024429f1572d3201dc82" => {
            Some([2008, 2032, 2061, 2064, 2069])
        }
        "sha256:f3610ade82495b6b064f3ba5894d9f40ac9406f92c5f24a1946e87138b4fdf0a" => {
            Some([5145, 5171, 3678, 3681, 3686])
        }
        "sha256:36616cd101dfdb1cf1cabfd02c9c50a6bb6b69eaff7d61000f44eeff4291be12" => {
            Some([3060, 3086, 2397, 2400, 2405])
        }
        "sha256:f9a2d64e84a6530281c016f1a5557924370fdd9a1a10eb6e785d491352ffacf0" => {
            Some([1317, 1337, 3662, 3665, 3670])
        }
        "sha256:39418aa9a7cfcccbc3c195cd757c9f6a20c0b10ba40ce74333de4277144dcb16" => {
            Some([1913, 1933, 4598, 4601, 4606])
        }
        "sha256:b828d7dd7150688e5b562cb4ab65f0ec208cebe97804d39f18fd69b7664627ac" => {
            Some([4932, 4954, 5553, 5556, 5561])
        }
        "sha256:88b440d1d73ebfcab39d94e78309ba9856012e181b6c66df2cf31018b6563a13" => {
            Some([2493, 2515, 1936, 1939, 1944])
        }
        "sha256:1f410f927c1b634f74f538becf8c1c2d04cbdfd3a74759c8fafc425b5fdcb2c0" => {
            Some([2484, 2504, 4443, 4446, 4451])
        }
        "sha256:9ed7d5be3f348f02bd009519be7161baaf41a9e8e77eb769d187afab997f3dd9" => {
            Some([2573, 2593, 6214, 6217, 6222])
        }
        _ => None,
    }
}

/// Original regular-armor scale call sites (donor `originalScales`).
fn original_regular_scales(digest: &str) -> Vec<crate::contract::ModQcRegularScale> {
    match digest {
        "sha256:f3610ade82495b6b064f3ba5894d9f40ac9406f92c5f24a1946e87138b4fdf0a" => {
            vec![crate::contract::ModQcRegularScale {
                caller: "superlavaspike_touch".to_string(),
                statement: 32005,
                scale: 0.5,
            }]
        }
        "sha256:b828d7dd7150688e5b562cb4ab65f0ec208cebe97804d39f18fd69b7664627ac" => {
            vec![crate::contract::ModQcRegularScale {
                caller: "superlavaspike_touch".to_string(),
                statement: 31488,
                scale: 0.5,
            }]
        }
        _ => Vec::new(),
    }
}

/// Qualify a regular-armor stage (donor `qcArmorStage`).
pub fn qc_armor_stage(
    program: &QcProgramView,
    declared: Option<&ModQcArmorStage>,
) -> Result<Option<QcArmorStage>, QcError> {
    let original = original_armor_region(program.digest);
    if declared.is_none() && original.is_none() {
        return Ok(None);
    }
    let owned;
    let source: &ModQcArmorStage = match (declared, original) {
        (Some(declared), _) => declared,
        (None, Some(region)) => {
            let scales = original_regular_scales(program.digest);
            let entry = region[0] as usize;
            let exit = region[1] as usize;
            owned = ModQcArmorStage {
                function: "T_Damage".to_string(),
                entry: region[0],
                exit: region[1],
                target: region[2],
                damage: region[3],
                saved: region[4],
                regular_scale: scales,
                flags: ModQcDamageFlags::None,
                statements: program
                    .statements
                    .get(entry..exit.saturating_add(1))
                    .unwrap_or(&[])
                    .iter()
                    .map(|statement| crate::contract::QcStatement {
                        opcode: statement.opcode.as_u32(),
                        a: u32::from(statement.a),
                        b: u32::from(statement.b),
                        c: u32::from(statement.c),
                    })
                    .collect(),
            };
            &owned
        }
        (None, None) => {
            return Err(QcError::program("Missing declared QC armor stage", program.source));
        }
    };
    let reject =
        |reason: &str| -> QcError { QcError::program(format!("Unsupported QC armor stage: {reason}"), program.source) };
    let function = program.function_named(&source.function)?;
    let end = program.function_end(function.first_statement);
    let entry = source.entry as usize;
    let exit = source.exit as usize;
    if function.named_builtin
        || function.first_statement <= 0
        || entry < usize::try_from(function.first_statement).unwrap_or(usize::MAX)
        || exit <= entry
        || exit >= end
    {
        return Err(reject("region bounds"));
    }
    let frame_end = function.parameter_start.saturating_add(function.local_words);
    let local = |word: u32, expected: QcValueType| -> Result<(), QcError> {
        let word = word as usize;
        if word < function.parameter_start
            || word >= frame_end
            || !program
                .globals
                .iter()
                .any(|global| global.offset == word && global.def_type == expected)
        {
            return Err(reject("typed frame words"));
        }
        Ok(())
    };
    local(source.target, QcValueType::Entity)?;
    local(source.damage, QcValueType::Float)?;
    local(source.saved, QcValueType::Float)?;
    let mut scale_sites: HashSet<u32> = HashSet::new();
    for scale in &source.regular_scale {
        let caller = program.function_named(&scale.caller)?;
        let statement_index = scale.statement as usize;
        let statement = program.statements.get(statement_index);
        let caller_end = program.function_end(caller.first_statement);
        let damage_offset = program.global_named("T_Damage").map(|global| global.offset);
        if statement_index < usize::try_from(caller.first_statement).unwrap_or(usize::MAX)
            || statement_index >= caller_end
            || statement.is_none_or(|statement| statement.opcode != QcOpcode::Call4)
            || statement.is_none_or(|statement| damage_offset != Some(usize::from(statement.a)))
            || !scale.scale.is_finite()
            || scale.scale < 0.0
            || scale_sites.contains(&scale.statement)
        {
            return Err(reject("original regular armor scale call site"));
        }
        scale_sites.insert(scale.statement);
    }
    if HashSet::from([source.target, source.damage, source.saved]).len() != 3 {
        return Err(reject("overlapping frame words"));
    }
    if let ModQcDamageFlags::Bits {
        word,
        no_armor,
        no_power_armor,
        no_regular_armor,
        energy,
    } = source.flags
    {
        local(word, QcValueType::Float)?;
        if [no_armor, no_power_armor, no_regular_armor, energy]
            .iter()
            .any(|mask| *mask > 0x7f_ffff)
        {
            return Err(reject("source flag masks"));
        }
    }
    if source.statements.len() != exit.saturating_sub(entry).saturating_add(1) {
        return Err(reject("incomplete instruction declaration"));
    }
    for (offset, expected) in source.statements.iter().enumerate() {
        let actual = program.statements.get(entry + offset);
        if actual.is_none_or(|actual| {
            actual.opcode.as_u32() != expected.opcode
                || u32::from(actual.a) != expected.a
                || u32::from(actual.b) != expected.b
                || u32::from(actual.c) != expected.c
        }) {
            return Err(reject("instruction declaration differs from artifact"));
        }
    }
    let first = program.statements.get(entry);
    let join = program.statements.get(exit);
    let field = program.global_named("armortype");
    let target = source.target as usize;
    let damage = source.damage as usize;
    let saved = source.saved as usize;
    let entry_ok = first.is_some_and(|first| {
        usize::from(first.a) == target
            && (first.opcode == QcOpcode::LoadF && field.is_some_and(|field| usize::from(first.b) == field.offset)
                || (first.opcode == QcOpcode::StoreF || first.opcode == QcOpcode::StoreEnt) && first.b == 4)
    });
    if !entry_ok {
        return Err(reject("entry must precede regular armor reads or call staging"));
    }
    if join.is_none_or(|join| {
        join.opcode != QcOpcode::SubF || usize::from(join.a) != damage || usize::from(join.b) != saved
    }) {
        return Err(reject("damage-minus-savings join"));
    }
    let mut pending = vec![entry as i64];
    let mut visited: HashSet<i64> = HashSet::new();
    while let Some(index) = pending.pop() {
        if index == exit as i64 || visited.contains(&index) {
            continue;
        }
        if index < entry as i64 || index > exit as i64 {
            return Err(reject("control flow leaves regular armor region"));
        }
        visited.insert(index);
        let Some(statement) = program.statements.get(usize::try_from(index).unwrap_or(usize::MAX)) else {
            return Err(reject("missing statement"));
        };
        let opcode = statement.opcode;
        if opcode == QcOpcode::Done || opcode == QcOpcode::Return || opcode == QcOpcode::State {
            return Err(reject("region exits or changes actor state"));
        }
        let destination = if opcode >= QcOpcode::StoreF && opcode <= QcOpcode::StoreFn {
            i64::from(statement.b)
        } else if opcode >= QcOpcode::MulF && opcode <= QcOpcode::Address
            || opcode >= QcOpcode::NotF && opcode <= QcOpcode::NotFn
            || opcode >= QcOpcode::And
        {
            i64::from(statement.c)
        } else {
            -1
        };
        let width = i64::from(matches!(
            opcode,
            QcOpcode::StoreV | QcOpcode::MulFV | QcOpcode::MulVF | QcOpcode::AddV | QcOpcode::SubV | QcOpcode::LoadV
        )) * 2
            + 1;
        if damage as i64 >= destination && (damage as i64) < destination + width
            || target as i64 >= destination && (target as i64) < destination + width
        {
            return Err(reject("region changes captured actor or damage local"));
        }
        let successors = if opcode == QcOpcode::Goto {
            vec![index + i64::from(signed_qc_branch(statement.a))]
        } else if opcode == QcOpcode::If || opcode == QcOpcode::IfNot {
            vec![index + 1, index + i64::from(signed_qc_branch(statement.b))]
        } else {
            vec![index + 1]
        };
        if successors.iter().any(|next| *next <= index) {
            return Err(reject("looping armor region"));
        }
        pending.extend(successors);
    }
    let written = written_words(program);
    let replaceable = replacement_safe(program, &written, source, end)?;
    let standalone = if standalone_safe(program, source)? {
        Some(InlineStandalone {
            saved,
            scope: StandaloneScope::Frame,
        })
    } else {
        None
    };
    Ok(Some(QcArmorStage {
        stage: source.clone(),
        region: QcInlineRegion {
            function_index: function.index,
            entry,
            exit,
            replaceable,
            standalone: if replaceable { standalone } else { None },
        },
    }))
}
