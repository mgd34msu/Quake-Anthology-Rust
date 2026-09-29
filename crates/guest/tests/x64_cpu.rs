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

#[test]
fn synthetic_linked_list_initializer_runs_head_insert() {
    // Synthetic stand-in for the donor's retail-DLL `.text+0` case, which
    // needs ambient artifacts: mov rbx,[A]; mov [B],rbx; lea rcx,[B-0x18];
    // mov [A],rcx; ret with A=0x50000 and B=0x50020.
    let mut f = Fixture::new(&[
        0x48, 0x8b, 0x1d, 0xf9, 0xff, 0x03, 0x00, //
        0x48, 0x89, 0x1d, 0x12, 0x00, 0x04, 0x00, //
        0x48, 0x8d, 0x0d, 0xf3, 0xff, 0x03, 0x00, //
        0x48, 0x89, 0x0d, 0xe4, 0xff, 0x03, 0x00, //
        0xc3,
    ]);
    f.with(|_, memory| {
        map(memory, 0x50000, 0x40, GuestPermissions::ReadWrite, None);
        memory.write_u64(addr(memory, 0x50000), 0x1234_5678_9abc_def0).unwrap();
    });
    assert_eq!(f.run(32).kind(), "return");
    f.with(|_, memory| {
        assert_eq!(memory.read_u64(addr(memory, 0x50020)).unwrap(), 0x1234_5678_9abc_def0);
        assert_eq!(memory.read_u64(addr(memory, 0x50000)).unwrap(), 0x50008);
    });
    assert_eq!(read(&mut f, GuestRegister::Rcx, GuestIntegerWidth::B64), 0x50008);
    assert_eq!(read(&mut f, GuestRegister::Rsp, GuestIntegerWidth::B64), STACK + 8);
}

#[test]
fn synthetic_relocated_image_returns_tables_through_integer_and_sse() {
    // Synthetic stand-in for the donor's retail GetGameAPI case: code at a
    // high canonical base reads a version word and a float through
    // RIP-relative addressing, publishes the version, and accumulates SSE.
    let image = 0xffff_8000_1800_0000u64;
    let code = vec![
        0x8b, 0x05, 0xfa, 0x0f, 0x00, 0x00, // mov eax,[rip+0xffa] (version)
        0x89, 0x05, 0xf4, 0x1f, 0x00, 0x00, // mov [rip+0x1ff4],eax (table)
        0xf3, 0x0f, 0x58, 0x05, 0xf0, 0x0f, 0x00, 0x00, // addss xmm0,[rip+0xff0]
        0xc3,
    ];
    let (mut state, mut memory) = build(&code, image, common::test_module("x64"));
    map(&mut memory, RETURNED, 8, GuestPermissions::ReadExecute, None);
    map(&mut memory, image + 0x1000, 0x10, GuestPermissions::ReadWrite, None);
    map(&mut memory, image + 0x2000, 0x10, GuestPermissions::ReadWrite, None);
    let returned = addr(&memory, RETURNED);
    memory.write_u32(addr(&memory, image + 0x1000), 2023).unwrap();
    memory.write_f32(addr(&memory, image + 0x1004), 0.025).unwrap();
    state.simd.xmm[0..4].copy_from_slice(&1.5f32.to_le_bytes());
    let mut cpu = qa_guest::x64::cpu::X64Cpu::new(state, memory).unwrap();
    let stopped = cpu.run(512, Some(returned));
    assert_eq!(stopped.kind(), "return");
    let (state, memory) = cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B64, false).unwrap(), 2023);
    assert_eq!(memory.read_u32(addr(memory, image + 0x2000)).unwrap(), 2023);
    assert_eq!(f32::from_le_bytes(state.simd.xmm[0..4].try_into().unwrap()), 1.5 + 0.025);
    assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false).unwrap(), STACK + 8);
}

#[test]
fn canonical_address_range_sign_wrap_and_fault_boundaries() {
    use qa_guest::x64::decoder::canonical_address;
    use qa_guest::x86::decoder::X86Error;

    let values: [i128; 16] = [
        0, 1, -1,
        -(1 << 47), -(1 << 47) - 1,
        (1 << 47) - 1, 1 << 47, (1 << 47) + 1,
        0xffff_7fff_ffff_ffff, 0xffff_8000_0000_0000, 0xffff_8000_0000_0001,
        (1 << 64) - 1, 1 << 64, (1 << 64) + 1, -(1 << 64), -(1 << 64) - 1,
    ];
    for value in values {
        for offset in [-1i128, 0, 1] {
            let raw = value.wrapping_add(offset) as u64;
            let high = raw >> 47;
            if high == 0 || high == 0x1ffff {
                assert_eq!(canonical_address(raw).unwrap(), raw, "input {value}+{offset}");
            } else {
                match canonical_address(raw) {
                    Err(X86Error::Fault { vector, detail, .. }) => {
                        assert_eq!(vector, 13);
                        assert_eq!(detail, format!("Noncanonical 48-bit virtual address 0x{raw:x}"));
                    }
                    other => panic!("expected canonical fault for {raw:#x}, got {other:?}"),
                }
            }
        }
    }
}

#[test]
fn instruction_sequence_preserves_fetch_fault_order() {
    use qa_guest::x86::decoder::X86Error;

    for start in [0x7fff_ffff_ffffu64, 0xffff_ffff_ffff_ffff, 0x1000] {
        let (mut state, mut memory) = build(&[0xb8], start, common::test_module("x64"));
        let mut cursor = X64DecodeCursor::new(&mut memory, &mut state, None).unwrap();
        assert_eq!(cursor.opcode(), 0xb8);
        assert_eq!(cursor.bytes(), &[0xb8]);
        match cursor.read_byte() {
            Err(X86Error::Fault { detail, .. }) => {
                assert!(detail.contains("Noncanonical"), "start {start:#x}: {detail}");
            }
            Err(X86Error::Memory { detail, .. }) => {
                let expected = if start == 0xffff_ffff_ffff_ffff { "null" } else { "unmapped" };
                assert!(detail.contains(expected), "start {start:#x}: {detail}");
            }
            other => panic!("expected fetch fault at {start:#x}, got {other:?}"),
        }
        assert_eq!(cursor.bytes(), &[0xb8]);
    }
    for start in [0x1000u64, 0x7fff_ffff_fff1, 0xffff_ffff_ffff_fff1] {
        let (mut state, mut memory) = build(&[0x66; 15], start, common::test_module("x64"));
        match X64DecodeCursor::new(&mut memory, &mut state, None) {
            Err(X86Error::Fault { vector, detail, .. }) => {
                assert_eq!(vector, 13);
                assert!(detail.contains("15 bytes"), "start {start:#x}: {detail}");
            }
            Ok(_) => panic!("expected 15-byte fault at {start:#x}, cursor decoded"),
            Err(other) => panic!("expected 15-byte fault at {start:#x}, got {other:?}"),
        }
    }
}

#[test]
fn committed_store_observers_fire_with_exact_ranges() {
    // The donor reenters the CPU from inside the observer; Rust observers
    // receive only the written ranges, so this ports the fire/unobserve
    // semantics with exact range assertions.
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut f = Fixture::new(&[0x66, 0xc7, 0x03, 0x34, 0x12, 0xb8, 86, 0, 0, 0, 0xc3]);
    let data = f.with(|state, memory| {
        let data = map(memory, 0x50000, 2, GuestPermissions::ReadWrite, None);
        state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        data
    });
    let fires: Rc<RefCell<Vec<(usize, usize)>>> = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&fires);
    let id = f.with(|_, memory| {
        memory
            .observe_writes(data, 2, Box::new(move |ranges| {
                observed.borrow_mut().extend(ranges.iter().map(|range| (range.byte_offset, range.byte_length)));
            }))
            .unwrap()
    });
    for _ in 0..2 {
        f.reset(BASE);
        assert_eq!(f.run(100).kind(), "return");
        assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 86);
        f.with(|_, memory| assert_eq!(memory.read_u16(data).unwrap(), 0x1234));
    }
    assert_eq!(*fires.borrow(), vec![(0, 2), (0, 2)]);
    f.with(|_, memory| memory.unobserve(id));
    f.reset(BASE);
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(fires.borrow().len(), 2);
}

fn host_callback() -> qa_guest::core::callbacks::GuestHostCallback {
    use qa_guest::core::callbacks::GuestHostCallback;
    use qa_guest::core::contracts::{CallbackId, GuestCallResult, GuestCallSignature, NativeCallAbi};

    GuestHostCallback {
        id: CallbackId::new("test", "plan-hook"),
        signature: GuestCallSignature {
            abi: NativeCallAbi::MicrosoftX64,
            parameters: vec![],
            result: None,
            variadic: false,
        },
        invoke: std::rc::Rc::new(|_, _, _| Ok(GuestCallResult::Void)),
    }
}

#[test]
fn retained_blocks_requalify_entries_and_gates() {
    // Register/IP-mutating observers have no Rust equivalent (hook
    // callbacks receive no state); firing, gating, and budget
    // continuation port exactly.
    use std::cell::Cell;
    use std::rc::Rc;

    use qa_guest::core::callbacks::HookState;

    let mut f = Fixture::new(&[0xb8, 1, 0, 0, 0, 0x83, 0xc0, 2, 0x83, 0xc0, 3, 0xc3]);
    let hooks = Rc::new(HookState::new());
    f.cpu.set_hook_state(Some(Rc::clone(&hooks)));
    let run = |f: &mut Fixture, budget: u64| {
        f.reset(BASE);
        f.run(budget)
    };
    for _ in 0..3 {
        assert_eq!(run(&mut f, 100).kind(), "return");
        assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 6);
    }
    let entries = Rc::new(Cell::new(0u32));
    let notify = Rc::clone(&entries);
    let entry = addr_of(&mut f, BASE + 5);
    let observer = f.with(|_, memory| {
        hooks.callbacks.borrow_mut().observe_entry(memory, entry, Rc::new(move || notify.set(notify.get() + 1))).unwrap()
    });
    assert_eq!(run(&mut f, 100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 6);
    assert_eq!(entries.get(), 1);
    hooks.callbacks.borrow_mut().unobserve_entry(entry, observer);
    assert_eq!(run(&mut f, 100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 6);
    assert_eq!(entries.get(), 1);
    let accepts = Rc::new(Cell::new(false));
    let gate = Rc::clone(&accepts);
    f.with(|_, memory| {
        hooks.callbacks.borrow_mut().bind_entry(memory, entry, host_callback(), Rc::new(move || gate.get())).unwrap();
    });
    assert_eq!(run(&mut f, 100).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 6);
    accepts.set(true);
    let stopped = run(&mut f, 100);
    assert_eq!(stopped.kind(), "host-call");
    assert_eq!(stopped.instructions(), 1);
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 1);
    hooks.callbacks.borrow_mut().unhook_entry(entry);
    f.reset(BASE);
    assert_eq!(f.run(2).kind(), "budget");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 3);
    f.with(|state, _| assert_eq!(state.instruction_pointer, BASE + 8));
    assert_eq!(f.run(2).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 6);
    let returns = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&returns);
    let ret = addr_of(&mut f, BASE + 11);
    let watcher = f.with(|_, memory| {
        hooks.callbacks.borrow_mut().observe_entry(memory, ret, Rc::new(move || counted.set(counted.get() + 1))).unwrap()
    });
    assert_eq!(run(&mut f, 100).kind(), "return");
    assert_eq!(returns.get(), 1);
    hooks.callbacks.borrow_mut().unobserve_entry(ret, watcher);
    assert_eq!(run(&mut f, 100).kind(), "return");
    assert_eq!(returns.get(), 1);
}

fn addr_of(f: &mut Fixture, offset: u64) -> qa_guest::core::contracts::GuestAddress {
    f.with(|_, memory| addr(memory, offset))
}

#[test]
fn raw_simd_preserves_scalar_lanes_overlaps_and_stores() {
    let mut f = Fixture::new(&[
        0xf3, 0x0f, 0x10, 0xc1, 0xf2, 0x0f, 0x10, 0x03, //
        0x66, 0x0f, 0x28, 0xd0, 0x66, 0x0f, 0x6f, 0xda, //
        0xf3, 0x0f, 0x7f, 0x5b, 17, 0x0f, 0x57, 0xdb, //
        0x66, 0x0f, 0xeb, 0xd8, 0x66, 0x0f, 0xdb, 0xda, //
        0x0f, 0x55, 0xd8, 0x0f, 0x29, 0x5b, 32, 0xc3,
    ]);
    let data = f.with(|_, memory| {
        let data = map(memory, 0x50000, 64, GuestPermissions::ReadWrite, None);
        memory.write(data, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]).unwrap();
        data
    });
    let mut expected = [0u8; 16];
    expected[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    for _ in 0..2 {
        f.with(|state, _| {
            state.instruction_pointer = BASE;
            state.registers.write(GuestRegister::Rsp, GuestIntegerWidth::B64, STACK, false).unwrap();
            state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
            state.simd.xmm[0..16].fill(0xaa);
            state.simd.xmm[16..32].fill(0xbb);
            state.flags.set_value(0x8d7);
            state.simd.mxcsr = 0x5fa0;
        });
        assert_eq!(f.run(1).kind(), "budget");
        f.with(|state, _| {
            let mut lanes = [0xaau8; 16];
            lanes[0..4].fill(0xbb);
            assert_eq!(state.simd.xmm[0..16], lanes);
        });
        assert_eq!(f.run(1).kind(), "budget");
        f.with(|state, _| assert_eq!(state.simd.xmm[0..16], expected));
        assert_eq!(f.run(3).kind(), "budget");
        f.with(|_, memory| {
            assert_eq!(memory.copy(memory.offset(data, 17).unwrap(), 16).unwrap(), expected);
        });
        assert_eq!(f.run(6).kind(), "return");
        f.with(|state, memory| {
            assert_eq!(memory.copy(memory.offset(data, 32).unwrap(), 16).unwrap(), vec![0u8; 16]);
            assert_eq!(state.flags.value(), 0x8d7);
            assert_eq!(state.simd.mxcsr, 0x5fa0);
        });
    }
}

#[test]
fn simd_alignment_fault_before_write_and_live_bytes() {
    let mut f = Fixture::new(&[0x0f, 0x28, 0x03, 0xc3]);
    f.with(|_, memory| {
        map(memory, 0x50000, 32, GuestPermissions::ReadWrite, Some(vec![0x5a; 32]));
    });
    let run = |f: &mut Fixture, offset: u64| {
        f.with(|state, _| {
            state.instruction_pointer = BASE;
            state.registers.write(GuestRegister::Rsp, GuestIntegerWidth::B64, STACK, false).unwrap();
            state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000 + offset, false).unwrap();
        });
        f.run(100)
    };
    assert_eq!(run(&mut f, 0).kind(), "return");
    f.with(|state, _| state.simd.xmm[0..16].fill(0xab));
    let misaligned = run(&mut f, 1);
    assert_eq!(misaligned.kind(), "exception");
    assert_eq!(processor_vector(&misaligned), 13);
    f.with(|state, _| assert_eq!(state.simd.xmm[0..16], [0xabu8; 16]));
    f.with(|_, memory| {
        let code = addr(memory, BASE);
        let alias = memory.map_alias(0x60000, 4, GuestPermissions::ReadWrite, "alias", code).unwrap();
        memory.write_u8(memory.offset(alias, 1).unwrap(), 0x10).unwrap();
    });
    assert_eq!(run(&mut f, 1).kind(), "return");
    f.with(|state, _| assert_eq!(state.simd.xmm[0..16], [0x5au8; 16]));
    f.with(|state, _| state.simd.xmm[0..16].fill(0xcd));
    assert_eq!(run(&mut f, 24).kind(), "exception");
    f.with(|state, _| assert_eq!(state.simd.xmm[0..16], [0xcdu8; 16]));
}

#[test]
fn integer_kernels_match_source_alu_flags_and_halves() {
    use qa_guest::x64::decoder::{X64MemoryOperand, X64Operand, X64RegisterOperand, X64Segment};
    use qa_guest::x64::integer_kernel::X64IntegerKernel;
    use qa_guest::x64::plan::{execute_x64_plan, make_x64_plan, X64PlanOperation, X64PlanSource};
    use qa_guest::x86::arithmetic::AluOperation;

    let operations = [
        AluOperation::Add, AluOperation::Adc, AluOperation::Sub, AluOperation::Sbb,
        AluOperation::Cmp, AluOperation::And, AluOperation::Test, AluOperation::Or,
        AluOperation::Xor,
    ];
    let widths = [GuestIntegerWidth::B8, GuestIntegerWidth::B16, GuestIntegerWidth::B32, GuestIntegerWidth::B64];
    let mut source = Fixture::new(&[0x90]);
    let mut compiled = Fixture::new(&[0x90]);
    let mut kernel = X64IntegerKernel::new();
    for width in widths {
        let bits = width.bits();
        let maximum = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
        let sign = 1u64 << (bits - 1);
        let edges = [
            (maximum, 1u64),
            (sign - 1, 1),
            (sign, maximum),
            (0, 1),
            (0x1234_5678_8765_4321 & maximum, 0x8765_4321_1234_5678 & maximum),
        ];
        for operation in operations {
            for (left, right) in edges {
                let flag_sets: &[u64] = if matches!(operation, AluOperation::Adc | AluOperation::Sbb) {
                    &[0x9876_5432_abcdefd6, 0x9876_5432_abcdefd7]
                } else {
                    &[0x9876_5432_abcdefd7]
                };
                for initial in flag_sets {
                    let plan = make_x64_plan(
                        X64PlanOperation::Alu {
                            operation,
                            destination: X64Operand::Register(X64RegisterOperand {
                                register: GuestRegister::Rax,
                                width,
                                high_byte: width == GuestIntegerWidth::B8,
                            }),
                            source: X64PlanSource::Operand(X64Operand::Register(
                                X64RegisterOperand { register: GuestRegister::Rbx, width, high_byte: false },
                            )),
                        },
                        BASE + 1,
                        false,
                    );
                    let prepared = qa_guest::x64::integer_kernel::prepare_x64_integer_plan(&plan)
                        .expect("integer plan prepares");
                    for f in [&mut source, &mut compiled] {
                        f.with(|state, _| {
                            state.flags.set_value(*initial);
                            state.registers.write(GuestRegister::Rax, GuestIntegerWidth::B64, 0xfedc_ba98_7654_3210, false).unwrap();
                            state.registers.write(GuestRegister::Rax, width, left, width == GuestIntegerWidth::B8).unwrap();
                            state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, right, false).unwrap();
                        });
                    }
                    let (source_state, source_memory) = source.cpu.parts();
                    execute_x64_plan(&plan, source_memory, source_state).unwrap();
                    let (compiled_state, compiled_memory) = compiled.cpu.parts();
                    kernel.execute(&prepared, compiled_state, compiled_memory).unwrap();
                    let (source_state, _) = source.cpu.parts();
                    let expected_registers = source_state.registers.checkpoint();
                    let expected_flags = source_state.flags.value();
                    let (compiled_state, _) = compiled.cpu.parts();
                    assert_eq!(compiled_state.registers.checkpoint(), expected_registers);
                    assert_eq!(compiled_state.flags.value(), expected_flags);
                }
            }
        }
    }
    let addresses = [
        X64MemoryOperand {
            width: GuestIntegerWidth::B64,
            base: Some(GuestRegister::Rax),
            index: Some(GuestRegister::Rbx),
            scale: 8,
            displacement: -17,
            rip_relative: false,
            address_bits: 64,
            segment: Some(X64Segment::Fs),
        },
        X64MemoryOperand {
            width: GuestIntegerWidth::B32,
            base: Some(GuestRegister::Rax),
            index: Some(GuestRegister::Rbx),
            scale: 4,
            displacement: 0x8000_0000,
            rip_relative: false,
            address_bits: 32,
            segment: None,
        },
        X64MemoryOperand {
            width: GuestIntegerWidth::B64,
            base: None,
            index: None,
            scale: 1,
            displacement: -0x1_0000_0007,
            rip_relative: true,
            address_bits: 64,
            segment: None,
        },
    ];
    for address in addresses {
        let plan = make_x64_plan(
            X64PlanOperation::Lea {
                destination: X64RegisterOperand { register: GuestRegister::R8, width: address.width, high_byte: false },
                source: address,
            },
            BASE + 7,
            false,
        );
        let prepared = qa_guest::x64::integer_kernel::prepare_x64_integer_plan(&plan).expect("LEA prepares");
        for f in [&mut source, &mut compiled] {
            f.with(|state, _| {
                state.registers.write(GuestRegister::Rax, GuestIntegerWidth::B64, 0xffff_8000_1234_5678, false).unwrap();
                state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0xffff_ffff_fffedcba & 0xffff_ffff_ffff_ffff, false).unwrap();
                state.segments[qa_guest::core::registers::GuestProcessorState::FS].base = 0x1234_5678_90;
            });
        }
        let (source_state, source_memory) = source.cpu.parts();
        execute_x64_plan(&plan, source_memory, source_state).unwrap();
        let (compiled_state, compiled_memory) = compiled.cpu.parts();
        kernel.execute(&prepared, compiled_state, compiled_memory).unwrap();
        let (source_state, _) = source.cpu.parts();
        let expected = source_state.registers.checkpoint();
        let (compiled_state, _) = compiled.cpu.parts();
        assert_eq!(compiled_state.registers.checkpoint(), expected);
    }
    for code in 0..16u8 {
        for flags in [0u64, 0x8d5, 0x881, 0x44] {
            let plan = make_x64_plan(
                X64PlanOperation::Branch { condition: Some(code), displacement: -3 },
                BASE + 7,
                false,
            );
            let prepared = qa_guest::x64::integer_kernel::prepare_x64_integer_plan(&plan).expect("branch prepares");
            source.with(|state, _| state.flags.set_value(flags));
            compiled.with(|state, _| state.flags.set_value(flags));
            let (source_state, source_memory) = source.cpu.parts();
            let expected = execute_x64_plan(&plan, source_memory, source_state);
            let (compiled_state, compiled_memory) = compiled.cpu.parts();
            assert_eq!(kernel.execute(&prepared, compiled_state, compiled_memory), expected);
        }
    }
}

#[test]
fn warmed_branch_preserves_noncanonical_target_fault() {
    // The donor compares generic and managed engines; this port has one
    // engine, so a warmed run is compared against a cold run instead.
    let location = 0x7fff_ffff_ffe0u64;
    let bytes = [0x0f, 0x85, 0x30, 0, 0, 0, 0xc3];
    let mut warm = Fixture::at(&bytes, location);
    warm.with(|state, _| state.flags.set(GuestFlag::Zero, true));
    assert_eq!(warm.run(100).kind(), "return");
    warm.reset(location);
    warm.with(|state, _| state.flags.set(GuestFlag::Zero, false));
    let mut cold = Fixture::at(&bytes, location);
    cold.with(|state, _| state.flags.set(GuestFlag::Zero, false));
    let actual = warm.run(100);
    let expected = cold.run(100);
    assert_eq!(actual.kind(), "exception");
    assert_eq!(expected.kind(), "exception");
    assert_eq!(actual.instructions(), 0);
    assert_eq!(expected.instructions(), 0);
    warm.with(|state, _| assert_eq!(state.instruction_pointer, location));
    let warm_checkpoint = warm.with(|state, _| (state.registers.checkpoint(), state.flags.value()));
    let cold_checkpoint = cold.with(|state, _| (state.registers.checkpoint(), state.flags.value()));
    assert_eq!(warm_checkpoint, cold_checkpoint);
}

#[test]
fn numeric_instructions_read_live_operands_and_fault_boundaries() {
    let mut sse = Fixture::new(&[0xf3, 0x0f, 0x58, 0x03, 0xc3]);
    let input = sse.with(|state, memory| {
        let input = map(memory, 0x50000, 4, GuestPermissions::ReadWrite, None);
        state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        input
    });
    for value in [2.25f32, 3.5, -1.5] {
        sse.reset(BASE);
        sse.with(|state, memory| {
            memory.write_f32(input, value).unwrap();
            state.simd.xmm[0..4].copy_from_slice(&1.5f32.to_le_bytes());
        });
        assert_eq!(sse.run(100).kind(), "return");
        sse.with(|state, _| {
            assert_eq!(f32::from_le_bytes(state.simd.xmm[0..4].try_into().unwrap()), 1.5 + value);
        });
    }
    // The donor uses execute-only mappings here; this port has no
    // execute-only form, so the read fault is exercised with no access.
    sse.with(|_, memory| memory.protect(input, 4, GuestPermissions::None).unwrap());
    sse.reset(BASE);
    let fault = sse.run(100);
    assert_eq!(fault.kind(), "exception");
    assert_eq!(fault.instructions(), 0);
    sse.with(|state, _| {
        assert_eq!(f32::from_le_bytes(state.simd.xmm[0..4].try_into().unwrap()), 0.0);
        assert_eq!(state.instruction_pointer, BASE);
    });
    let mut x87 = Fixture::new(&[0xd9, 0x03, 0xd9, 0x5b, 4, 0xc3]);
    let values = x87.with(|state, memory| {
        let values = map(memory, 0x50000, 8, GuestPermissions::ReadWrite, None);
        state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        values
    });
    for value in [2.25f32, -3.5, 7.5] {
        x87.reset(BASE);
        x87.with(|_, memory| memory.write_f32(values, value).unwrap());
        assert_eq!(x87.run(100).kind(), "return");
        x87.with(|_, memory| {
            assert_eq!(memory.read_f32(memory.offset(values, 4).unwrap()).unwrap(), value);
        });
    }
    x87.with(|_, memory| {
        memory.protect(memory.offset(values, 4).unwrap(), 4, GuestPermissions::Read).unwrap();
    });
    x87.reset(BASE);
    let fault = x87.run(100);
    assert_eq!(fault.kind(), "exception");
    assert_eq!(fault.instructions(), 1);
    x87.with(|state, _| assert_eq!(state.instruction_pointer, BASE + 2));
}

#[test]
fn returns_preserve_stack_adjustment_and_precise_faults() {
    for discard in [0u8, 32] {
        let bytes: Vec<u8> = if discard == 0 { vec![0xc3] } else { vec![0xc2, discard, 0] };
        let mut f = Fixture::new(&bytes);
        for _ in 0..2 {
            f.reset(BASE);
            assert_eq!(f.run(1).kind(), "return");
            assert_eq!(read(&mut f, GuestRegister::Rsp, GuestIntegerWidth::B64), STACK + 8 + u64::from(discard));
        }
        f.reset(BASE);
        f.with(|_, memory| memory.write_u64(addr(memory, STACK), 0x8000_0000_0000).unwrap());
        let target = f.run(100);
        assert_eq!(target.kind(), "exception");
        assert_eq!(target.instructions(), 0);
        f.with(|state, _| {
            assert_eq!(state.instruction_pointer, BASE);
            assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false).unwrap(), STACK);
        });
        f.with(|_, memory| memory.unmap(addr(memory, STACK + 4), 4).unwrap());
        let stack = f.run(100);
        assert_eq!(stack.kind(), "exception");
        assert_eq!(stack.instructions(), 0);
        f.with(|state, _| {
            assert_eq!(state.instruction_pointer, BASE);
            assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false).unwrap(), STACK);
        });
    }
}

#[test]
fn warm_blocks_match_cold_budgets_registers_and_read_faults() {
    // MOV EAX,7; ADD EAX,5; MOV ECX,[RBX]; ADC EAX,ECX; RET.
    let bytes = [0xb8, 7, 0, 0, 0, 0x83, 0xc0, 5, 0x8b, 0x0b, 0x11, 0xc8, 0xc3];
    let mut cold = Fixture::new(&bytes);
    let mut warm = Fixture::new(&bytes);
    for f in [&mut cold, &mut warm] {
        f.with(|state, memory| {
            let data = map(memory, 0x50000, 4, GuestPermissions::ReadWrite, None);
            memory.write_u32(data, 30).unwrap();
            state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        });
        assert_eq!(f.run(100).kind(), "return");
    }
    for budget in [1, 2, 3, 4, 5] {
        for f in [&mut cold, &mut warm] {
            f.reset(BASE);
            f.with(|state, _| state.flags.set_value(0x1234_5678_0000_0003));
        }
        let expected = cold.run(budget);
        let actual = warm.run(budget);
        assert_eq!(actual.kind(), expected.kind());
        assert_eq!(actual.instructions(), expected.instructions());
        let (cold_ip, cold_regs, cold_flags) =
            cold.with(|state, _| (state.instruction_pointer, state.registers.checkpoint(), state.flags.value()));
        let (warm_ip, warm_regs, warm_flags) =
            warm.with(|state, _| (state.instruction_pointer, state.registers.checkpoint(), state.flags.value()));
        assert_eq!(warm_ip, cold_ip);
        assert_eq!(warm_regs, cold_regs);
        assert_eq!(warm_flags, cold_flags);
    }
    for f in [&mut cold, &mut warm] {
        f.with(|state, _| {
            state.instruction_pointer = BASE;
            state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0, false).unwrap();
            state.registers.write(GuestRegister::Rcx, GuestIntegerWidth::B64, 123, false).unwrap();
            state.registers.write(GuestRegister::Rsp, GuestIntegerWidth::B64, STACK, false).unwrap();
            state.flags.set_value(0x1234_5678_0000_0003);
        });
    }
    let expected = cold.run(100);
    let actual = warm.run(100);
    assert_eq!(actual.kind(), "exception");
    assert_eq!(actual.instructions(), 2);
    assert_eq!(expected.kind(), "exception");
    warm.with(|state, _| assert_eq!(state.instruction_pointer, BASE + 8));
    let (cold_regs, cold_flags) =
        cold.with(|state, _| (state.registers.checkpoint(), state.flags.value()));
    let (warm_regs, warm_flags) =
        warm.with(|state, _| (state.registers.checkpoint(), state.flags.value()));
    assert_eq!(warm_regs, cold_regs);
    assert_eq!(warm_flags, cold_flags);
    assert_eq!(read(&mut warm, GuestRegister::Rax, GuestIntegerWidth::B32), 12);
    assert_eq!(read(&mut warm, GuestRegister::Rcx, GuestIntegerWidth::B32), 123);
}

#[test]
fn block_boundaries_observe_stores_entries_and_aliases() {
    // Two read-only instructions, then a store boundary, then two read-only
    // instructions. Code patches apply from the test body: hook callbacks
    // receive no processor state in this port.
    use std::cell::Cell;
    use std::rc::Rc;

    use qa_guest::core::callbacks::HookState;

    let mut f = Fixture::new(&[
        0xb8, 1, 0, 0, 0, 0x83, 0xc0, 2, 0x89, 0x03, 0x83, 0xc0, 3, 0x83, 0xc0, 4, 0xc3,
    ]);
    let data = f.with(|state, memory| {
        let data = map(memory, 0x50000, 4, GuestPermissions::ReadWrite, None);
        state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        data
    });
    let hooks = Rc::new(HookState::new());
    f.cpu.set_hook_state(Some(Rc::clone(&hooks)));
    let run = |f: &mut Fixture| {
        f.reset(BASE);
        f.run(100)
    };
    assert_eq!(run(&mut f).kind(), "return");
    assert_eq!(run(&mut f).kind(), "return");
    let code = f.with(|_, memory| {
        memory.map_alias(0x60000, 17, GuestPermissions::ReadWrite, "alias", addr(memory, BASE)).unwrap()
    });
    let observed = Rc::new(Cell::new(0u32));
    let fired = Rc::clone(&observed);
    let watcher = f.with(|_, memory| {
        memory.observe_writes(data, 4, Box::new(move |_| fired.set(fired.get() + 1))).unwrap()
    });
    f.with(|_, memory| memory.write_u8(memory.offset(code, 12).unwrap(), 8).unwrap());
    assert_eq!(run(&mut f).kind(), "return");
    assert_eq!(observed.get(), 1);
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 15);
    f.with(|_, memory| memory.unobserve(watcher));
    let entries = Rc::new(Cell::new(0u32));
    let counted = Rc::clone(&entries);
    let entry = addr_of(&mut f, BASE + 13);
    let observer = f.with(|_, memory| {
        hooks.callbacks.borrow_mut().observe_entry(memory, entry, Rc::new(move || counted.set(counted.get() + 1))).unwrap()
    });
    f.with(|_, memory| memory.write_u8(memory.offset(code, 15).unwrap(), 9).unwrap());
    assert_eq!(run(&mut f).kind(), "return");
    assert_eq!(entries.get(), 1);
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 20);
    hooks.callbacks.borrow_mut().unobserve_entry(entry, observer);
    assert_eq!(run(&mut f).kind(), "return");
    assert_eq!(read(&mut f, GuestRegister::Rax, GuestIntegerWidth::B32), 20);
    f.with(|_, memory| memory.protect(addr(memory, BASE), 17, GuestPermissions::Read).unwrap());
    f.reset(BASE);
    let fault = f.run(100);
    assert_eq!(fault.kind(), "exception");
    assert_eq!(fault.instructions(), 0);
}

#[test]
fn store_observers_report_ranges_and_unobserve_cleanly() {
    // Rust observers are infallible and stateless, so the donor's throwing
    // mid-instruction observer ports as range/unobserve semantics.
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut f = Fixture::new(&[0xb8, 1, 0, 0, 0, 0x83, 0xc0, 2, 0x89, 0x03, 0xc3]);
    let data = f.with(|state, memory| {
        let data = map(memory, 0x50000, 4, GuestPermissions::ReadWrite, None);
        state.registers.write(GuestRegister::Rbx, GuestIntegerWidth::B64, 0x50000, false).unwrap();
        data
    });
    assert_eq!(f.run(100).kind(), "return");
    let fires: Rc<RefCell<Vec<(usize, usize)>>> = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::clone(&fires);
    let id = f.with(|_, memory| {
        memory
            .observe_writes(data, 4, Box::new(move |ranges| {
                observed.borrow_mut().extend(ranges.iter().map(|range| (range.byte_offset, range.byte_length)));
            }))
            .unwrap()
    });
    f.reset(BASE);
    assert_eq!(f.run(100).kind(), "return");
    assert_eq!(*fires.borrow(), vec![(0, 4)]);
    f.with(|state, memory| {
        assert_eq!(state.instruction_pointer, RETURNED);
        assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B64, false).unwrap(), 3);
        assert_eq!(memory.read_u32(data).unwrap(), 3);
    });
    f.with(|_, memory| memory.unobserve(id));
    fires.borrow_mut().clear();
    f.reset(BASE);
    assert_eq!(f.run(100).kind(), "return");
    assert!(fires.borrow().is_empty());
}
