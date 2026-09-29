//! Original-function region qualification for hooks and evaluation.
//!
//! Port of `src/compat/qvm/regions.ts` (`qualifyQvmRegion`,
//! `qualifyQvmRegionEvaluation`). A region is a forward instruction slice of one
//! original function with no escaping or backward edge, no live operands at
//! either boundary, and no incoming interior edge. Standalone evaluation adds a
//! scalar live-in proof: locals must be declared before they are read, local
//! pointers must not escape, and read-only evaluations cannot call, publish
//! arguments, copy memory, break, or write outside their own frame.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::GuestError;

use super::image::{QvmInstruction, QvmOpcode, QvmOperand};

/// Standalone region: entry/join instruction indices, local-frame live-in
/// offsets, and an optional result local offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmRegionEvaluation {
    /// Entry instruction index (inclusive).
    pub entry: usize,
    /// Join instruction index (exclusive).
    pub join: usize,
    /// Declared live-in local offsets.
    pub inputs: Vec<usize>,
    /// Result local offset, or `None` for no result.
    pub result: Option<usize>,
}

/// Access granted to a standalone region evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmRegionAccess {
    /// Full source access.
    Source,
    /// No calls, argument publication, memory copies, breaks, or writes
    /// outside the region's own local frame.
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operand {
    Local(usize),
    LocalDerived,
    Constant(i32),
    Unknown,
}

fn is_local(value: &Operand) -> bool {
    matches!(value, Operand::Local(_) | Operand::LocalDerived)
}

#[derive(Debug, Clone)]
struct Path {
    stack: Vec<Operand>,
    initialized: BTreeSet<usize>,
}

/// Check scalar live-ins of a standalone source frame; arguments retain their
/// original caller ABI. Returns the owning frame size.
pub fn qualify_qvm_region_evaluation(
    instructions: &[QvmInstruction],
    owner: usize,
    region: &QvmRegionEvaluation,
    access: QvmRegionAccess,
) -> Result<i32, GuestError> {
    let frame = qualify_qvm_region(instructions, owner, region.entry, region.join)?;
    let frame_size = usize::try_from(frame)
        .map_err(|_| GuestError::invalid("QVM region frame exceeds its address range"))?;
    let valid = |offset: usize| offset >= 8 && offset % 4 == 0 && offset + 4 <= frame_size;
    if region.inputs.iter().any(|offset| !valid(*offset))
        || BTreeSet::from_iter(region.inputs.iter().copied()).len() != region.inputs.len()
        || region.result.is_some_and(|offset| !valid(offset))
    {
        return Err(GuestError::invalid(
            "QVM region live-in or result is outside its source frame",
        ));
    }
    let mut pending: BTreeMap<usize, Path> = BTreeMap::new();
    pending.insert(
        region.entry,
        Path {
            stack: Vec::new(),
            initialized: region.inputs.iter().copied().collect(),
        },
    );
    while let Some(entry) = pending.iter().next().map(|(pc, _)| *pc) {
        let path = pending.remove(&entry).ok_or_else(|| GuestError::invalid("Missing QVM region input path"))?;
        if entry == region.join {
            if let Some(result) = region.result {
                if !path.initialized.contains(&result) {
                    return Err(GuestError::invalid(
                        "QVM region result is not initialized on every source path",
                    ));
                }
            }
            continue;
        }
        let instruction = instructions.get(entry).ok_or_else(|| GuestError::invalid("Missing QVM region instruction"))?;
        let mut stack = path.stack;
        let mut initialized = path.initialized;
        let opcode = instruction.opcode;
        if access == QvmRegionAccess::ReadOnly
            && matches!(
                opcode,
                QvmOpcode::OpCall | QvmOpcode::OpArg | QvmOpcode::OpBlockCopy | QvmOpcode::OpBreak
            )
        {
            return Err(GuestError::invalid(
                "Read-only QVM region cannot call, publish arguments, copy memory or break",
            ));
        }
        let mut pop = |stack: &mut Vec<Operand>| -> Result<Operand, GuestError> {
            stack.pop().ok_or_else(|| GuestError::invalid("Invalid QVM region operand proof"))
        };
        let mut jump: Option<usize> = None;
        match opcode {
            QvmOpcode::OpLocal => {
                let QvmOperand::Word(offset) = instruction.operand else {
                    return Err(GuestError::invalid("QVM region local lacks its source offset"));
                };
                // The 48 bytes past the frame cover the private argument window.
                if offset < 8 || offset % 4 != 0 || offset as i64 + 4 > frame as i64 + 48 {
                    return Err(GuestError::invalid(
                        "QVM region local address exceeds its source frame and arguments",
                    ));
                }
                stack.push(Operand::Local(offset as usize));
            }
            QvmOpcode::OpConst => {
                let QvmOperand::Word(value) = instruction.operand else {
                    return Err(GuestError::invalid("QVM region constant lacks its source value"));
                };
                stack.push(Operand::Constant(value));
            }
            QvmOpcode::OpPush => stack.push(Operand::Unknown),
            QvmOpcode::OpPop => {
                pop(&mut stack)?;
            }
            QvmOpcode::OpArg => {
                pop(&mut stack)?;
                let QvmOperand::Byte(offset) = instruction.operand else {
                    return Err(GuestError::invalid("QVM region argument lacks its source offset"));
                };
                initialized.insert(offset as usize);
            }
            QvmOpcode::OpLoad1 | QvmOpcode::OpLoad2 | QvmOpcode::OpLoad4 => {
                let address = pop(&mut stack)?;
                if address == Operand::LocalDerived {
                    return Err(GuestError::invalid(
                        "QVM region reads an unresolved source local address",
                    ));
                }
                if let Operand::Local(offset) = address {
                    if offset < frame_size && !initialized.contains(&offset) {
                        return Err(GuestError::invalid(format!(
                            "QVM region reads undeclared source local {offset}"
                        )));
                    }
                }
                stack.push(Operand::Unknown);
            }
            QvmOpcode::OpStore1 | QvmOpcode::OpStore2 | QvmOpcode::OpStore4 => {
                if is_local(&pop(&mut stack)?) {
                    return Err(GuestError::invalid(
                        "QVM region stores an escaping source local pointer",
                    ));
                }
                let address = pop(&mut stack)?;
                if access == QvmRegionAccess::ReadOnly
                    && !matches!(address, Operand::Local(offset) if offset >= 8 && offset + 4 <= frame_size)
                {
                    return Err(GuestError::invalid(
                        "Read-only QVM region cannot write outside its own local frame",
                    ));
                }
                if let Operand::Local(offset) = address {
                    if opcode == QvmOpcode::OpStore4 {
                        initialized.insert(offset);
                    }
                }
            }
            QvmOpcode::OpBlockCopy => {
                let QvmOperand::Word(count) = instruction.operand else {
                    return Err(GuestError::invalid("QVM region copy lacks its source count"));
                };
                let source = pop(&mut stack)?;
                if source == Operand::LocalDerived {
                    return Err(GuestError::invalid(
                        "QVM region copies an unresolved source local address",
                    ));
                }
                if let Operand::Local(offset) = source {
                    let mut delta = 0;
                    while delta < count {
                        if !initialized.contains(&(offset + delta as usize)) {
                            return Err(GuestError::invalid(
                                "QVM region copies an undeclared source local",
                            ));
                        }
                        delta += 4;
                    }
                }
                let address = pop(&mut stack)?;
                if let Operand::Local(offset) = address {
                    let mut delta = 0;
                    while delta + 4 <= count {
                        initialized.insert(offset + delta as usize);
                        delta += 4;
                    }
                }
            }
            QvmOpcode::OpJump => {
                let destination = pop(&mut stack)?;
                let Operand::Constant(target) = destination else {
                    return Err(GuestError::invalid("QVM region jump lost its source target"));
                };
                let target = usize::try_from(target)
                    .map_err(|_| GuestError::invalid("QVM region jump lost its source target"))?;
                jump = Some(target);
            }
            _ if opcode.is_branch() => {
                pop(&mut stack)?;
                pop(&mut stack)?;
                let QvmOperand::Word(target) = instruction.operand else {
                    return Err(GuestError::invalid("QVM region branch lacks its source target"));
                };
                let target = usize::try_from(target)
                    .map_err(|_| GuestError::invalid("QVM region branch lacks its source target"))?;
                merge(&mut pending, target, &stack, &initialized);
            }
            QvmOpcode::OpCall => {
                pop(&mut stack)?;
                stack.push(Operand::Unknown);
            }
            _ if is_binary(opcode) => {
                let right = pop(&mut stack)?;
                let left = pop(&mut stack)?;
                let folded = match (&left, &right) {
                    (Operand::Local(base), Operand::Constant(delta))
                        if matches!(opcode, QvmOpcode::OpAdd | QvmOpcode::OpSub) =>
                    {
                        let offset = if opcode == QvmOpcode::OpAdd {
                            *base as i64 + *delta as i64
                        } else {
                            *base as i64 - *delta as i64
                        };
                        Some(usize::try_from(offset).map_or(Operand::LocalDerived, Operand::Local))
                    }
                    _ => None,
                };
                stack.push(folded.unwrap_or_else(|| {
                    if is_local(&left) || is_local(&right) {
                        Operand::LocalDerived
                    } else {
                        Operand::Unknown
                    }
                }));
            }
            QvmOpcode::OpIgnore | QvmOpcode::OpBreak => {}
            _ => {
                let value = pop(&mut stack)?;
                stack.push(if is_local(&value) {
                    Operand::LocalDerived
                } else {
                    Operand::Unknown
                });
            }
        }
        if let Some(target) = jump {
            merge(&mut pending, target, &stack, &initialized);
        } else {
            merge(&mut pending, entry + 1, &stack, &initialized);
        }
    }
    Ok(frame)
}

fn merge(
    pending: &mut BTreeMap<usize, Path>,
    pc: usize,
    stack: &[Operand],
    initialized: &BTreeSet<usize>,
) {
    if let Some(previous) = pending.remove(&pc) {
        let merged_stack = stack
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let before = previous.stack.get(index);
                let same_local = matches!((value, before), (Operand::Local(a), Some(Operand::Local(b))) if a == b);
                let same_constant = matches!((value, before), (Operand::Constant(a), Some(Operand::Constant(b))) if a == b);
                if same_local || same_constant {
                    value.clone()
                } else if is_local(value) || before.is_some_and(is_local) {
                    Operand::LocalDerived
                } else {
                    Operand::Unknown
                }
            })
            .collect();
        let merged_initialized = initialized
            .iter()
            .copied()
            .filter(|offset| previous.initialized.contains(offset))
            .collect();
        pending.insert(
            pc,
            Path {
                stack: merged_stack,
                initialized: merged_initialized,
            },
        );
    } else {
        pending.insert(
            pc,
            Path {
                stack: stack.to_vec(),
                initialized: initialized.clone(),
            },
        );
    }
}

fn is_binary(opcode: QvmOpcode) -> bool {
    matches!(
        opcode,
        QvmOpcode::OpAdd
            | QvmOpcode::OpSub
            | QvmOpcode::OpDivi
            | QvmOpcode::OpDivu
            | QvmOpcode::OpModi
            | QvmOpcode::OpModu
            | QvmOpcode::OpMuli
            | QvmOpcode::OpMulu
            | QvmOpcode::OpBand
            | QvmOpcode::OpBor
            | QvmOpcode::OpBxor
            | QvmOpcode::OpLsh
            | QvmOpcode::OpRshi
            | QvmOpcode::OpRshu
            | QvmOpcode::OpAddf
            | QvmOpcode::OpSubf
            | QvmOpcode::OpDivf
            | QvmOpcode::OpMulf
    )
}

/// Qualify a forward original region with no escaping edge or live operand at
/// either boundary. Returns the owning frame size.
pub fn qualify_qvm_region(
    instructions: &[QvmInstruction],
    owner: usize,
    entry: usize,
    join: usize,
) -> Result<i32, GuestError> {
    let first = instructions.get(owner).ok_or_else(|| GuestError::invalid("QVM region owner is not a function"))?;
    if first.opcode != QvmOpcode::OpEnter {
        return Err(GuestError::invalid("QVM region owner is not a function"));
    }
    let QvmOperand::Word(frame) = first.operand else {
        return Err(GuestError::invalid("QVM region owner is not a function"));
    };
    let mut end = owner + 1;
    while end < instructions.len() && instructions[end].opcode != QvmOpcode::OpEnter {
        end += 1;
    }
    if entry <= owner || join <= entry || join >= end {
        return Err(GuestError::invalid("QVM region is outside its owning function"));
    }
    let mut depth: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pending = vec![(entry, 0usize)];
    let mut joined = false;
    while let Some((pc, count)) = pending.pop() {
        if let Some(known) = depth.get(&pc) {
            if *known != count {
                return Err(GuestError::invalid("QVM region paths disagree on operand depth"));
            }
            continue;
        }
        depth.insert(pc, count);
        if pc == join {
            if count != 0 {
                return Err(GuestError::invalid("QVM region join retains live operands"));
            }
            joined = true;
            continue;
        }
        let instruction = instructions.get(pc).ok_or_else(|| GuestError::invalid("QVM region has no original instruction"))?;
        let opcode = instruction.opcode;
        let (required, change): (usize, isize) = if matches!(
            opcode,
            QvmOpcode::OpConst | QvmOpcode::OpLocal | QvmOpcode::OpPush
        ) {
            (0, 1)
        } else if matches!(opcode, QvmOpcode::OpPop | QvmOpcode::OpArg | QvmOpcode::OpJump) {
            (1, -1)
        } else if matches!(
            opcode,
            QvmOpcode::OpStore1 | QvmOpcode::OpStore2 | QvmOpcode::OpStore4 | QvmOpcode::OpBlockCopy
        ) {
            (2, -2)
        } else if opcode.is_branch() {
            (2, -2)
        } else if matches!(opcode as u8, 38..=52) || matches!(opcode as u8, 54..=57) {
            // OP_ADD..OP_RSHU and OP_ADDF..OP_MULF; BCOM keeps its operand.
            if opcode == QvmOpcode::OpBcom {
                (1, 0)
            } else {
                (2, -1)
            }
        } else if opcode == QvmOpcode::OpCall
            || matches!(opcode as u8, 27..=29)
            || matches!(opcode as u8, 35..=37)
            || opcode == QvmOpcode::OpNegf
            || opcode == QvmOpcode::OpCvif
            || opcode == QvmOpcode::OpCvfi
        {
            (1, 0)
        } else if matches!(opcode, QvmOpcode::OpIgnore | QvmOpcode::OpBreak) {
            (0, 0)
        } else {
            return Err(GuestError::invalid(
                "QVM region contains a frame or unsupported instruction",
            ));
        };
        if count < required {
            return Err(GuestError::invalid(
                "QVM region requires operands from outside its boundary",
            ));
        }
        let result = (count as isize + change) as usize;
        let mut edge = |from: usize, target: usize, count: usize| -> Result<(), GuestError> {
            if target <= from || target > join {
                return Err(GuestError::invalid("QVM region has an escaping or backward edge"));
            }
            pending.push((target, count));
            Ok(())
        };
        if opcode == QvmOpcode::OpJump {
            if pc == 0 {
                return Err(GuestError::invalid("QVM region has an indirect jump"));
            }
            let target = &instructions[pc - 1];
            if target.opcode != QvmOpcode::OpConst {
                return Err(GuestError::invalid("QVM region has an indirect jump"));
            }
            let QvmOperand::Word(destination) = target.operand else {
                return Err(GuestError::invalid("QVM region has an indirect jump"));
            };
            let destination = usize::try_from(destination)
                .map_err(|_| GuestError::invalid("QVM region has an indirect jump"))?;
            edge(pc, destination, result)?;
        } else {
            edge(pc, pc + 1, result)?;
            if opcode.is_branch() {
                let QvmOperand::Word(destination) = instruction.operand else {
                    return Err(GuestError::invalid("QVM region branch lacks its source target"));
                };
                let destination = usize::try_from(destination)
                    .map_err(|_| GuestError::invalid("QVM region branch lacks its source target"))?;
                edge(pc, destination, result)?;
            }
        }
    }
    if !joined {
        return Err(GuestError::invalid("QVM region does not reach its original join"));
    }
    for pc in owner + 1..end {
        if (entry..join).contains(&pc) {
            continue;
        }
        let instruction = &instructions[pc];
        let mut destination: Option<usize> = None;
        if instruction.opcode.is_branch() {
            if let QvmOperand::Word(target) = instruction.operand {
                destination = usize::try_from(target).ok();
            }
        } else if instruction.opcode == QvmOpcode::OpJump && pc > 0 {
            let target = &instructions[pc - 1];
            if target.opcode == QvmOpcode::OpConst {
                if let QvmOperand::Word(index) = target.operand {
                    destination = usize::try_from(index).ok();
                }
            }
        }
        if destination.is_some_and(|target| target > entry && target < join) {
            return Err(GuestError::invalid("QVM region has an incoming interior edge"));
        }
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(byte_offset: usize, opcode: QvmOpcode, operand: QvmOperand) -> QvmInstruction {
        QvmInstruction { byte_offset, opcode, operand }
    }

    /// ENTER 16; CONST 1; CONST 2; ADD; POP; LEAVE 16
    fn program() -> Vec<QvmInstruction> {
        vec![
            instruction(0, QvmOpcode::OpEnter, QvmOperand::Word(16)),
            instruction(5, QvmOpcode::OpConst, QvmOperand::Word(1)),
            instruction(10, QvmOpcode::OpConst, QvmOperand::Word(2)),
            instruction(15, QvmOpcode::OpAdd, QvmOperand::None),
            instruction(16, QvmOpcode::OpPop, QvmOperand::None),
            instruction(17, QvmOpcode::OpLeave, QvmOperand::Word(16)),
        ]
    }

    #[test]
    fn qualifies_a_closed_stack_neutral_region() {
        let program = program();
        assert_eq!(qualify_qvm_region(&program, 0, 1, 5).unwrap(), 16);
    }

    #[test]
    fn rejects_regions_with_live_operands() {
        let program = program();
        assert!(qualify_qvm_region(&program, 0, 1, 4).is_err());
        assert!(qualify_qvm_region(&program, 0, 1, 6).is_err());
        assert!(qualify_qvm_region(&program, 1, 2, 5).is_err());
    }

    #[test]
    fn evaluation_checks_live_in_initialization() {
        // ENTER 16; LOCAL 8; LOAD4; POP; LEAVE 16
        let program = vec![
            instruction(0, QvmOpcode::OpEnter, QvmOperand::Word(16)),
            instruction(5, QvmOpcode::OpLocal, QvmOperand::Word(8)),
            instruction(10, QvmOpcode::OpLoad4, QvmOperand::None),
            instruction(11, QvmOpcode::OpPop, QvmOperand::None),
            instruction(12, QvmOpcode::OpLeave, QvmOperand::Word(16)),
        ];
        let region = QvmRegionEvaluation { entry: 1, join: 4, inputs: vec![8], result: None };
        assert_eq!(
            qualify_qvm_region_evaluation(&program, 0, &region, QvmRegionAccess::Source).unwrap(),
            16
        );
        let missing = QvmRegionEvaluation { entry: 1, join: 4, inputs: vec![], result: None };
        assert!(qualify_qvm_region_evaluation(&program, 0, &missing, QvmRegionAccess::Source).is_err());
    }

    #[test]
    fn read_only_rejects_calls_and_foreign_writes() {
        // ENTER 16; CONST 0; CALL; LEAVE 16
        let program = vec![
            instruction(0, QvmOpcode::OpEnter, QvmOperand::Word(16)),
            instruction(5, QvmOpcode::OpConst, QvmOperand::Word(0)),
            instruction(10, QvmOpcode::OpCall, QvmOperand::None),
            instruction(11, QvmOpcode::OpLeave, QvmOperand::Word(16)),
        ];
        let region = QvmRegionEvaluation { entry: 1, join: 3, inputs: vec![], result: None };
        assert!(qualify_qvm_region_evaluation(&program, 0, &region, QvmRegionAccess::ReadOnly).is_err());
        assert!(qualify_qvm_region_evaluation(&program, 0, &region, QvmRegionAccess::Source).is_ok());
    }
}
