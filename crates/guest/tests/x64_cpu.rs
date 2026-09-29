//! x86-64 CPU integration tests: synthetic fixtures with exact
//! register/memory assertions.
//!
//! Donor: `tests/guest/x64/cpu.test.ts`. Retail-DLL cases are replaced by
//! synthetic equivalents (no ambient artifacts); hook-observer cases that
//! mutate processor state from inside callbacks are adapted to the Rust
//! hook API, which passes no state into observers.

mod common;

use qa_guest::abi::GuestCpu;
use qa_guest::core::contracts::{
    GuestAccess, GuestException, GuestExecutionStop, GuestFlag, GuestIntegerWidth, GuestRegister,
};
use qa_guest::x64::decoder::X64DecodeCursor;

use common::{addr, build, map, Fixture, BASE, RETURNED, STACK};
use qa_guest::core::contracts::GuestPermissions;

fn read(f: &mut Fixture, register: GuestRegister, width: GuestIntegerWidth) -> u64 {
    f.with(|state, _| {
        state.registers.read(register, width, false).unwrap()
    })
}

fn write(f: &mut Fixture, register: GuestRegister, width: GuestIntegerWidth, value: u64) {
    f.with(|state, _| {
        state.registers.write(register, width, value, false).unwrap();
    });
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
fn cpuid_baseline_without_host_capabilities() {
    let mut baseline = Fixture::new(&[0xb8, 1, 0, 0, 0, 0x0f, 0xa2, 0xc3]);
    baseline.with(|state, _| state.flags.set_value(0x8d7));
    assert_eq!(baseline.run(100).kind(), "return");
    assert_eq!(read(&mut baseline, GuestRegister::Rdx, GuestIntegerWidth::B64), 0x0600_8101);
    assert_eq!(read(&mut baseline, GuestRegister::Rcx, GuestIntegerWidth::B64), 0);
    assert_eq!(baseline.with(|state, _| state.flags.value()), 0x8d7);
    let mut structured = Fixture::new(&[0xb8, 7, 0, 0, 0, 0x0f, 0xa2, 0xc3]);
    assert_eq!(structured.run(100).kind(), "return");
    assert_eq!(read(&mut structured, GuestRegister::Rbx, GuestIntegerWidth::B64), 0);
    let mut locked = Fixture::new(&[0xf0, 0x0f, 0xa2]);
    let stopped = locked.run(100);
    assert_eq!(stopped.kind(), "exception");
    assert_eq!(processor_vector(&stopped), 6);
}

#[test]
fn decoded_instructions_observe_live_operands_aliases_and_retirement() {
    let mut f = Fixture::new(&[0x8b, 0x43, 4, 0x83, 0xc0, 1, 0xc3]);
    f.with(|_, memory| {
        let data = map(memory, 0x50000, 16, GuestPermissions::ReadWrite, None);
        memory.write_u32(memory.offset(data, 4).unwrap(), 10).unwrap();
        memory.write_u32(memory.offset(data, 8).unwrap(), 20).unwrap();
    });
    let run = |f: &mut Fixture, offset: u64| -> u64 {
        f.with(|state, _| {
            state.instruction_pointer = BASE;
            state.registers.write(GuestRegister::Rsp, GuestIntegerWidth::B64, STACK, false).unwrap();
            state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000 + offset, false).unwrap();
        });
        assert_eq!(f.run(100).kind(), "return");
        read(f, GuestRegister::Rax, GuestIntegerWidth::B32)
    };
    assert_eq!(run(&mut f, 0), 11);
    assert_eq!(run(&mut f, 4), 21);
    // Writes through a writable alias change the decoded instruction bytes.
    f.with(|_, memory| {
        let code = addr(memory, BASE);
        let alias = memory.map_alias(0x60000, 7, GuestPermissions::ReadWrite, "alias", code).unwrap();
        memory.write_u8(memory.offset(alias, 5).unwrap(), 7).unwrap();
    });
    assert_eq!(run(&mut f, 0), 17);
    f.with(|state, memory| {
        let code = addr(memory, BASE);
        memory.protect(code, 7, GuestPermissions::Read).unwrap();
        state.instruction_pointer = BASE;
    });
    assert_eq!(f.run(100).kind(), "exception");
    f.with(|_, memory| {
        let code = addr(memory, BASE);
        memory.unmap(code, 7).unwrap();
        map(memory, BASE, 6, GuestPermissions::ReadExecute, Some(vec![0xb8, 99, 0, 0, 0, 0xc3]));
    });
    assert_eq!(run(&mut f, 0), 99);
}

#[test]
fn immediate_decoding_retains_widths_and_partial_fault_bytes() {
    for width in [1usize, 2, 4, 8] {
        let mut bytes = vec![0x90];
        bytes.extend(std::iter::repeat_n(0xff, width));
        let mut f = Fixture::new(&bytes);
        f.with(|state, memory| {
            let mut cursor = X64DecodeCursor::new(memory, state, None).unwrap();
            let maximum = if width == 8 { u64::MAX } else { (1u64 << (width * 8)) - 1 };
            assert_eq!(cursor.read_unsigned(width).unwrap(), maximum);
            assert_eq!(cursor.next_ip(), BASE + (width as u64) + 1);
            assert_eq!(cursor.bytes(), bytes.as_slice());
        });
    }
    let mut signed = Fixture::new(&[0x90, 0xff, 0xff, 0xff, 0xff]);
    signed.with(|state, memory| {
        let mut cursor = X64DecodeCursor::new(memory, state, None).unwrap();
        assert_eq!(cursor.read_signed(4).unwrap(), -1);
    });
    let mut truncated = Fixture::new(&[0x90, 0x12, 0x34]);
    truncated.with(|state, memory| {
        let mut cursor = X64DecodeCursor::new(memory, state, None).unwrap();
        assert!(cursor.read_unsigned(4).is_err());
        assert_eq!(cursor.bytes(), &[0x90, 0x12, 0x34]);
    });
    let mut bytes = vec![0x90];
    bytes.extend(std::iter::repeat_n(0xff, 15));
    let mut long = Fixture::new(&bytes);
    long.with(|state, memory| {
        let mut cursor = X64DecodeCursor::new(memory, state, None).unwrap();
        cursor.read_unsigned(8).unwrap();
        cursor.read_unsigned(4).unwrap();
        let error = cursor.read_unsigned(4).unwrap_err();
        assert!(format!("{error:?}").contains("15 bytes"), "unexpected: {error:?}");
        assert_eq!(cursor.bytes().len(), 15);
    });
}

#[test]
fn mov_widths_preserve_aliases_and_zero_extend() {
    let mut f = Fixture::new(&[
        0x48, 0xb8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xb4, 0x12, 0x66, 0xb8, 0x34,
        0x56, 0xc3,
    ]);
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 0xffff_ffff_ffff_5634);
    let mut dword = Fixture::new(&[0xb8, 0xef, 0xcd, 0xab, 0x89, 0xc3]);
    write(&mut dword, GuestRegister::Rax, GuestIntegerWidth::B64, 0xffff_ffff_ffff_ffff);
    assert_eq!(dword.run(100).kind(), "return");
    assert_eq!(read(&mut dword, GuestRegister::Rax, GuestIntegerWidth::B64), 0x89ab_cdef);
}

#[test]
fn rex_aliases_and_prefix_order_select_spl_vs_ah() {
    let mut rex = Fixture::new(&[0x40, 0xb4, 0x80]);
    assert_eq!(rex.run(1).kind(), "budget");
    assert_eq!(read(&mut rex, GuestRegister::Rsp, GuestIntegerWidth::B64), 0x30080);
    let mut overridden = Fixture::new(&[0x40, 0x66, 0xb4, 0x12, 0xc3]);
    assert_eq!(overridden.run(100).kind(), "return");
    assert_eq!(read(&mut overridden, GuestRegister::Rax, GuestIntegerWidth::B64), 0x1200);
}

#[test]
fn extended_registers_and_sib_beyond_number_precision() {
    let mut f = Fixture::new(&[0x4f, 0x89, 0x4c, 0xa5, 0xf8, 0x4f, 0x8b, 0x54, 0xa5, 0xf8, 0xc3]);
    let address = 0xffff_8000_0001_0000u64;
    f.with(|state, memory| {
        map(memory, address, 64, GuestPermissions::ReadWrite, None);
        state.registers.write(GuestRegister::R13, GuestIntegerWidth::B64, address, false).unwrap();
        state.registers.write(GuestRegister::R12, GuestIntegerWidth::B64, 4, false).unwrap();
        state.registers.write(GuestRegister::R9, GuestIntegerWidth::B64, 0xfedc_ba98_7654_3210, false).unwrap();
    });
    assert_eq!(f.run(100).kind(), "return");
    f.with(|_, memory| {
        assert_eq!(memory.read_u64(addr(memory, address + 8)).unwrap(), 0xfedc_ba98_7654_3210);
    });
    assert_eq!(read(&mut f, GuestRegister::R10, GuestIntegerWidth::B64), 0xfedc_ba98_7654_3210);
}

#[test]
fn rip_relative_store_and_address_override_truncation() {
    let mut store = Fixture::new(&[0xc7, 0x05, 0xf6, 0xff, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0xc3]);
    store.with(|_, memory| {
        map(memory, 0x20000, 4, GuestPermissions::ReadWrite, None);
    });
    assert_eq!(store.run(100).kind(), "return");
    store.with(|_, memory| {
        assert_eq!(memory.read_u32(addr(memory, 0x20000)).unwrap(), 0x1234_5678);
    });
    let mut low = Fixture::at(&[0x67, 0x8b, 0x05, 0xf9, 0xff, 0x00, 0x00, 0xc3], 0x1_0001_0000);
    low.with(|_, memory| {
        map(memory, 0x20000, 4, GuestPermissions::ReadWrite, Some(vec![42, 0, 0, 0]));
    });
    assert_eq!(low.run(100).kind(), "return");
    assert_eq!(read(&mut low, GuestRegister::Rax, GuestIntegerWidth::B64), 42);
}

#[test]
fn sib_no_base_and_fs_addressing_with_lea_exclusion() {
    let mut f = Fixture::new(&[
        0x64, 0x48, 0x8b, 0x04, 0x25, 0x08, 0, 0, 0, 0x64, 0x48, 0x8d, 0x0c, 0x25, 0x08, 0, 0, 0,
        0xc3,
    ]);
    f.with(|state, memory| {
        map(memory, 0x50000, 32, GuestPermissions::ReadWrite, None);
        state.segments[qa_guest::core::registers::GuestProcessorState::FS].base = 0x50000;
        memory.write_u64(addr(memory, 0x50008), 0x1234_5678_9abc_def0).unwrap();
    });
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 0x1234_5678_9abc_def0);
    assert_eq!(read(&mut f, GuestRegister::Rcx, GuestIntegerWidth::B64), 8);
}

#[test]
fn integer_flags_carry_chains_and_width_masks() {
    let mut f = Fixture::new(&[0x48, 0x83, 0xc0, 1, 0x49, 0x83, 0xd0, 0, 0xc3]);
    write(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64, 0xffff_ffff_ffff_ffff);
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 0);
    assert_eq!(read(&mut f, GuestRegister::R8, GuestIntegerWidth::B64), 1);
    let mut overflow = Fixture::new(&[0x48, 0x83, 0xc0, 1, 0xc3]);
    write(&mut overflow, GuestRegister::Rax, GuestIntegerWidth::B64, 0x7fff_ffff_ffff_ffff);
    assert_eq!(overflow.run(100).kind(), "return");
    overflow.with(|state, _| {
        assert!(state.flags.get(GuestFlag::Overflow));
        assert!(!state.flags.get(GuestFlag::Carry));
        assert!(state.flags.get(GuestFlag::Sign));
    });
}

#[test]
fn call_ret_and_backward_jcc_loop() {
    let mut f = Fixture::new(&[
        0xb9, 3, 0, 0, 0, 0xe8, 1, 0, 0, 0, 0xc3, 0x48, 0x83, 0xc0, 2, 0xff, 0xc9, 0x75, 0xf8, 0xc3,
    ]);
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 6);
    assert_eq!(read(&mut f, GuestRegister::Rsp, GuestIntegerWidth::B64), STACK + 8);
}

#[test]
fn indirect_call_stops_at_host_trap_with_return_on_stack() {
    use qa_guest::core::callbacks::{GuestHostCallback, HookState};
    use qa_guest::core::contracts::{
        CallbackId, GuestCallResult, GuestCallSignature, NativeCallAbi,
    };
    use qa_guest::x64::cpu::X64Cpu;

    let (mut state, mut memory) = build(&[0xff, 0xd0, 0xc3], BASE, common::test_module("x64"));
    map(&mut memory, RETURNED, 8, GuestPermissions::ReadExecute, None);
    map(&mut memory, 0x60000, 8, GuestPermissions::ReadExecute, None);
    let returned = addr(&memory, RETURNED);
    state.registers.write(GuestRegister::Rax, GuestIntegerWidth::B64, 0x60000, false).unwrap();
    let hooks = std::rc::Rc::new(HookState::new());
    let host = addr(&memory, 0x60000);
    hooks
        .callbacks
        .borrow_mut()
        .bind_entry(
            &mut memory,
            host,
            GuestHostCallback {
                id: CallbackId::new("test", "host"),
                signature: GuestCallSignature {
                    abi: NativeCallAbi::MicrosoftX64,
                    parameters: vec![],
                    result: None,
                    variadic: false,
                },
                invoke: std::rc::Rc::new(|_, _, _| Ok(GuestCallResult::Void)),
            },
            std::rc::Rc::new(|| true),
        )
        .unwrap();
    let mut cpu = X64Cpu::new(state, memory).unwrap();
    cpu.set_hook_state(Some(std::rc::Rc::clone(&hooks)));
    let stopped = cpu.run(10, Some(returned));
    assert_eq!(stopped.kind(), "host-call");
    let (state, memory) = cpu.parts();
    assert_eq!(state.instruction_pointer, 0x60000);
    assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false).unwrap(), STACK - 8);
    assert_eq!(memory.read_u64(addr(memory, STACK - 8)).unwrap(), BASE + 2);
}

#[test]
fn mul_div_128bit_dividend_and_idiv_fault_atomicity() {
    let mut multiplied = Fixture::new(&[0x48, 0xf7, 0xe1, 0x48, 0xf7, 0xf1, 0xc3]);
    write(&mut multiplied, GuestRegister::Rax, GuestIntegerWidth::B64, 0xffff_ffff_ffff_ffff);
    write(&mut multiplied, GuestRegister::Rcx, GuestIntegerWidth::B64, 2);
    assert_eq!(multiplied.run(100).kind(), "return");
    assert_eq!(read(&mut multiplied, GuestRegister::Rax, GuestIntegerWidth::B64), 0xffff_ffff_ffff_ffff);
    assert_eq!(read(&mut multiplied, GuestRegister::Rdx, GuestIntegerWidth::B64), 0);
    let mut divided = Fixture::new(&[0x48, 0xf7, 0xf9]);
    write(&mut divided, GuestRegister::Rax, GuestIntegerWidth::B64, 0x8000_0000_0000_0000);
    write(&mut divided, GuestRegister::Rdx, GuestIntegerWidth::B64, 0xffff_ffff_ffff_ffff);
    write(&mut divided, GuestRegister::Rcx, GuestIntegerWidth::B64, 0xffff_ffff_ffff_ffff);
    let stopped = divided.run(100);
    assert_eq!(stopped.kind(), "exception");
    assert_eq!(processor_vector(&stopped), 0);
    divided.with(|state, _| {
        assert_eq!(state.instruction_pointer, BASE);
        assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B64, false).unwrap(), 0x8000_0000_0000_0000);
    });
}

#[test]
fn rep_movsb_overlapping_copy_and_budget_resume() {
    let mut f = Fixture::new(&[0xf3, 0xa4, 0xc3]);
    f.with(|state, memory| {
        map(memory, 0x50000, 8, GuestPermissions::ReadWrite, Some(vec![1, 2, 3, 4, 5]));
        state.registers.write(GuestRegister::Rsi, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        state.registers.write(GuestRegister::Rdi, GuestIntegerWidth::B64, 0x50001, false).unwrap();
        state.registers.write(GuestRegister::Rcx, GuestIntegerWidth::B64, 4, false).unwrap();
    });
    assert_eq!(f.run(2).kind(), "budget");
    f.with(|state, _| {
        assert_eq!(state.instruction_pointer, BASE);
        assert_eq!(state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B64, false).unwrap(), 2);
    });
    assert_eq!(f.run(100).kind(), "return");
    f.with(|_, memory| {
        assert_eq!(memory.copy(addr(memory, 0x50000), 5).unwrap(), vec![1, 1, 1, 1, 1]);
    });
}

#[test]
fn repe_cmpsb_mismatch_positions() {
    let mut f = Fixture::new(&[0xf3, 0xa6, 0xc3]);
    f.with(|state, memory| {
        map(memory, 0x50000, 16, GuestPermissions::ReadWrite, Some(vec![1, 2, 3, 0, 1, 9, 3]));
        state.registers.write(GuestRegister::Rsi, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        state.registers.write(GuestRegister::Rdi, GuestIntegerWidth::B64, 0x50004, false).unwrap();
        state.registers.write(GuestRegister::Rcx, GuestIntegerWidth::B64, 3, false).unwrap();
    });
    assert_eq!(f.run(100).kind(), "return");
    f.with(|state, _| {
        assert_eq!(state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B64, false).unwrap(), 1);
        assert_eq!(state.registers.read(GuestRegister::Rsi, GuestIntegerWidth::B64, false).unwrap(), 0x50002);
        assert!(!state.flags.get(GuestFlag::Zero));
    });
}

#[test]
fn xadd_freezes_address_before_exchange() {
    let mut f = Fixture::new(&[0x48, 0x0f, 0xc1, 0x00, 0xc3]);
    f.with(|state, memory| {
        map(memory, 0x50000, 8, GuestPermissions::ReadWrite, None);
        memory.write_u64(addr(memory, 0x50000), 7).unwrap();
        state.registers.write(GuestRegister::Rax, GuestIntegerWidth::B64, 0x50000, false).unwrap();
    });
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 7);
    f.with(|_, memory| {
        assert_eq!(memory.read_u64(addr(memory, 0x50000)).unwrap(), 0x50007);
    });
}

#[test]
fn memory_write_fault_rolls_back_and_reports_address() {
    let mut f = Fixture::new(&[0x48, 0x83, 0x00, 1]);
    f.with(|state, memory| {
        map(memory, 0x50000, 8, GuestPermissions::Read, Some(vec![7]));
        state.registers.write(GuestRegister::Rax, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        state.flags.set_value(0x803);
    });
    let stopped = f.run(100);
    assert_eq!(stopped.kind(), "exception");
    match &stopped {
        GuestExecutionStop::Exception {
            exception: GuestException::Memory { address, access, .. },
            ..
        } => {
            assert_eq!(address.offset, 0x50000);
            assert_eq!(*access, GuestAccess::Write);
        }
        other => panic!("expected memory exception, got {other:?}"),
    }
    f.with(|state, memory| {
        assert_eq!(state.flags.value(), 0x803);
        assert_eq!(state.instruction_pointer, BASE);
        assert_eq!(memory.read_u64(addr(memory, 0x50000)).unwrap(), 7);
    });
}

#[test]
fn sequential_runs_keep_independent_rollback_snapshots() {
    // The donor nests a run inside a host-call hook; the Rust runner owns
    // the CPU, so the equivalent sequencing (outer step, checkpoint,
    // nested region, restore, faulting step) runs in the open.
    let mut f = Fixture::new(&[0xb8, 1, 0, 0, 0, 0x0f, 0x0b]);
    let nested = BASE + 0x1000;
    f.with(|_, memory| {
        map(memory, nested, 11, GuestPermissions::ReadExecute, Some(vec![0xb8, 2, 0, 0, 0, 0xbb, 3, 0, 0, 0, 0xf4]));
    });
    assert_eq!(f.run(1).kind(), "budget");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 1);
    let checkpoint = f.with(|state, _| state.registers.checkpoint());
    f.with(|state, _| state.instruction_pointer = nested);
    assert_eq!(f.run(2).kind(), "budget");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B64), 2);
    assert_eq!(read(&mut f, GuestRegister::Rbx, GuestIntegerWidth::B64), 3);
    f.with(|state, _| {
        state.registers.restore(&checkpoint).unwrap();
        state.instruction_pointer = BASE + 5;
    });
    let stopped = f.run(100);
    assert_eq!(stopped.kind(), "exception");
    assert_eq!(stopped.instructions(), 0);
    assert_eq!(processor_vector(&stopped), 6);
    f.with(|state, _| {
        assert_eq!(state.instruction_pointer, BASE + 5);
        assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B64, false).unwrap(), 1);
        assert_eq!(state.registers.read(GuestRegister::Rbx, GuestIntegerWidth::B64, false).unwrap(), 0);
    });
}

#[test]
fn unsupported_opcode_and_noncanonical_preserve_rip() {
    let mut invalid = Fixture::new(&[0x0f, 0x05]);
    let stopped = invalid.run(100);
    assert_eq!(stopped.kind(), "unsupported");
    match &stopped {
        GuestExecutionStop::Unsupported { instruction, .. } => {
            assert_eq!(instruction.bytes, vec![0x0f, 0x05]);
            assert_eq!(instruction.address.offset, BASE);
        }
        other => panic!("expected unsupported stop, got {other:?}"),
    }
    let mut noncanonical = Fixture::new(&[0x48, 0x8b, 0x00]);
    write(&mut noncanonical, GuestRegister::Rax, GuestIntegerWidth::B64, 0x0000_8000_0000_0000);
    let fault = noncanonical.run(100);
    assert_eq!(fault.kind(), "exception");
    assert_eq!(processor_vector(&fault), 13);
    noncanonical.with(|state, _| assert_eq!(state.instruction_pointer, BASE));
}

#[test]
fn wait_dispatches_x87_and_preserves_pending_exception() {
    let mut normal = Fixture::new(&[0x9b, 0xc3]);
    assert_eq!(normal.run(100).kind(), "return");
    let mut pending = Fixture::new(&[0x9b, 0xc3]);
    pending.with(|state, _| {
        state.x87.control_word &= !1;
        state.x87.status_word = 1;
    });
    let stopped = pending.run(100);
    assert_eq!(stopped.kind(), "exception");
    assert_eq!(processor_vector(&stopped), 16);
    pending.with(|state, _| {
        assert_eq!(state.instruction_pointer, BASE);
        assert_eq!(state.x87.status_word, 1);
    });
}
