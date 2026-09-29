//! Player-body call scoping: qualify original mesh calls inside one function.
//!
//! Port of `src/compat/qvm/body-scope.ts` (`qualifyQvmBodyCalls`). A declared
//! body helper owns only its direct calls inside the matched original player
//! function: every qualified call must be an `OP_CALL` whose target word is an
//! `OP_CONST` naming the mesh entry, strictly inside the player function body.
//!
//! Local mirrors (the presentation contract is owned by another worker):
//! `QvmBodyPart`, `QvmBodyPlayer`, `QvmBodyMesh`, and `QvmBodyScope` mirror the
//! `body` member of `QvmScenePresentation` from the donor
//! `qvm-mod-presentation` contracts.

use std::collections::BTreeMap;

use crate::error::GuestError;

use super::image::{QvmImage, QvmOpcode, QvmOperand, QVM_MAX_PRIVATE_ARGUMENT_WORDS};

/// Anatomical part selected by one mesh call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmBodyPart {
    /// Whole body.
    Body,
    /// Lower body.
    Lower,
    /// Upper body.
    Upper,
    /// Head.
    Head,
}

impl QvmBodyPart {
    /// Donor `"body"` / `"lower"` / `"upper"` / `"head"` spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Lower => "lower",
            Self::Upper => "upper",
            Self::Head => "head",
        }
    }

    /// Parse a donor spelling.
    pub fn parse(text: &str) -> Result<Self, GuestError> {
        match text {
            "body" => Ok(Self::Body),
            "lower" => Ok(Self::Lower),
            "upper" => Ok(Self::Upper),
            "head" => Ok(Self::Head),
            other => Err(GuestError::invalid(format!("Unknown QVM body part {other}"))),
        }
    }
}

/// Declared original player function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmBodyPlayer {
    /// Player function entry instruction index.
    pub entry: usize,
    /// Centity argument word.
    pub centity_argument: usize,
}

/// Declared original mesh helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmBodyMesh {
    /// Mesh function entry instruction index.
    pub entry: usize,
    /// Entity argument word.
    pub entity_argument: usize,
    /// State argument word.
    pub state_argument: usize,
    /// Shader field offset (must be 112, the original `refEntity` ABI).
    pub shader_offset: usize,
    /// Exact direct `CALL` sites mapped to parts; `None` qualifies every
    /// direct player-to-mesh call as [`QvmBodyPart::Body`].
    pub parts: Option<Vec<QvmBodyCallPart>>,
}

/// One declared direct call site and its part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmBodyCallPart {
    /// Call instruction index.
    pub call: usize,
    /// Selected part.
    pub part: QvmBodyPart,
}

/// Declared body scope: player function plus mesh helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmBodyScope {
    /// Player function declaration.
    pub player: QvmBodyPlayer,
    /// Mesh helper declaration.
    pub mesh: QvmBodyMesh,
}

/// Qualify original player-to-mesh calls, mapping call sites to parts.
pub fn qualify_qvm_body_calls(
    image: &QvmImage,
    body: &QvmBodyScope,
) -> Result<BTreeMap<usize, QvmBodyPart>, GuestError> {
    let arguments = [
        body.player.centity_argument,
        body.mesh.entity_argument,
        body.mesh.state_argument,
    ];
    if arguments
        .iter()
        .any(|argument| *argument >= QVM_MAX_PRIVATE_ARGUMENT_WORDS)
        || body.mesh.shader_offset != 112
    {
        return Err(GuestError::invalid(
            "Source body arguments differ from the original refEntity ABI",
        ));
    }
    let player_entry = image
        .instructions
        .get(body.player.entry)
        .map(|instruction| instruction.opcode);
    let mesh_entry = image
        .instructions
        .get(body.mesh.entry)
        .map(|instruction| instruction.opcode);
    if player_entry != Some(QvmOpcode::OpEnter) || mesh_entry != Some(QvmOpcode::OpEnter) {
        return Err(GuestError::invalid(
            "Source body scope requires original function entries",
        ));
    }
    let mut end = body.player.entry + 1;
    while end < image.instructions.len() && image.instructions[end].opcode != QvmOpcode::OpEnter {
        end += 1;
    }
    let valid = |index: usize| -> bool {
        if index <= body.player.entry || index >= end || index == 0 {
            return false;
        }
        let Some(target) = image.instructions.get(index - 1) else {
            return false;
        };
        let Some(current) = image.instructions.get(index) else {
            return false;
        };
        current.opcode == QvmOpcode::OpCall
            && target.opcode == QvmOpcode::OpConst
            && matches!(target.operand, QvmOperand::Word(word) if word as usize == body.mesh.entry)
    };
    let mut calls = BTreeMap::new();
    match body.mesh.parts.as_ref() {
        None => {
            for index in body.player.entry + 1..end {
                if valid(index) {
                    calls.insert(index, QvmBodyPart::Body);
                }
            }
        }
        Some(parts) => {
            for row in parts {
                if !valid(row.call) || calls.contains_key(&row.call) {
                    return Err(GuestError::invalid(
                        "Source body part does not name a distinct original player-to-mesh call",
                    ));
                }
                calls.insert(row.call, row.part);
            }
        }
    }
    if calls.is_empty() {
        return Err(GuestError::invalid(
            "Source body scope has no qualified original mesh calls",
        ));
    }
    Ok(calls)
}

#[cfg(test)]
mod tests {
    use super::super::image::QvmInstruction;
    use super::*;

    fn image(program: Vec<(QvmOpcode, QvmOperand)>) -> QvmImage {
        let mut offset = 0usize;
        let instructions = program
            .into_iter()
            .map(|(opcode, operand)| {
                let instruction = QvmInstruction {
                    byte_offset: offset,
                    opcode,
                    operand,
                };
                offset += 1 + opcode.operand_width();
                instruction
            })
            .collect();
        QvmImage {
            source: "test".to_string(),
            instructions,
            code_offset: 0,
            code_length: offset,
            data_length: 0,
            literal_length: 0,
            bss_length: 0,
            initialized_data: Vec::new(),
            allocated_data_length: 1,
            data_mask: 0,
        }
    }

    fn scope(parts: Option<Vec<QvmBodyCallPart>>) -> QvmBodyScope {
        QvmBodyScope {
            player: QvmBodyPlayer {
                entry: 0,
                centity_argument: 1,
            },
            mesh: QvmBodyMesh {
                entry: 5,
                entity_argument: 1,
                state_argument: 2,
                shader_offset: 112,
                parts,
            },
        }
    }

    /// Player body with two direct mesh calls; mesh helper after.
    fn program() -> QvmImage {
        use QvmOpcode as O;
        image(vec![
            (O::OpEnter, QvmOperand::Word(8)),
            (O::OpConst, QvmOperand::Word(5)),
            (O::OpCall, QvmOperand::None),
            (O::OpConst, QvmOperand::Word(5)),
            (O::OpCall, QvmOperand::None),
            (O::OpEnter, QvmOperand::Word(8)),
            (O::OpConst, QvmOperand::Word(0)),
            (O::OpLeave, QvmOperand::Word(8)),
        ])
    }

    #[test]
    fn qualifies_every_direct_call_as_body() {
        let calls = qualify_qvm_body_calls(&program(), &scope(None)).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[&2], QvmBodyPart::Body);
        assert_eq!(calls[&4], QvmBodyPart::Body);
    }

    #[test]
    fn declared_parts_must_name_distinct_calls() {
        let calls = qualify_qvm_body_calls(
            &program(),
            &scope(Some(vec![
                QvmBodyCallPart {
                    call: 2,
                    part: QvmBodyPart::Head,
                },
                QvmBodyCallPart {
                    call: 4,
                    part: QvmBodyPart::Lower,
                },
            ])),
        )
        .unwrap();
        assert_eq!(calls[&2], QvmBodyPart::Head);
        assert_eq!(calls[&4], QvmBodyPart::Lower);
        assert!(qualify_qvm_body_calls(
            &program(),
            &scope(Some(vec![
                QvmBodyCallPart {
                    call: 2,
                    part: QvmBodyPart::Head
                },
                QvmBodyCallPart {
                    call: 2,
                    part: QvmBodyPart::Lower
                },
            ])),
        )
        .is_err());
        assert!(qualify_qvm_body_calls(&program(), &scope(Some(vec![]))).is_err());
    }

    #[test]
    fn rejects_abi_mismatches_and_indirect_calls() {
        use QvmOpcode as O;
        let mut bad = scope(None);
        bad.mesh.shader_offset = 0;
        assert!(qualify_qvm_body_calls(&program(), &bad).is_err());
        let mut bad = scope(None);
        bad.player.entry = 1;
        assert!(qualify_qvm_body_calls(&program(), &bad).is_err());
        // Indirect call target: CONST names another function.
        let indirect = image(vec![
            (O::OpEnter, QvmOperand::Word(8)),
            (O::OpConst, QvmOperand::Word(6)),
            (O::OpCall, QvmOperand::None),
            (O::OpConst, QvmOperand::Word(0)),
            (O::OpLeave, QvmOperand::Word(8)),
            (O::OpEnter, QvmOperand::Word(8)),
            (O::OpEnter, QvmOperand::Word(8)),
            (O::OpLeave, QvmOperand::Word(8)),
        ]);
        assert!(qualify_qvm_body_calls(&indirect, &scope(None)).is_err());
        assert_eq!(QvmBodyPart::parse("head").unwrap(), QvmBodyPart::Head);
        assert!(QvmBodyPart::parse("tail").is_err());
    }
}
