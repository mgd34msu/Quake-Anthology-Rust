//! Exact floating-point execution for x87 and SSE instructions.
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

/// Dispatch one numeric instruction to the x87 or SSE executor.
pub fn execute_numeric_instruction(
    context: NumericExecutionContext,
) -> NumericExecutionResult {
    let opcode = context.instruction.opcode;
    let result = if opcode == 0x9b || (0xd8..=0xdf).contains(&opcode) {
        x87::execute_x87(context).map(|()| NumericExecutionResult::Executed)
    } else if opcode == 0x0f {
        sse::execute_sse(context).map(|()| NumericExecutionResult::Executed)
    } else {
        return NumericExecutionResult::Unsupported {
            detail: "Unsupported numeric instruction family".to_string(),
        };
    };
    match result {
        Ok(done) => done,
        Err(NumericError::Unsupported(detail)) => NumericExecutionResult::Unsupported { detail },
        Err(NumericError::Fault { vector, detail }) => {
            NumericExecutionResult::Exception { vector, detail }
        }
        Err(NumericError::Guest(error)) => NumericExecutionResult::Unsupported {
            detail: format!("Guest failure in numeric instruction: {error}"),
        },
    }
}

/// Host-side numeric failure surfaced from guest execution.
#[must_use]
pub fn numeric_guest_error(error: GuestError) -> NumericError {
    NumericError::Guest(error)
}
