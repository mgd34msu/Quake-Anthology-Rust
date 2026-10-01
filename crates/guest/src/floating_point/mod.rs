//! Exact floating-point execution for x87 and SSE instructions.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/guest/floating-point/index.ts`.
#[allow(dead_code)]
pub mod binary;
#[allow(dead_code)]
pub mod contracts;
#[allow(dead_code)]
pub mod raw_sse;
#[allow(dead_code)]
pub mod sse;
#[allow(dead_code)]
pub mod trigonometric;
#[allow(dead_code)]
pub mod x87;

use crate::error::GuestError;

use self::contracts::{NumericError, NumericExecutionContext, NumericExecutionResult};

/// Dispatch one numeric instruction to the x87 or SSE executor. Guest
/// memory failures propagate to the caller like any other CPU fault.
pub fn execute_numeric_instruction(context: NumericExecutionContext) -> Result<NumericExecutionResult, GuestError> {
    let opcode = context.instruction.opcode;
    let result = if opcode == 0x9b || (0xd8..=0xdf).contains(&opcode) {
        x87::execute_x87(context).map(|()| NumericExecutionResult::Executed)
    } else if opcode == 0x0f {
        sse::execute_sse(context).map(|()| NumericExecutionResult::Executed)
    } else {
        return Ok(NumericExecutionResult::Unsupported {
            detail: "Unsupported numeric instruction family".to_string(),
        });
    };
    match result {
        Ok(done) => Ok(done),
        Err(NumericError::Unsupported(detail)) => Ok(NumericExecutionResult::Unsupported { detail }),
        Err(NumericError::Fault { vector, detail }) => Ok(NumericExecutionResult::Exception { vector, detail }),
        Err(NumericError::Guest(error)) => Err(error),
    }
}

/// Host-side numeric failure surfaced from guest execution.
#[must_use]
pub fn numeric_guest_error(error: GuestError) -> NumericError {
    NumericError::Guest(error)
}
