//! Floating-point execution tests: x87 extended arithmetic, rounding
//! modes, SSE lanes, and exact binary helpers.
//!
//! Donor: `tests/guest/floating-point/execution.test.ts`. The retail-math
//! suite needs user-supplied binaries outside the source boundary.

mod common;

use qa_guest::core::contracts::{GuestAddress, GuestArchitecture, GuestIntegerWidth, GuestPermissions, GuestRegister};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use qa_guest::floating_point::binary::{
    arithmetic, decode_binary, decode_binary32, encode_binary, from_integer, read_bits,
    square_root, write_bits, zero, BigInt, BinaryOperation, BinaryValue, BinaryWidth, Rounding,
    BINARY32, BINARY64, BINARY80,
};
use qa_guest::floating_point::contracts::{
    NumericExecutionContext, NumericExecutionResult, NumericInstruction, NumericOperand, NumericPrefix,
};
use qa_guest::floating_point::x87::{push_x87, read_x87_register, read_x87_return, write_x87_return};
use qa_guest::floating_point::execute_numeric_instruction;

use common::{map, test_module};

struct Fixture {
    state: GuestProcessorState,
    memory: SparseGuestMemory,
    address: GuestAddress,
}

fn bits_u64(value: &BigInt) -> u64 {
    let bytes = value.to_bytes_le(8);
    u64::from_le_bytes(bytes.try_into().unwrap())
}

fn bits_u128(value: &BigInt) -> u128 {
    let bytes = value.to_bytes_le(16);
    u128::from_le_bytes(bytes.try_into().unwrap())
}

fn fixture() -> Fixture {
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::X86_64,
        instruction_pointer: 0x1000,
        stack_pointer: 0x3000,
        flags: 0,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    let mut memory = SparseGuestMemory::new(test_module("floating-point"), 8, 0x10000).unwrap();
    let address = map(&mut memory, 0x2000, 256, GuestPermissions::ReadWrite, None);
    Fixture { state, memory, address }
}

fn run(
    fixture: &mut Fixture,
    opcode: u8,
    modrm: u8,
    secondary: Option<u8>,
    prefix: NumericPrefix,
    immediate: Option<u8>,
) -> NumericExecutionResult {
    let instruction = NumericInstruction {
        opcode,
        secondary_opcode: secondary,
        modrm: Some(modrm),
        operand: Some(if modrm >= 0xc0 {
            NumericOperand::Register((modrm & 7) as usize)
        } else {
            NumericOperand::Memory(fixture.address)
        }),
        register_index: ((modrm >> 3) & 7) as usize,
        prefix,
        operand_bits: 32,
        immediate,
    };
    execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction,
    })
    .unwrap()
}

fn set_xmm(fixture: &mut Fixture, index: usize, bits: u128) {
    let bytes = write_bits(&BigInt::from_u128(bits), 16);
    fixture.state.simd.xmm[index * 16..index * 16 + 16].copy_from_slice(&bytes);
}

fn get_xmm(fixture: &Fixture, index: usize) -> u128 {
    bits_u128(&read_bits(&fixture.state.simd.xmm[index * 16..index * 16 + 16]))
}

fn get_xmm_low(fixture: &Fixture, index: usize) -> u64 {
    bits_u64(&read_bits(&fixture.state.simd.xmm[index * 16..index * 16 + 4]))
}

#[test]
fn integer_arithmetic_preserves_x87_64_bit_significand() {
    let mut fixture = fixture();
    fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u64(1 << 53), 8)).unwrap();
    assert_eq!(run(&mut fixture, 0xdf, 0x28, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    assert_eq!(run(&mut fixture, 0xd9, 0xe8, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    assert_eq!(run(&mut fixture, 0xde, 0xc1, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    let value = read_x87_register(&mut fixture.state.x87, 0).unwrap();
    assert_eq!(bits_u128(&encode_binary(&value, BinaryWidth::W80)), 0x4034_8000_0000_0000_0400);
    fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u64(0x4340_0000_0000_0000), 8)).unwrap();
    assert_eq!(run(&mut fixture, 0xdc, 0x20, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    assert_eq!(read_x87_return(&mut fixture.state.x87, BinaryWidth::W64).unwrap(), 1.0);
}

#[test]
fn precision_control_clears_low_significand_bits_only_at_arithmetic() {
    for (control, expected, flags) in [(0x37f, 16_777_217.0, 0), (0x7f, 16_777_216.0, 32), (0x87f, 16_777_218.0, 32)] {
        let mut fixture = fixture();
        fixture.state.x87.control_word = control;
        write_x87_return(&mut fixture.state.x87, 16_777_216.0, BinaryWidth::W64).unwrap();
        assert_eq!(run(&mut fixture, 0xd9, 0xe8, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        assert_eq!(run(&mut fixture, 0xde, 0xc1, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        assert_eq!(read_x87_return(&mut fixture.state.x87, BinaryWidth::W64).unwrap(), expected);
        assert_eq!(fixture.state.x87.status_word & 32, flags);
    }
}

#[test]
fn x87_float_stores_round_midpoint_and_preserve_negative_zero() {
    for (mode, expected) in [(0u16, 0x3f80_0000u32), (1, 0x3f80_0000), (2, 0x3f80_0001), (3, 0x3f80_0000)] {
        let mut fixture = fixture();
        fixture.state.x87.control_word = 0x37f | (mode << 10);
        write_x87_return(&mut fixture.state.x87, 1.0 + 2f64.powi(-24), BinaryWidth::W64).unwrap();
        assert_eq!(run(&mut fixture, 0xd9, 0x18, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        let stored = fixture.memory.copy(fixture.address, 4).unwrap();
        assert_eq!(bits_u64(&read_bits(&stored)), u64::from(expected));
        assert_eq!(fixture.state.x87.tag_word, 0xffff);
    }
    let mut fixture = fixture();
    write_x87_return(&mut fixture.state.x87, -0.0, BinaryWidth::W64).unwrap();
    assert_eq!(run(&mut fixture, 0xd9, 0xfa, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    let returned = read_x87_return(&mut fixture.state.x87, BinaryWidth::W64).unwrap();
    assert!(returned == 0.0 && returned.is_sign_negative());
}

#[test]
fn x87_raw80_load_store_preserves_signaling_nan_payload() {
    let mut fixture = fixture();
    let bits = 0xffff_8000_0000_0000_0123u128;
    fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u128(bits), 10)).unwrap();
    assert_eq!(run(&mut fixture, 0xdb, 0x28, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    assert_eq!(fixture.state.x87.status_word & 1, 0);
    assert_eq!(run(&mut fixture, 0xdb, 0x38, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    let stored = fixture.memory.copy(fixture.address, 10).unwrap();
    assert_eq!(bits_u128(&read_bits(&stored)), bits);
}

#[test]
fn x87_raw80_transfer_preserves_noncanonical_encodings() {
    for bits in [0x0000_8000_0000_0000_0001u128, 0x4000_0000_0000_0000_0123u128] {
        let mut fixture = fixture();
        fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u128(bits), 10)).unwrap();
        assert_eq!(run(&mut fixture, 0xdb, 0x28, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        assert_eq!(run(&mut fixture, 0xdb, 0x38, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        let stored = fixture.memory.copy(fixture.address, 10).unwrap();
        assert_eq!(bits_u128(&read_bits(&stored)), bits);
    }
}

#[test]
fn unmasked_x87_range_exceptions_wrap_exponents_and_defer_notification() {
    for (input, mask, group, expected) in [
        (0x7ffe_8000_0000_0000_0000u128, 8u16, 0x08u8, 0x1fff_8000_0000_0000_0000u128),
        (0x0001_8000_0000_0000_0000u128, 16u16, 0x30u8, 0x6000_8000_0000_0000_0000u128),
    ] {
        let mut fixture = fixture();
        fixture.state.x87.control_word &= !mask;
        fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u128(input), 10)).unwrap();
        assert_eq!(run(&mut fixture, 0xdb, 0x28, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u64(0x4000_0000), 4)).unwrap();
        assert_eq!(run(&mut fixture, 0xd8, group, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
        let value = read_x87_register(&mut fixture.state.x87, 0).unwrap();
        assert_eq!(bits_u128(&encode_binary(&value, BinaryWidth::W80)), expected);
        assert_eq!(fixture.state.x87.status_word & (0x8080 | mask | 32), 0x8080 | mask);
    }
}

#[test]
fn unmasked_x87_store_overflow_leaves_memory_and_top_unchanged() {
    let mut fixture = fixture();
    write_x87_return(&mut fixture.state.x87, 2f64.powi(128), BinaryWidth::W64).unwrap();
    fixture.state.x87.control_word &= !8;
    fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u64(0x1234_5678), 4)).unwrap();
    let top = fixture.state.x87.status_word & 0x3800;
    assert_eq!(run(&mut fixture, 0xd9, 0x18, None, NumericPrefix::None, None), NumericExecutionResult::Executed);
    assert_eq!(fixture.memory.read_u32(fixture.address).unwrap(), 0x1234_5678);
    assert_eq!(fixture.state.x87.status_word & 0x3800, top);
    assert_eq!(fixture.state.x87.status_word & (8 | 32 | 0x200), 8);
}

#[test]
fn x87_stack_fault_defers_until_waiting_instruction() {
    let mut fixture = fixture();
    fixture.state.x87.control_word &= !1;
    let before = fixture.state.x87.registers;
    let instruction = NumericInstruction {
        opcode: 0xd8,
        secondary_opcode: None,
        modrm: Some(0xc1),
        operand: Some(NumericOperand::Register(1)),
        register_index: 0,
        prefix: NumericPrefix::None,
        operand_bits: 32,
        immediate: None,
    };
    let outcome = execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction,
    })
    .unwrap();
    assert_eq!(outcome, NumericExecutionResult::Executed);
    assert_eq!(fixture.state.x87.status_word & 0x80c1, 0x80c1);
    assert_eq!(fixture.state.x87.registers, before);
    assert_eq!(fixture.state.x87.tag_word, 0xffff);
    let waiting = NumericInstruction { opcode: 0x9b, modrm: None, operand: None, ..instruction };
    let outcome = execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction: waiting,
    })
    .unwrap();
    assert_eq!(
        outcome,
        NumericExecutionResult::Exception { vector: 16, detail: "Pending x87 floating-point exception".to_string() }
    );
}

#[test]
fn exact_divide_and_square_root_round_correctly() {
    let one = from_integer(&BigInt::one());
    let three = from_integer(&BigInt::from_u64(3));
    let quotient = arithmetic(BinaryOperation::Divide, &one, &three, BINARY64, Rounding::Nearest, false).unwrap();
    assert_eq!(bits_u64(&encode_binary(&quotient.value, BinaryWidth::W64)), 0x3fd5_5555_5555_5555);
    let two = from_integer(&BigInt::from_u64(2));
    let root = square_root(&two, BINARY64, Rounding::Nearest).unwrap();
    assert_eq!(bits_u64(&encode_binary(&root.value, BinaryWidth::W64)), 0x3ff6_a09e_667f_3bcd);
    let big = from_integer(&BigInt::from_u64(1 << 63));
    let extended = arithmetic(BinaryOperation::Add, &big, &one, BINARY80, Rounding::Nearest, false).unwrap();
    assert_eq!(bits_u128(&encode_binary(&extended.value, BinaryWidth::W80)), 0x403e_8000_0000_0000_0001);
}

#[test]
fn sse_scalar_move_retains_upper_lanes_while_load_clears_them() {
    let mut fixture = fixture();
    set_xmm(&mut fixture, 0, 0xaaaa_aaaa_aaaa_aaaa_bbbb_bbbb_cccc_cccc);
    set_xmm(&mut fixture, 1, 0x1111_1111_2222_2222_3333_3333_3f80_0000);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x10), NumericPrefix::XF3, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm(&fixture, 0), 0xaaaa_aaaa_aaaa_aaaa_bbbb_bbbb_3f80_0000);
    fixture.memory.write(fixture.address, &write_bits(&BigInt::from_u64(0x8000_0000), 4)).unwrap();
    assert_eq!(run(&mut fixture, 0x0f, 0x00, Some(0x10), NumericPrefix::XF3, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm(&fixture, 0), 0x8000_0000);
}

#[test]
fn sse_unaligned_movups_works_while_movaps_faults_first() {
    let mut fixture = fixture();
    let unaligned = fixture.memory.offset(fixture.address, 1).unwrap();
    fixture.memory.write(unaligned, &vec![0xab; 16]).unwrap();
    let instruction = NumericInstruction {
        opcode: 0x0f,
        secondary_opcode: Some(0x10),
        modrm: Some(0),
        operand: Some(NumericOperand::Memory(unaligned)),
        register_index: 0,
        prefix: NumericPrefix::None,
        operand_bits: 32,
        immediate: None,
    };
    let outcome = execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction,
    })
    .unwrap();
    assert_eq!(outcome, NumericExecutionResult::Executed);
    assert_eq!(get_xmm(&fixture, 0), 0xabab_abab_abab_abab_abab_abab_abab_abab);
    let aligned = NumericInstruction { secondary_opcode: Some(0x28), ..instruction };
    let outcome = execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction: aligned,
    })
    .unwrap();
    assert!(matches!(outcome, NumericExecutionResult::Exception { .. }), "{outcome:?}");
    assert_eq!(get_xmm(&fixture, 0), 0xabab_abab_abab_abab_abab_abab_abab_abab);
}

#[test]
fn sse_uses_mxcsr_rounding_and_preserves_first_source_nan() {
    let mut fixture = fixture();
    fixture.state.simd.mxcsr = 0x5f80;
    set_xmm(&mut fixture, 0, 0x3f80_0000);
    set_xmm(&mut fixture, 1, 0x3380_0000);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x58), NumericPrefix::XF3, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm_low(&fixture, 0), 0x3f80_0001);
    assert_eq!(fixture.state.simd.mxcsr & 32, 32);
    set_xmm(&mut fixture, 0, 0xffc1_2345);
    set_xmm(&mut fixture, 1, 0x7fc5_4321);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x58), NumericPrefix::XF3, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm_low(&fixture, 0), 0xffc1_2345);
}

#[test]
fn sse_exceptions_aggregate_before_packed_destination_changes() {
    let mut fixture = fixture();
    set_xmm(&mut fixture, 0, 0x3f80_0000_3f80_0000_3f80_0000_3f80_0000);
    set_xmm(&mut fixture, 1, 0x3f80_0000_3f80_0000_0000_0000_3f80_0000);
    fixture.state.simd.mxcsr &= !(1 << 9);
    let before = get_xmm(&fixture, 0);
    let outcome = run(&mut fixture, 0x0f, 0xc1, Some(0x5e), NumericPrefix::None, None);
    assert_eq!(
        outcome,
        NumericExecutionResult::Exception { vector: 19, detail: "Unmasked SIMD floating-point exception".to_string() }
    );
    assert_eq!(get_xmm(&fixture, 0), before);
    assert_eq!(fixture.state.simd.mxcsr & 4, 4);
}

#[test]
fn sse_daz_turns_signed_denormal_into_signed_zero() {
    let mut fixture = fixture();
    fixture.state.simd.mxcsr |= 64;
    set_xmm(&mut fixture, 0, 0x8000_0001);
    set_xmm(&mut fixture, 1, 0x3f80_0000);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x59), NumericPrefix::XF3, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm_low(&fixture, 0), 0x8000_0000);
    assert_eq!(fixture.state.simd.mxcsr & 2, 0);
}

#[test]
fn sse_integer_conversion_truncates_and_returns_indefinite() {
    let mut fixture = fixture();
    set_xmm(&mut fixture, 1, 0xbff3_3333_3333_3333);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x2c), NumericPrefix::XF2, None), NumericExecutionResult::Executed);
    assert_eq!(fixture.state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), 0xffff_ffff);
    set_xmm(&mut fixture, 1, 0x7ff0_0000_0000_0000);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x2c), NumericPrefix::XF2, None), NumericExecutionResult::Executed);
    assert_eq!(fixture.state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), 0x8000_0000);
    assert_eq!(fixture.state.simd.mxcsr & 1, 1);
}

#[test]
fn packed_integer_lanes_wrap_and_compare_yields_masks() {
    let mut fixture = fixture();
    set_xmm(&mut fixture, 0, 0x7fff_ffff_0000_0000_ffff_ffff_ffff_ffff);
    set_xmm(&mut fixture, 1, 0x0000_0001_0000_0001_0000_0001_0000_0001);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0xfe), NumericPrefix::X66, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm(&fixture, 0), 0x8000_0000_0000_0001_0000_0000_0000_0000);
    assert_eq!(run(&mut fixture, 0x0f, 0xc1, Some(0x76), NumericPrefix::X66, None), NumericExecutionResult::Executed);
    assert_eq!(get_xmm(&fixture, 0), 0x0000_0000_ffff_ffff_0000_0000_0000_0000);
}

#[test]
fn binary32_keeps_exact_subnormals_and_cancellation_signs() {
    let smallest = decode_binary(&BigInt::one(), BinaryWidth::W32);
    let sum = arithmetic(BinaryOperation::Add, &smallest, &smallest, BINARY32, Rounding::Nearest, true).unwrap();
    assert_eq!(bits_u64(&encode_binary(&sum.value, BinaryWidth::W32)), 2);
    assert_eq!(sum.flags, 2);
    let one = from_integer(&BigInt::one());
    let cancelled = arithmetic(BinaryOperation::Subtract, &one, &one, BINARY80, Rounding::Down, false).unwrap();
    assert_eq!(bits_u128(&encode_binary(&cancelled.value, BinaryWidth::W80)), 1 << 79);
}

#[test]
fn binary32_word_decoding_matches_exact_words() {
    for bits in [0u32, 1, 0x007f_ffff, 0x0080_0000, 0x3f7f_ffff, 0x3f80_0000, 0x3f80_0001, 0x7f7f_ffff,
        0x7f80_0000, 0x7f80_0001, 0x7fbfffff, 0x7fc0_0000, 0x7fc1_2345, 0x7fff_ffff]
    {
        for sign in [0u32, 0x8000_0000] {
            let word = bits | sign;
            assert_eq!(decode_binary32(word), decode_binary(&BigInt::from_u64(u64::from(word)), BinaryWidth::W32));
        }
    }
}

#[test]
fn binary_value_kind_queries_cover_signs() {
    let value = from_integer(&BigInt::one());
    let _ = format!("{value:?}");
    assert!(!value.negative());
    assert!(zero(true).negative());
}

struct Trig {
    state: GuestProcessorState,
    memory: SparseGuestMemory,
}

fn trig_fixture(value: &BinaryValue, control: u16) -> Trig {
    let mut state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::I386,
        instruction_pointer: 0,
        stack_pointer: 0,
        flags: 0,
        x87_control_word: control,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    push_x87(&mut state.x87, value).unwrap();
    let memory = SparseGuestMemory::new(test_module("x87-trig"), 4, 0x10000).unwrap();
    Trig { state, memory }
}

fn trig_run(fixture: &mut Trig, cosine: bool) -> NumericExecutionResult {
    let instruction = NumericInstruction {
        opcode: 0xd9,
        secondary_opcode: None,
        modrm: Some(if cosine { 0xff } else { 0xfe }),
        operand: Some(NumericOperand::Register(if cosine { 7 } else { 6 })),
        register_index: 7,
        prefix: NumericPrefix::None,
        operand_bits: 32,
        immediate: None,
    };
    execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction,
    })
    .unwrap()
}

fn trig_bits(fixture: &mut Trig) -> u128 {
    let value = read_x87_register(&mut fixture.state.x87, 0).unwrap();
    bits_u128(&encode_binary(&value, BinaryWidth::W80))
}

#[test]
fn x87_trig_preserves_signed_zero_full_precision_and_pi_reduction() {
    let mut negative_zero = trig_fixture(&zero(true), 0x37f);
    assert_eq!(trig_run(&mut negative_zero, false), NumericExecutionResult::Executed);
    assert_eq!(trig_bits(&mut negative_zero), 0x8000_0000_0000_0000_0000);
    let mut cosine_zero = trig_fixture(&zero(false), 0x37f);
    assert_eq!(trig_run(&mut cosine_zero, true), NumericExecutionResult::Executed);
    assert_eq!(trig_bits(&mut cosine_zero), 0x3fff_8000_0000_0000_0000);
    let mut pi = trig_fixture(&decode_binary(&BigInt::from_u128(0x4000_c90f_daa2_2168_c235), BinaryWidth::W80), 0x37f);
    assert_eq!(trig_run(&mut pi, false), NumericExecutionResult::Executed);
    assert_eq!(trig_bits(&mut pi), 0xbfbf_8000_0000_0000_0000);
    let mut one = trig_fixture(&from_integer(&BigInt::one()), 0x7f);
    assert_eq!(trig_run(&mut one, false), NumericExecutionResult::Executed);
    assert_eq!(trig_bits(&mut one), 0x3ffe_d76a_a478_4867_7021);
    assert_eq!(one.state.x87.status_word & 32, 32);
}

#[test]
fn x87_trig_range_and_exceptional_operands_follow_deferred_state() {
    for magnitude in [BigInt::from_u64(1 << 63), BigInt::from_i64(i64::MIN)] {
        let mut fixture = trig_fixture(&from_integer(&magnitude), 0x37f);
        let bits = trig_bits(&mut fixture);
        assert_eq!(trig_run(&mut fixture, false), NumericExecutionResult::Executed);
        assert_eq!(trig_bits(&mut fixture), bits);
        assert_eq!(fixture.state.x87.status_word & 0x400, 0x400);
    }
    let mut infinite = trig_fixture(&BinaryValue::Infinity { negative: false }, 0x37f);
    assert_eq!(trig_run(&mut infinite, false), NumericExecutionResult::Executed);
    assert_eq!(infinite.state.x87.status_word & 1, 1);
    assert!(matches!(read_x87_register(&mut infinite.state.x87, 0).unwrap(), BinaryValue::Nan { .. }));
    let mut unmasked = trig_fixture(&BinaryValue::Infinity { negative: false }, 0x37e);
    assert_eq!(trig_run(&mut unmasked, false), NumericExecutionResult::Executed);
    assert!(matches!(read_x87_register(&mut unmasked.state.x87, 0).unwrap(), BinaryValue::Infinity { .. }));
    assert_eq!(unmasked.state.x87.status_word & 0x8081, 0x8081);
    assert!(matches!(trig_run(&mut unmasked, false), NumericExecutionResult::Exception { .. }));
    let mut denormal = trig_fixture(&decode_binary(&BigInt::one(), BinaryWidth::W80), 0x37f);
    assert_eq!(trig_run(&mut denormal, false), NumericExecutionResult::Executed);
    assert_eq!(denormal.state.x87.status_word & 2, 2);
    assert_eq!(trig_bits(&mut denormal), 1);
}

#[test]
fn x87_cosine_tiny_inputs_obey_directed_rounding() {
    for exponent in [-128, -1000] {
        let value = BinaryValue::Finite {
            negative: false,
            coefficient: BigInt::one(),
            exponent,
            denormal: false,
        };
        for mode in [0u16, 1, 2, 3] {
            let mut fixture = trig_fixture(&value, 0x37f | (mode << 10));
            assert_eq!(trig_run(&mut fixture, true), NumericExecutionResult::Executed);
            assert_eq!(
                trig_bits(&mut fixture),
                if mode == 1 || mode == 3 { 0x3ffe_ffff_ffff_ffff_ffff } else { 0x3fff_8000_0000_0000_0000 }
            );
        }
    }
}

#[test]
fn fldpi_rounds_architectural_constant_without_precision() {
    for mode in [0u16, 1, 2, 3] {
        let mut state = GuestProcessorState::create(GuestProcessorInitialState {
            architecture: GuestArchitecture::I386,
            instruction_pointer: 0,
            stack_pointer: 0,
            flags: 0,
            x87_control_word: 0x7f | (mode << 10),
            mxcsr: 0x1f80,
            mxcsr_mask: 0xffff,
        })
        .unwrap();
        let mut memory = SparseGuestMemory::new(test_module("x87-trig"), 4, 0x10000).unwrap();
        let instruction = NumericInstruction {
            opcode: 0xd9,
            secondary_opcode: None,
            modrm: Some(0xeb),
            operand: Some(NumericOperand::Register(3)),
            register_index: 5,
            prefix: NumericPrefix::None,
            operand_bits: 32,
            immediate: None,
        };
        let outcome = execute_numeric_instruction(NumericExecutionContext {
            state: &mut state,
            memory: &mut memory,
            instruction,
        })
        .unwrap();
        assert_eq!(outcome, NumericExecutionResult::Executed);
        let value = read_x87_register(&mut state.x87, 0).unwrap();
        assert_eq!(
            bits_u128(&encode_binary(&value, BinaryWidth::W80)),
            if mode == 1 || mode == 3 { 0x4000_c90f_daa2_2168_c234 } else { 0x4000_c90f_daa2_2168_c235 }
        );
        assert_eq!(state.x87.status_word & 0x220, 0);
    }
}

struct Atan {
    state: GuestProcessorState,
    memory: SparseGuestMemory,
}

fn atan_fixture(y: &BinaryValue, x: &BinaryValue, control: u16) -> Atan {
    let mut state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::I386,
        instruction_pointer: 0,
        stack_pointer: 0,
        flags: 0,
        x87_control_word: control,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    push_x87(&mut state.x87, y).unwrap();
    push_x87(&mut state.x87, x).unwrap();
    let memory = SparseGuestMemory::new(test_module("x87-trig"), 4, 0x10000).unwrap();
    Atan { state, memory }
}

fn atan_run(fixture: &mut Atan) -> NumericExecutionResult {
    let instruction = NumericInstruction {
        opcode: 0xd9,
        secondary_opcode: None,
        modrm: Some(0xf3),
        operand: Some(NumericOperand::Register(3)),
        register_index: 6,
        prefix: NumericPrefix::None,
        operand_bits: 32,
        immediate: None,
    };
    execute_numeric_instruction(NumericExecutionContext {
        state: &mut fixture.state,
        memory: &mut fixture.memory,
        instruction,
    })
    .unwrap()
}

fn atan_bits(fixture: &mut Atan) -> u128 {
    let value = read_x87_register(&mut fixture.state.x87, 0).unwrap();
    bits_u128(&encode_binary(&value, BinaryWidth::W80))
}

#[test]
fn fpatan_uses_cartesian_quadrants_and_pops_once() {
    for (x, expected) in [(1i64, 0x3ffe_c90f_daa2_2168_c235u128), (-1, 0x4000_96cb_e3f9_990e_91a8u128)] {
        for negative in [false, true] {
            let mut fixture = atan_fixture(
                &from_integer(&BigInt::from_i64(if negative { -1 } else { 1 })),
                &from_integer(&BigInt::from_i64(x)),
                0x7f,
            );
            let previous = fixture.state.x87.status_word & 0x3800;
            assert_eq!(atan_run(&mut fixture), NumericExecutionResult::Executed);
            assert_eq!(atan_bits(&mut fixture), expected | (u128::from(u8::from(negative)) << 79));
            assert_eq!(fixture.state.x87.status_word & 0x3800, (previous + 0x800) & 0x3800);
            assert_eq!(fixture.state.x87.status_word & 32, 32);
        }
    }
}

#[test]
fn fpatan_defines_signed_zero_and_infinite_axes() {
    for sign in [0u32, 1] {
        let y = zero(sign == 1);
        let mut positive = atan_fixture(&y, &zero(false), 0x37f);
        atan_run(&mut positive);
        assert_eq!(atan_bits(&mut positive), u128::from(sign) << 79);
        assert_eq!(positive.state.x87.status_word & 63, 0);
        let mut negative = atan_fixture(&y, &zero(true), 0x37f);
        atan_run(&mut negative);
        assert_eq!(atan_bits(&mut negative), 0x4000_c90f_daa2_2168_c235 | (u128::from(sign) << 79));
        assert_eq!(negative.state.x87.status_word & 5, 0);
    }
    let mut both = atan_fixture(
        &BinaryValue::Infinity { negative: false },
        &BinaryValue::Infinity { negative: false },
        0x37f,
    );
    atan_run(&mut both);
    assert_eq!(atan_bits(&mut both), 0x3ffe_c90f_daa2_2168_c235);
    assert_eq!(both.state.x87.status_word & 5, 0);
    let mut axis = atan_fixture(&from_integer(&BigInt::one()), &zero(false), 0x37f);
    atan_run(&mut axis);
    assert_eq!(atan_bits(&mut axis), 0x3fff_c90f_daa2_2168_c235);
}

#[test]
fn fpatan_keeps_tiny_ratios_and_respects_denormal_exceptions() {
    let mut nearest = atan_fixture(&decode_binary(&BigInt::one(), BinaryWidth::W80), &from_integer(&BigInt::one()), 0x37f);
    atan_run(&mut nearest);
    assert_eq!(atan_bits(&mut nearest), 1);
    assert_eq!(nearest.state.x87.status_word & 50, 50);
    let mut down = atan_fixture(&decode_binary(&BigInt::one(), BinaryWidth::W80), &from_integer(&BigInt::one()), 0x77f);
    atan_run(&mut down);
    assert_eq!(atan_bits(&mut down), 0);
    assert_eq!(down.state.x87.status_word & 50, 50);
    let mut bad = atan_fixture(
        &BinaryValue::Nan { negative: false, payload: BigInt::one(), signaling: true },
        &from_integer(&BigInt::one()),
        0x37e,
    );
    let top = bad.state.x87.status_word & 0x3800;
    assert_eq!(atan_run(&mut bad), NumericExecutionResult::Executed);
    assert_eq!(bad.state.x87.status_word & 0x3800, top);
    assert_eq!(atan_bits(&mut bad), 0x3fff_8000_0000_0000_0000);
    assert!(matches!(atan_run(&mut bad), NumericExecutionResult::Exception { .. }));
}

#[test]
fn fpatan_evaluates_finite_ratios_in_all_quadrants() {
    for y in [-99.0f64, -3.0, 1.0, 7.0] {
        for x in [-31.0f64, -2.0, 1.0, 17.0] {
            let mut fixture = atan_fixture(
                &from_integer(&BigInt::from_i64(y as i64)),
                &from_integer(&BigInt::from_i64(x as i64)),
                0x37f,
            );
            assert_eq!(atan_run(&mut fixture), NumericExecutionResult::Executed);
            let value = read_x87_return(&mut fixture.state.x87, BinaryWidth::W64).unwrap();
            assert!((value - y.atan2(x)).abs() < 1e-14, "{value} vs {}", y.atan2(x));
        }
    }
    for mode in [0u16, 1, 2, 3] {
        let mut fixture = atan_fixture(&from_integer(&BigInt::one()), &from_integer(&BigInt::one()), 0x37f | (mode << 10));
        atan_run(&mut fixture);
        assert_eq!(
            atan_bits(&mut fixture),
            if mode == 1 || mode == 3 { 0x3ffe_c90f_daa2_2168_c234 } else { 0x3ffe_c90f_daa2_2168_c235 }
        );
    }
}
