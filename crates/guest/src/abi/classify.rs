//! ABI call plans: classify parameters and results into registers/stack.
//!
//! Donor: `src/guest/abi/classify.ts`. Covers Microsoft x64, System V AMD64,
//! Windows i386 (`cdecl`/`stdcall`/`thiscall`/`fastcall`), and System V i386,
//! including System V eightbyte classification, hidden aggregate-result
//! pointers, and callee stack cleanup.

use crate::core::contracts::{
    GuestCallSignature, GuestRegister, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use crate::error::GuestError;

use super::values::{align_up, argument_bytes, storage_bytes, validate_value_layout, value_alignment, value_bytes};

/// One value fragment location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiLocation {
    /// Integer register fragment.
    Integer {
        /// Destination register.
        register: GuestRegister,
        /// Byte offset within the value.
        offset: usize,
        /// Fragment length.
        bytes: usize,
    },
    /// XMM register fragment.
    Sse {
        /// XMM register index.
        register: usize,
        /// Byte offset within the value.
        offset: usize,
        /// Fragment length.
        bytes: usize,
    },
    /// Outgoing stack fragment.
    Stack {
        /// Offset from the entry stack pointer (past the return address).
        stack_offset: usize,
        /// Byte offset within the value.
        offset: usize,
        /// Fragment length.
        bytes: usize,
    },
}

impl AbiLocation {
    /// Byte offset within the value.
    #[must_use]
    pub const fn offset(&self) -> usize {
        match self {
            Self::Integer { offset, .. } | Self::Sse { offset, .. } | Self::Stack { offset, .. } => {
                *offset
            }
        }
    }

    /// Fragment length in bytes.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        match self {
            Self::Integer { bytes, .. } | Self::Sse { bytes, .. } | Self::Stack { bytes, .. } => {
                *bytes
            }
        }
    }
}

/// One planned call argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiArgument {
    /// Argument layout.
    pub layout: GuestValueLayout,
    /// Passed by hidden pointer to a temporary.
    pub indirect: bool,
    /// Assigned fragment locations.
    pub locations: Vec<AbiLocation>,
}

/// Planned call result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiResult {
    /// No result.
    Void,
    /// x87 register-stack result.
    X87 {
        /// Result storage.
        storage: GuestStorage,
    },
    /// Register result fragments.
    Registers {
        /// Result layout.
        layout: GuestValueLayout,
        /// Assigned fragment locations.
        locations: Vec<AbiLocation>,
    },
    /// Memory result through a hidden pointer argument.
    Memory {
        /// Result layout.
        layout: GuestValueLayout,
        /// Hidden pointer argument.
        pointer: AbiArgument,
    },
}

/// Complete call plan: arguments, result, and frame geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiCallPlan {
    /// Calling convention.
    pub abi: NativeCallAbi,
    /// Planned arguments.
    pub arguments: Vec<AbiArgument>,
    /// Planned result.
    pub result: AbiResult,
    /// Outgoing stack bytes past the return-address slot.
    pub stack_bytes: usize,
    /// Entry stack alignment.
    pub stack_alignment: usize,
    /// Bytes the callee pops.
    pub callee_pop_bytes: usize,
    /// Vector registers consumed (System V `al` for variadics).
    pub vector_registers: usize,
}

/// System V eightbyte class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EightbyteClass {
    /// No significant bytes.
    None,
    /// Integer fragment.
    Integer,
    /// SSE fragment.
    Sse,
}

/// Classify an aggregate into System V eightbyte classes, or `None` for
/// memory classification. Layouts describe POD aggregates of scalar fields,
/// including overlapping union fields.
pub fn classify_system_v_aggregate(
    layout: &GuestValueLayout,
) -> Result<Option<Vec<EightbyteClass>>, GuestError> {
    validate_value_layout(layout, 8)?;
    if let GuestValueLayout::Scalar(storage) = layout {
        return Ok(Some(vec![if storage.is_float() {
            EightbyteClass::Sse
        } else {
            EightbyteClass::Integer
        }]));
    }
    let GuestValueLayout::Aggregate(record) = layout else {
        unreachable!("scalar handled above");
    };
    if record.byte_length > 16 {
        return Ok(None);
    }
    let mut classes = vec![EightbyteClass::None; record.byte_length.div_ceil(8)];
    for field in &record.fields {
        let size = storage_bytes(field.storage, 8);
        if field.byte_offset % size != 0 {
            return Ok(None);
        }
        for index in 0..field.count {
            let start = field.byte_offset + index * size;
            let end = start + size;
            for slot in start / 8..end.div_ceil(8) {
                let Some(prior) = classes.get(slot).copied() else {
                    return Err(GuestError::abi("Aggregate classification exceeds layout"));
                };
                let next = if field.storage.is_float() {
                    EightbyteClass::Sse
                } else {
                    EightbyteClass::Integer
                };
                classes[slot] = if prior == EightbyteClass::Integer || next == EightbyteClass::Integer
                {
                    EightbyteClass::Integer
                } else {
                    EightbyteClass::Sse
                };
            }
        }
    }
    Ok(Some(classes))
}

fn hidden_result(signature: &GuestCallSignature) -> Result<bool, GuestError> {
    let Some(result) = &signature.result else {
        return Ok(false);
    };
    if matches!(result, GuestValueLayout::Scalar(_)) {
        return Ok(false);
    }
    match signature.abi {
        NativeCallAbi::Thiscall => Ok(true),
        NativeCallAbi::SystemVI386 => Ok(true),
        NativeCallAbi::SystemVX86_64 => Ok(classify_system_v_aggregate(result)?.is_none()),
        _ => {
            let GuestValueLayout::Aggregate(record) = result else {
                return Ok(false);
            };
            Ok(![1, 2, 4, 8].contains(&record.byte_length))
        }
    }
}

/// Plan a call, defaulting to the signature's parameter layouts.
pub fn plan_guest_call(
    signature: &GuestCallSignature,
    layouts: Option<&[GuestValueLayout]>,
) -> Result<AbiCallPlan, GuestError> {
    match layouts {
        Some(layouts) => build_call_plan(signature, layouts),
        None => build_call_plan(signature, &signature.parameters),
    }
}

/// Plan a call over explicit (possibly variadic-extended) layouts.
pub fn plan_guest_call_layouts(
    signature: &GuestCallSignature,
    layouts: &[GuestValueLayout],
) -> Result<AbiCallPlan, GuestError> {
    build_call_plan(signature, layouts)
}

struct Planner<'a> {
    signature: &'a GuestCallSignature,
    word: usize,
    microsoft64: bool,
    system64: bool,
    integer_registers: &'static [GuestRegister],
    stack_bytes: usize,
    integer_count: usize,
    vector_count: usize,
    ordinal: usize,
    stack_alignment: usize,
}

impl Planner<'_> {
    fn stack(&mut self, layout: GuestValueLayout, indirect: bool) -> AbiArgument {
        let size = if indirect {
            self.word
        } else {
            argument_bytes(&layout, self.word)
        };
        let alignment = match self.signature.abi {
            NativeCallAbi::Cdecl | NativeCallAbi::Stdcall | NativeCallAbi::Thiscall | NativeCallAbi::Fastcall => 4,
            _ if indirect => self.word,
            _ => self.word.max(value_alignment(&layout, self.word)),
        };
        self.stack_alignment = self.stack_alignment.max(alignment);
        self.stack_bytes = align_up(self.stack_bytes - self.word, alignment) + self.word;
        let location = AbiLocation::Stack {
            stack_offset: self.stack_bytes,
            offset: 0,
            bytes: size,
        };
        self.stack_bytes += align_up(size, self.word);
        AbiArgument {
            layout,
            indirect,
            locations: vec![location],
        }
    }

    fn assign(
        &mut self,
        layout: GuestValueLayout,
        is_hidden: bool,
        parameter_index: usize,
    ) -> Result<AbiArgument, GuestError> {
        let size = argument_bytes(&layout, self.word);
        let floating = matches!(
            &layout,
            GuestValueLayout::Scalar(storage) if storage.is_float()
        );
        if self.microsoft64 {
            let position = self.ordinal;
            self.ordinal += 1;
            let indirect = matches!(&layout, GuestValueLayout::Aggregate(_))
                && ![1, 2, 4, 8].contains(&size);
            let Some(register) = self.integer_registers.get(position).copied() else {
                return Ok(self.stack(layout, indirect));
            };
            let mut locations = if floating {
                vec![AbiLocation::Sse {
                    register: position,
                    offset: 0,
                    bytes: size,
                }]
            } else {
                vec![AbiLocation::Integer {
                    register,
                    offset: 0,
                    bytes: if indirect { self.word } else { size },
                }]
            };
            if floating && self.signature.variadic {
                locations.push(AbiLocation::Integer {
                    register,
                    offset: 0,
                    bytes: size,
                });
            }
            if floating {
                self.vector_count = self.vector_count.max(position + 1);
            }
            return Ok(AbiArgument {
                layout,
                indirect,
                locations,
            });
        }
        if self.system64 {
            let Some(classes) = classify_system_v_aggregate(&layout)? else {
                return Ok(self.stack(layout, false));
            };
            let needed_integers = classes.iter().filter(|class| **class == EightbyteClass::Integer).count();
            let needed_vectors = classes.iter().filter(|class| **class == EightbyteClass::Sse).count();
            if self.integer_count + needed_integers > self.integer_registers.len()
                || self.vector_count + needed_vectors > 8
            {
                return Ok(self.stack(layout, false));
            }
            let mut locations = Vec::new();
            for (index, class) in classes.iter().enumerate() {
                let offset = index * 8;
                let bytes = 8.min(size - offset);
                match class {
                    EightbyteClass::Integer => {
                        let Some(register) =
                            self.integer_registers.get(self.integer_count).copied()
                        else {
                            return Err(GuestError::abi(
                                "System V integer register allocation overflow",
                            ));
                        };
                        self.integer_count += 1;
                        locations.push(AbiLocation::Integer {
                            register,
                            offset,
                            bytes,
                        });
                    }
                    EightbyteClass::Sse => {
                        locations.push(AbiLocation::Sse {
                            register: self.vector_count,
                            offset,
                            bytes,
                        });
                        self.vector_count += 1;
                    }
                    EightbyteClass::None => {}
                }
            }
            return Ok(AbiArgument {
                layout,
                indirect: false,
                locations,
            });
        }
        if matches!(
            self.signature.abi,
            NativeCallAbi::Cdecl
                | NativeCallAbi::Stdcall
                | NativeCallAbi::Thiscall
                | NativeCallAbi::Fastcall
        ) && !self.signature.variadic
        {
            if self.signature.abi == NativeCallAbi::Thiscall
                && parameter_index == 0
                && !is_hidden
            {
                if !matches!(
                    &layout,
                    GuestValueLayout::Scalar(GuestStorage::Pointer)
                ) {
                    return Err(GuestError::abi(
                        "thiscall's first explicit argument must be the this pointer",
                    ));
                }
                return Ok(AbiArgument {
                    layout,
                    indirect: false,
                    locations: vec![AbiLocation::Integer {
                        register: GuestRegister::Rcx,
                        offset: 0,
                        bytes: 4,
                    }],
                });
            }
            if self.signature.abi == NativeCallAbi::Fastcall
                && matches!(&layout, GuestValueLayout::Scalar(_))
                && !floating
                && size <= 4
                && self.integer_count < 2
            {
                let register = if self.integer_count == 0 {
                    GuestRegister::Rcx
                } else {
                    GuestRegister::Rdx
                };
                self.integer_count += 1;
                return Ok(AbiArgument {
                    layout,
                    indirect: false,
                    locations: vec![AbiLocation::Integer {
                        register,
                        offset: 0,
                        bytes: size,
                    }],
                });
            }
        }
        Ok(self.stack(layout, false))
    }
}

fn build_call_plan(
    signature: &GuestCallSignature,
    layouts: &[GuestValueLayout],
) -> Result<AbiCallPlan, GuestError> {
    let abi = signature.abi;
    let word = abi.pointer_bytes();
    let hidden = hidden_result(signature)?;
    if (!signature.variadic && layouts.len() != signature.parameters.len())
        || layouts.len() < signature.parameters.len()
    {
        return Err(GuestError::abi("Call argument count differs from signature"));
    }
    for layout in layouts {
        validate_value_layout(layout, word)?;
    }
    if let Some(result) = &signature.result {
        validate_value_layout(result, word)?;
    }
    let microsoft64 = abi == NativeCallAbi::MicrosoftX64;
    let system64 = abi == NativeCallAbi::SystemVX86_64;
    let integer_registers: &[GuestRegister] = if microsoft64 {
        &[GuestRegister::Rcx, GuestRegister::Rdx, GuestRegister::R8, GuestRegister::R9]
    } else if system64 {
        &[
            GuestRegister::Rdi,
            GuestRegister::Rsi,
            GuestRegister::Rdx,
            GuestRegister::Rcx,
            GuestRegister::R8,
            GuestRegister::R9,
        ]
    } else {
        &[]
    };
    let mut planner = Planner {
        signature,
        word,
        microsoft64,
        system64,
        integer_registers,
        stack_bytes: word + if microsoft64 { 32 } else { 0 },
        integer_count: 0,
        vector_count: 0,
        ordinal: 0,
        stack_alignment: if word == 8 || abi == NativeCallAbi::SystemVI386 {
            16
        } else {
            4
        },
    };
    let this_before_hidden =
        hidden && abi == NativeCallAbi::Thiscall && signature.variadic;
    if this_before_hidden
        && !matches!(
            layouts.first(),
            Some(GuestValueLayout::Scalar(GuestStorage::Pointer))
        )
    {
        return Err(GuestError::abi("Variadic thiscall requires an explicit this pointer"));
    }
    let first_argument = if this_before_hidden {
        let layout = layouts[0].clone();
        Some(planner.assign(layout, false, 0)?)
    } else {
        None
    };
    let hidden_pointer = if hidden {
        Some(planner.assign(
            GuestValueLayout::Scalar(GuestStorage::Pointer),
            true,
            usize::MAX,
        )?)
    } else {
        None
    };
    let mut arguments = Vec::with_capacity(layouts.len());
    for (index, layout) in layouts.iter().enumerate() {
        if index == 0 && first_argument.is_some() {
            arguments.push(first_argument.clone().expect("checked above"));
        } else {
            arguments.push(planner.assign(layout.clone(), false, index)?);
        }
    }
    let result = match (&signature.result, hidden_pointer) {
        (None, _) => AbiResult::Void,
        (Some(result), Some(pointer)) => AbiResult::Memory {
            layout: result.clone(),
            pointer,
        },
        (Some(result), None)
            if word == 4
                && matches!(
                    result,
                    GuestValueLayout::Scalar(storage) if storage.is_float()
                ) =>
        {
            let GuestValueLayout::Scalar(storage) = result else {
                unreachable!("float check above");
            };
            AbiResult::X87 { storage: *storage }
        }
        (Some(result), None) => {
            let size = value_bytes(result, word);
            let mut locations = Vec::new();
            if system64 {
                let Some(classes) = classify_system_v_aggregate(result)? else {
                    return Err(GuestError::abi("Memory result lacks hidden pointer"));
                };
                let (mut integer_return, mut vector_return) = (0, 0);
                for (index, class) in classes.iter().enumerate() {
                    let offset = index * 8;
                    let bytes = 8.min(size - offset);
                    match class {
                        EightbyteClass::Integer => {
                            locations.push(AbiLocation::Integer {
                                register: if integer_return == 0 {
                                    GuestRegister::Rax
                                } else {
                                    GuestRegister::Rdx
                                },
                                offset,
                                bytes,
                            });
                            integer_return += 1;
                        }
                        EightbyteClass::Sse => {
                            locations.push(AbiLocation::Sse {
                                register: vector_return,
                                offset,
                                bytes,
                            });
                            vector_return += 1;
                        }
                        EightbyteClass::None => {}
                    }
                }
            } else if microsoft64
                && matches!(
                    result,
                    GuestValueLayout::Scalar(storage) if storage.is_float()
                )
            {
                locations.push(AbiLocation::Sse {
                    register: 0,
                    offset: 0,
                    bytes: size,
                });
            } else {
                locations.push(AbiLocation::Integer {
                    register: GuestRegister::Rax,
                    offset: 0,
                    bytes: word.min(size),
                });
                if size > word {
                    locations.push(AbiLocation::Integer {
                        register: GuestRegister::Rdx,
                        offset: word,
                        bytes: size - word,
                    });
                }
            }
            AbiResult::Registers {
                layout: result.clone(),
                locations,
            }
        }
    };
    let callee_pop_bytes = if abi == NativeCallAbi::SystemVI386 && hidden {
        4
    } else if matches!(
        abi,
        NativeCallAbi::Stdcall | NativeCallAbi::Thiscall | NativeCallAbi::Fastcall
    ) && !signature.variadic
    {
        planner.stack_bytes - word
    } else {
        0
    };
    Ok(AbiCallPlan {
        abi,
        arguments,
        result,
        stack_bytes: planner.stack_bytes,
        stack_alignment: planner.stack_alignment,
        callee_pop_bytes,
        vector_registers: planner.vector_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_signature(abi: NativeCallAbi, parameters: Vec<GuestStorage>) -> GuestCallSignature {
        GuestCallSignature {
            abi,
            parameters: parameters.into_iter().map(GuestValueLayout::Scalar).collect(),
            result: Some(GuestValueLayout::Scalar(GuestStorage::Int32)),
            variadic: false,
        }
    }

    #[test]
    fn system_v_assigns_registers_then_stack() {
        let signature = scalar_signature(
            NativeCallAbi::SystemVX86_64,
            vec![GuestStorage::Int32; 8],
        );
        let plan = plan_guest_call(&signature, None).unwrap();
        assert!(matches!(
            plan.arguments[0].locations[0],
            AbiLocation::Integer {
                register: GuestRegister::Rdi,
                ..
            }
        ));
        assert!(matches!(
            plan.arguments[6].locations[0],
            AbiLocation::Stack { .. }
        ));
        assert!(matches!(
            plan.result,
            AbiResult::Registers { .. }
        ));
    }

    #[test]
    fn i386_float_results_use_x87() {
        let signature = GuestCallSignature {
            abi: NativeCallAbi::Cdecl,
            parameters: vec![],
            result: Some(GuestValueLayout::Scalar(GuestStorage::Float64)),
            variadic: false,
        };
        let plan = plan_guest_call(&signature, None).unwrap();
        assert!(matches!(plan.result, AbiResult::X87 { .. }));
    }

    #[test]
    fn stdcall_callee_pops_stack_arguments() {
        let signature = scalar_signature(NativeCallAbi::Stdcall, vec![GuestStorage::Int32]);
        let plan = plan_guest_call(&signature, None).unwrap();
        assert_eq!(plan.callee_pop_bytes, plan.stack_bytes - 4);
    }
}
