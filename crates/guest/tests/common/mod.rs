//! Shared fixtures for guest integration tests: synthetic code, stack,
//! and return-address setup mirroring the donor `tests/guest` harnesses.

use qa_core::identity::ProviderId;
use qa_guest::abi::GuestCpu;
use qa_guest::core::contracts::{
    ContentDigest, GuestAddress, GuestArchitecture, GuestExecutionStop, GuestMapOptions,
    GuestPermissions, ModuleIdentity,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{
    GuestProcessorInitialState, GuestProcessorState,
};
use qa_guest::x64::cpu::X64Cpu;

/// Fixture code base.
pub const BASE: u64 = 0x10000;
/// Fixture stack pointer.
pub const STACK: u64 = 0x30000;
/// Fixture return address.
pub const RETURNED: u64 = 0x40000;

/// Test module identity.
pub fn test_module(name: &str) -> ModuleIdentity {
    ModuleIdentity::new(
        ProviderId::new("test", name),
        "authored-test-bytes",
        ContentDigest::new("sha256", "abc"),
        "1",
    )
}

/// Resolve a mapped offset, panicking when unmapped.
pub fn addr(memory: &SparseGuestMemory, offset: u64) -> GuestAddress {
    memory
        .pointer(offset)
        .expect("fixture pointer")
        .expect("nonnull fixture address")
}

/// Map a fixture region.
pub fn map(
    memory: &mut SparseGuestMemory,
    base: u64,
    byte_length: usize,
    permissions: GuestPermissions,
    bytes: Option<Vec<u8>>,
) -> GuestAddress {
    memory
        .map(&GuestMapOptions {
            base,
            byte_length,
            permissions,
            label: "fixture".to_string(),
            bytes,
        })
        .expect("fixture map")
}

/// Fresh processor state and memory with `bytes` mapped executable at
/// `start`, a stack at [`STACK`], and [`RETURNED`] pushed as the return
/// address.
pub fn build(bytes: &[u8], start: u64, module: ModuleIdentity) -> (GuestProcessorState, SparseGuestMemory) {
    let mut memory = SparseGuestMemory::new(module, 8, 0x10000).expect("fixture memory");
    map(
        &mut memory,
        start,
        bytes.len(),
        GuestPermissions::ReadExecute,
        Some(bytes.to_vec()),
    );
    map(
        &mut memory,
        STACK - 0x1000,
        0x1008,
        GuestPermissions::ReadWrite,
        None,
    );
    let stack = addr(&memory, STACK);
    memory.write_u64(stack, RETURNED).expect("fixture return");
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::X86_64,
        instruction_pointer: start,
        stack_pointer: STACK,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .expect("fixture state");
    (state, memory)
}

/// x86-64 fixture CPU.
pub struct Fixture {
    /// Fixture CPU (owns state and memory).
    pub cpu: X64Cpu,
    /// Fixture return address.
    pub returned: GuestAddress,
}

impl Fixture {
    /// Build a fixture at [`BASE`].
    pub fn new(bytes: &[u8]) -> Self {
        Self::at(bytes, BASE)
    }

    /// Build a fixture at `start`.
    pub fn at(bytes: &[u8], start: u64) -> Self {
        let (state, mut memory) = build(bytes, start, test_module("x64"));
        memory
            .map(&GuestMapOptions {
                base: RETURNED,
                byte_length: 8,
                permissions: GuestPermissions::ReadExecute,
                label: "fixture return".to_string(),
                bytes: None,
            })
            .expect("fixture return map");
        let returned = addr(&memory, RETURNED);
        let cpu = X64Cpu::new(state, memory).expect("fixture cpu");
        Self { cpu, returned }
    }

    /// Run with `budget`, stopping at the fixture return address.
    pub fn run(&mut self, budget: u64) -> GuestExecutionStop {
        self.cpu.run(budget, Some(self.returned))
    }

    /// Borrow state and memory together.
    pub fn with<R>(
        &mut self,
        f: impl FnOnce(&mut GuestProcessorState, &mut SparseGuestMemory) -> R,
    ) -> R {
        let (state, memory) = self.cpu.parts();
        f(state, memory)
    }
}
