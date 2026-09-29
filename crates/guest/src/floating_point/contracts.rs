//! Numeric executor contracts: decoded instruction, context, outcome.
//!
//! Donor: `src/guest/floating-point/contracts.ts`.

use crate::core::contracts::GuestAddress;
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;

/// Numeric instruction operand: guest memory or an XMM/register index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericOperand {
    /// Guest memory operand.
    Memory(GuestAddress),
    /// Register operand (XMM index for SSE, stack index for x87).
    Register(usize),
}

/// Decoded numeric instruction handed to the x87/SSE executors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericInstruction {
    /// Primary opcode byte.
    pub opcode: u8,
    /// Secondary opcode byte for `0x0f` escapes.
    pub secondary_opcode: Option<u8>,
    /// ModRM byte, if present.
    pub modrm: Option<u8>,
    /// Decoded operand.
    pub operand: Option<NumericOperand>,
    /// Register field of ModRM.
    pub register_index: usize,
    /// SIMD mandatory prefix.
    pub prefix: NumericPrefix,
    /// Operand width in bits.
    pub operand_bits: u16,
    /// Trailing immediate byte, if present.
    pub immediate: Option<u8>,
}

/// Mandatory SIMD prefix selecting the lane type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumericPrefix {
    /// No prefix.
    #[default]
    None,
    /// `0x66`: packed double / integer lanes.
    X66,
    /// `0xf2`: scalar double lanes.
    XF2,
    /// `0xf3`: scalar single lanes.
    XF3,
}

/// Live execution context for one numeric instruction.
pub struct NumericExecutionContext<'a> {
    /// Processor state.
    pub state: &'a mut GuestProcessorState,
    /// Guest memory.
    pub memory: &'a mut SparseGuestMemory,
    /// Decoded instruction.
    pub instruction: NumericInstruction,
}

/// Outcome of one numeric instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumericExecutionResult {
    /// Instruction retired.
    Executed,
    /// Undecodable or unimplemented encoding.
    Unsupported {
        /// Human-readable detail.
        detail: String,
    },
    /// Processor exception raised.
    Exception {
        /// Exception vector (13, 16, or 19).
        vector: u8,
        /// Human-readable detail.
        detail: String,
    },
}

/// Internal numeric failure: unsupported encoding, fault, or guest error.
#[derive(Debug)]
pub enum NumericError {
    /// Undecodable or unimplemented encoding.
    Unsupported(String),
    /// Processor exception (vector 13, 16, or 19).
    Fault {
        /// Exception vector.
        vector: u8,
        /// Human-readable detail.
        detail: String,
    },
    /// Underlying guest failure.
    Guest(GuestError),
}

impl NumericError {
    /// Unsupported encoding.
    #[must_use]
    pub fn unsupported(detail: impl Into<String>) -> Self {
        Self::Unsupported(detail.into())
    }

    /// Processor fault.
    #[must_use]
    pub fn fault(vector: u8, detail: impl Into<String>) -> Self {
        Self::Fault {
            vector,
            detail: detail.into(),
        }
    }
}

impl From<GuestError> for NumericError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error)
    }
}
