//! i386 CPU integration tests: synthetic fixtures with exact
//! register/memory assertions.
//!
//! Donor: `tests/guest/x86/cpu.test.ts`. The retail-DLL case is replaced
//! by a synthetic equivalent (no ambient artifacts).

mod common;

use qa_guest::abi::GuestCpu;
use qa_guest::core::contracts::{
    GuestArchitecture, GuestException, GuestExecutionStop, GuestFlag, GuestIntegerWidth, GuestPermissions,
    GuestRegister,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use qa_guest::x86::cpu::I386Cpu;

use common::{addr, map, test_module};

const BASE: u64 = 0x1000;
const STACK: u64 = 0x9000;
const RETURNED: u64 = 0x7000;

fn machine(code: &[u8]) -> I386Fixture {
    let mut memory = SparseGuestMemory::new(test_module("x86"), 4, 0x10000).unwrap();
    map(
        &mut memory,
        BASE,
        code.len(),
        GuestPermissions::ReadExecute,
        Some(code.to_vec()),
    );
    map(&mut memory, 0x3000, 0x1000, GuestPermissions::ReadWrite, None);
    map(&mut memory, 0x8000, 0x2000, GuestPermissions::ReadWrite, None);
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::I386,
        instruction_pointer: BASE,
        stack_pointer: STACK,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    memory.write_u32(addr(&memory, STACK), RETURNED as u32).unwrap();
    let returned = addr(&memory, RETURNED);
    let cpu = I386Cpu::new(state, memory).unwrap();
    I386Fixture { cpu, returned }
}

struct I386Fixture {
    cpu: I386Cpu,
    returned: qa_guest::core::contracts::GuestAddress,
}

impl I386Fixture {
    fn run(&mut self, budget: u64) -> GuestExecutionStop {
        self.cpu.run(budget, Some(self.returned))
    }

    fn with<R>(&mut self, f: impl FnOnce(&mut GuestProcessorState, &mut SparseGuestMemory) -> R) -> R {
        let (state, memory) = self.cpu.parts();
        f(state, memory)
    }
}

fn read(f: &mut I386Fixture, register: GuestRegister, width: GuestIntegerWidth) -> u64 {
    f.with(|state, _| state.registers.read(register, width, false).unwrap())
}

fn write(f: &mut I386Fixture, register: GuestRegister, width: GuestIntegerWidth, value: u64) {
    f.with(|state, _| state.registers.write(register, width, value, false).unwrap());
}

fn processor_vector(stop: &GuestExecutionStop) -> u32 {
    match stop {
        GuestExecutionStop::Exception {
            exception: GuestException::Processor { vector, .. },
            ..
        } => *vector,
        other => panic!("expected processor exception, got {other:?}"),
    }
}

#[test]
fn aliases_operand_size_override_and_movsx_share_registers() {
    let mut f = machine(&[
        0xb8, 0x78, 0x56, 0x34, 0x12, 0xb4, 0xab, 0x66, 0xb8, 0xef, 0xbe, 0x0f, 0xbe, 0xc8, 0xc3,
    ]);
    assert_eq!(f.run(20).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 0x1234_beef);
    assert_eq!(read(&mut f, GuestRegister::Rcx, GuestIntegerWidth::B32), 0xffff_ffef);
    assert_eq!(read(&mut f, GuestRegister::Rsp, GuestIntegerWidth::B32), 0x9004);
}

#[test]
fn add_overflow_and_inc_carry_preservation_follow_width() {
    let mut first = machine(&[0xb0, 0x7f, 0x04, 0x01]);
    assert_eq!(first.cpu.run(2, None).kind(), "budget");
    assert_eq!(read(&mut first, GuestRegister::Rax, GuestIntegerWidth::B8), 0x80);
    first.with(|state, _| {
        assert!(state.flags.get(GuestFlag::Overflow));
        assert!(state.flags.get(GuestFlag::AuxiliaryCarry));
        assert!(state.flags.get(GuestFlag::Sign));
        assert!(!state.flags.get(GuestFlag::Carry));
        assert!(!state.flags.get(GuestFlag::Parity));
    });
    let mut second = machine(&[0xf9, 0xb8, 0xff, 0xff, 0xff, 0xff, 0x40, 0xc3]);
    assert_eq!(second.run(10).kind(), "return");
    assert_eq!(read(&mut second, GuestRegister::Rax, GuestIntegerWidth::B32), 0);
    second.with(|state, _| {
        assert!(state.flags.get(GuestFlag::Carry));
        assert!(state.flags.get(GuestFlag::Zero));
    });
}

#[test]
fn rotates_carry_through_carry_and_loop_branch() {
    let mut run = machine(&[0xb0, 1, 0xf8, 0xc0, 0xc0, 8, 0xd0, 0xd0, 0xc3]);
    assert_eq!(run.cpu.run(3, None).kind(), "budget");
    assert_eq!(read(&mut run, GuestRegister::Rax, GuestIntegerWidth::B8), 1);
    run.with(|state, _| assert!(state.flags.get(GuestFlag::Carry)));
    assert_eq!(run.run(10).kind(), "return");
    assert_eq!(read(&mut run, GuestRegister::Rax, GuestIntegerWidth::B8), 3);
    run.with(|state, _| assert!(!state.flags.get(GuestFlag::Carry)));
    let mut with_loop = machine(&[0xb9, 3, 0, 0, 0, 0xb8, 0, 0, 0, 0, 0x40, 0xe2, 0xfd, 0xc3]);
    assert_eq!(with_loop.run(20).kind(), "return");
    assert_eq!(read(&mut with_loop, GuestRegister::Rax, GuestIntegerWidth::B32), 3);
    assert_eq!(read(&mut with_loop, GuestRegister::Rcx, GuestIntegerWidth::B32), 0);
    let mut sar = machine(&[0x66, 0xb8, 0, 0x80, 0x66, 0xd1, 0xf8, 0xc3]);
    assert_eq!(sar.run(10).kind(), "return");
    assert_eq!(read(&mut sar, GuestRegister::Rax, GuestIntegerWidth::B16), 0xc000);
    sar.with(|state, _| {
        assert!(!state.flags.get(GuestFlag::Overflow));
        assert!(state.flags.get(GuestFlag::Sign));
    });
}

#[test]
fn modrm_sib_16bit_fs_and_lea_keep_address_rules() {
    let mut run = machine(&[
        0x8b, 0x44, 0x8b, 0x10, 0x67, 0x66, 0x8b, 0x52, 0x04, 0x64, 0x8b, 0x35, 0x20, 0, 0, 0, 0xc3,
    ]);
    run.with(|state, memory| {
        state
            .registers
            .write(GuestRegister::Rbx, GuestIntegerWidth::B32, 0x3000, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rcx, GuestIntegerWidth::B32, 3, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rbp, GuestIntegerWidth::B32, 0x3000, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rsi, GuestIntegerWidth::B32, 0x40, false)
            .unwrap();
        state.segments[GuestProcessorState::FS].base = 0x3000;
        memory.write_u32(addr(memory, 0x301c), 0x1122_3344).unwrap();
        memory.write_u16(addr(memory, 0x3044), 0xabcd).unwrap();
        memory.write_u32(addr(memory, 0x3020), 0x5566_7788).unwrap();
    });
    assert_eq!(run.run(20).kind(), "return");
    assert_eq!(read(&mut run, GuestRegister::Rax, GuestIntegerWidth::B32), 0x1122_3344);
    assert_eq!(read(&mut run, GuestRegister::Rdx, GuestIntegerWidth::B32), 0xabcd);
    assert_eq!(read(&mut run, GuestRegister::Rsi, GuestIntegerWidth::B32), 0x5566_7788);
    let mut lea = machine(&[0x8d, 0x14, 0x08, 0xc3]);
    assert_eq!(lea.run(10).kind(), "return");
    assert_eq!(read(&mut lea, GuestRegister::Rdx, GuestIntegerWidth::B32), 0);
}

#[test]
fn call_stops_before_host_fetch_and_resumes_from_saved_state() {
    use qa_guest::core::callbacks::{GuestHostCallback, HookState};
    use qa_guest::core::contracts::{CallbackId, GuestCallResult, GuestCallSignature, NativeCallAbi};

    let mut run = machine(&[0xe8, 0xfb, 0x3f, 0, 0, 0x83, 0xc0, 3, 0xc3]);
    let hooks = std::rc::Rc::new(HookState::new());
    run.with(|_, memory| {
        map(memory, 0x5000, 8, GuestPermissions::ReadExecute, None);
        let host = addr(memory, 0x5000);
        hooks
            .callbacks
            .borrow_mut()
            .bind_entry(
                memory,
                host,
                GuestHostCallback {
                    id: CallbackId::new("test", "host"),
                    signature: GuestCallSignature {
                        abi: NativeCallAbi::Cdecl,
                        parameters: vec![],
                        result: None,
                        variadic: false,
                    },
                    invoke: std::rc::Rc::new(|_, _, _| Ok(GuestCallResult::Void)),
                },
                std::rc::Rc::new(|| true),
            )
            .unwrap();
    });
    run.cpu.set_hook_state(Some(std::rc::Rc::clone(&hooks)));
    let stop = run.run(10);
    assert_eq!(stop.kind(), "host-call");
    run.with(|state, _| assert_eq!(state.instruction_pointer, 0x5000));
    assert_eq!(read(&mut run, GuestRegister::Rsp, GuestIntegerWidth::B32), 0x8ffc);
    let saved = run.with(|_, memory| memory.read_u32(addr(memory, 0x8ffc)).unwrap());
    assert_eq!(saved, 0x1005);
    run.with(|state, _| {
        state
            .registers
            .write(GuestRegister::Rax, GuestIntegerWidth::B32, 39, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rsp, GuestIntegerWidth::B32, 0x9000, false)
            .unwrap();
        state.instruction_pointer = u64::from(saved);
    });
    assert_eq!(run.run(10).kind(), "return");
    assert_eq!(read(&mut run, GuestRegister::Rax, GuestIntegerWidth::B32), 42);
}

#[test]
fn pop_through_esp_addresses_after_increment() {
    let mut run = machine(&[0x8f, 0x04, 0x24]);
    run.with(|_, memory| memory.write_u32(addr(memory, 0x9000), 0x1234_5678).unwrap());
    assert_eq!(run.cpu.run(1, None).kind(), "budget");
    assert_eq!(read(&mut run, GuestRegister::Rsp, GuestIntegerWidth::B32), 0x9004);
    run.with(|_, memory| assert_eq!(memory.read_u32(addr(memory, 0x9004)).unwrap(), 0x1234_5678));
}

#[test]
fn rep_movsb_restarts_at_budgets_and_overlaps() {
    let mut run = machine(&[0xf3, 0xa4, 0xc3]);
    run.with(|state, memory| {
        memory.write(addr(memory, 0x3000), &[7, 8, 9, 10, 11]).unwrap();
        state
            .registers
            .write(GuestRegister::Rsi, GuestIntegerWidth::B32, 0x3000, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rdi, GuestIntegerWidth::B32, 0x3001, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rcx, GuestIntegerWidth::B32, 4, false)
            .unwrap();
    });
    assert_eq!(run.cpu.run(2, Some(run.returned)).kind(), "budget");
    run.with(|state, _| assert_eq!(state.instruction_pointer, 0x1000));
    assert_eq!(read(&mut run, GuestRegister::Rcx, GuestIntegerWidth::B32), 2);
    assert_eq!(run.run(10).kind(), "return");
    run.with(|_, memory| assert_eq!(memory.copy(addr(memory, 0x3000), 5).unwrap(), vec![7, 7, 7, 7, 7]));
}

#[test]
fn repe_cmpsb_mismatch_and_direction_flag() {
    let mut run = machine(&[0xfd, 0xf3, 0xa6, 0xc3]);
    run.with(|state, memory| {
        memory.write(addr(memory, 0x3000), &[1, 2, 3]).unwrap();
        memory.write(addr(memory, 0x3010), &[1, 9, 3]).unwrap();
        state
            .registers
            .write(GuestRegister::Rsi, GuestIntegerWidth::B32, 0x3002, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rdi, GuestIntegerWidth::B32, 0x3012, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rcx, GuestIntegerWidth::B32, 3, false)
            .unwrap();
    });
    assert_eq!(run.run(10).kind(), "return");
    assert_eq!(read(&mut run, GuestRegister::Rcx, GuestIntegerWidth::B32), 1);
    assert_eq!(read(&mut run, GuestRegister::Rsi, GuestIntegerWidth::B32), 0x3000);
    run.with(|state, _| assert!(!state.flags.get(GuestFlag::Zero)));
}

#[test]
fn lock_cmpxchg_and_xadd_share_one_value() {
    let mut run = machine(&[0xf0, 0x0f, 0xb1, 0x0b, 0xf0, 0x0f, 0xc1, 0x13, 0xc3]);
    run.with(|state, memory| {
        state
            .registers
            .write(GuestRegister::Rbx, GuestIntegerWidth::B32, 0x3000, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rax, GuestIntegerWidth::B32, 5, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rcx, GuestIntegerWidth::B32, 20, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rdx, GuestIntegerWidth::B32, 3, false)
            .unwrap();
        memory.write_u32(addr(memory, 0x3000), 5).unwrap();
    });
    assert_eq!(run.run(10).kind(), "return");
    run.with(|_, memory| assert_eq!(memory.read_u32(addr(memory, 0x3000)).unwrap(), 23));
    assert_eq!(read(&mut run, GuestRegister::Rdx, GuestIntegerWidth::B32), 20);
}

#[test]
fn idiv_quotient_remainder_and_error_atomicity() {
    let mut run = machine(&[0xb8, 0xf9, 0xff, 0xff, 0xff, 0x99, 0xb9, 3, 0, 0, 0, 0xf7, 0xf9, 0xc3]);
    assert_eq!(run.run(20).kind(), "return");
    assert_eq!(read(&mut run, GuestRegister::Rax, GuestIntegerWidth::B32), 0xffff_fffe);
    assert_eq!(read(&mut run, GuestRegister::Rdx, GuestIntegerWidth::B32), 0xffff_ffff);
    let mut bad = machine(&[0xf7, 0xf1]);
    write(&mut bad, GuestRegister::Rax, GuestIntegerWidth::B32, 17);
    let result = bad.cpu.run(1, None);
    assert_eq!(result.kind(), "exception");
    assert_eq!(processor_vector(&result), 0);
    bad.with(|state, _| assert_eq!(state.instruction_pointer, 0x1000));
    assert_eq!(read(&mut bad, GuestRegister::Rax, GuestIntegerWidth::B32), 17);
}

#[test]
fn faults_and_unsupported_report_exact_bytes() {
    let mut invalid = machine(&[0xf0, 0x01, 0xc0]);
    let lock = invalid.cpu.run(10, None);
    assert_eq!(lock.kind(), "exception");
    assert_eq!(processor_vector(&lock), 6);
    let mut unsupported = machine(&[0x90, 0x0f, 0xa2]);
    let result = unsupported.cpu.run(10, None);
    match &result {
        GuestExecutionStop::Unsupported {
            instructions,
            instruction,
            ..
        } => {
            assert_eq!(*instructions, 1);
            assert_eq!(instruction.address.offset, 0x1001);
            assert_eq!(instruction.bytes, vec![0x0f, 0xa2]);
        }
        other => panic!("expected unsupported CPUID stop, got {other:?}"),
    }
    let mut fault = machine(&[0x83, 0x03, 0x01]);
    write(&mut fault, GuestRegister::Rbx, GuestIntegerWidth::B32, 0x1000);
    let (flags, before) = fault.with(|state, _| (state.flags.value(), state.registers.checkpoint()));
    assert_eq!(fault.cpu.run(1, None).kind(), "exception");
    fault.with(|state, _| {
        assert_eq!(state.registers.checkpoint(), before);
        assert_eq!(state.flags.value(), flags);
        assert_eq!(state.instruction_pointer, 0x1000);
    });
}

#[test]
fn decoded_x87_uses_shared_numeric_state() {
    let mut run = machine(&[0xd9, 0xe8, 0xd9, 0xe8, 0xde, 0xc1, 0xd9, 0x1b, 0xc3]);
    write(&mut run, GuestRegister::Rbx, GuestIntegerWidth::B32, 0x3000);
    assert_eq!(run.run(10).kind(), "return");
    run.with(|_, memory| assert_eq!(memory.read_f32(addr(memory, 0x3000)).unwrap(), 2.0));
    run.with(|state, _| assert_eq!(state.x87.tag_word, 0xffff));
}

#[test]
fn synthetic_getgameapi_copies_imports_and_returns_export_table() {
    // Synthetic stand-in for the donor's retail LMCTF GetGameAPI case:
    // push esi/edi; copy 44 dwords from the import block argument to the
    // export table; restore callee-saved registers; return the table.
    let mut run = machine(&[
        0x56, 0x57, 0x8b, 0x74, 0x24, 0x0c, 0xbf, 0x00, 0xb0, 0x00, 0x00, 0xb9, 0x2c, 0x00, 0x00, 0x00, 0xf3, 0xa5,
        0x5f, 0x5e, 0xb8, 0x00, 0xb2, 0x00, 0x00, 0xc3,
    ]);
    let imports: Vec<u8> = (0..44 * 4).map(|index| (index * 7 + 11) as u8).collect();
    run.with(|state, memory| {
        map(memory, 0xa000, 0x100, GuestPermissions::ReadWrite, None);
        map(memory, 0xb000, 0x300, GuestPermissions::ReadWrite, None);
        memory.write(addr(memory, 0xa000), &imports).unwrap();
        memory.write_u32(addr(memory, 0xb200), 3).unwrap();
        memory.write_u32(addr(memory, 0xb204), 0x1000).unwrap();
        memory.write_u32(addr(memory, 0xb264), 0x3bc).unwrap();
        memory.write_u32(addr(memory, 0x9004), 0xa000).unwrap();
        state
            .registers
            .write(GuestRegister::Rsi, GuestIntegerWidth::B32, 0xabcdef01, false)
            .unwrap();
        state
            .registers
            .write(GuestRegister::Rdi, GuestIntegerWidth::B32, 0x1234_5678, false)
            .unwrap();
    });
    assert_eq!(run.run(1000).kind(), "return");
    assert_eq!(read(&mut run, GuestRegister::Rax, GuestIntegerWidth::B32), 0xb200);
    assert_eq!(read(&mut run, GuestRegister::Rsi, GuestIntegerWidth::B32), 0xabcdef01);
    assert_eq!(read(&mut run, GuestRegister::Rdi, GuestIntegerWidth::B32), 0x1234_5678);
    run.with(|_, memory| {
        assert_eq!(memory.copy(addr(memory, 0xb000), imports.len()).unwrap(), imports);
        assert_eq!(memory.read_u32(addr(memory, 0xb200)).unwrap(), 3);
        assert_eq!(memory.read_u32(addr(memory, 0xb204)).unwrap(), 0x1000);
        assert_eq!(memory.read_u32(addr(memory, 0xb264)).unwrap(), 0x3bc);
    });
}
