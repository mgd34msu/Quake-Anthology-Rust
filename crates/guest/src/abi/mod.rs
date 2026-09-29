//! Guest ABI marshalling: value layouts, call plans, adapters, runner.
#[allow(dead_code)]
pub mod adapter;
#[allow(dead_code)]
pub mod classify;
#[allow(dead_code)]
pub mod runner;
#[allow(dead_code)]
pub mod values;

use std::rc::Rc;

use crate::core::callbacks::HookState;
use crate::core::contracts::{GuestAddress, GuestExecutionStop};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;

/// Guest CPU surface consumed by ABI adapters and the call runner.
pub trait GuestCpu {
    /// Borrow processor state and memory together.
    fn parts(&mut self) -> (&mut GuestProcessorState, &mut SparseGuestMemory);

    /// Install or clear the shared callback-hook state consulted at every
    /// instruction entry.
    fn set_hook_state(&mut self, hooks: Option<Rc<HookState>>);

    /// Run until the budget exhausts, `return_address` is reached, a trap
    /// fires, or execution faults.
    fn run(
        &mut self,
        instruction_budget: u64,
        return_address: Option<GuestAddress>,
    ) -> GuestExecutionStop;
}
