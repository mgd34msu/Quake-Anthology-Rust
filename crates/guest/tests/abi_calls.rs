//! ABI round-trip tests: call plans, argument marshalling, and result
//! handling across conventions.
//!
//! Donor: `tests/guest/abi/calls.test.ts`. The native-combat case belongs
//! to the compat lane and is not ported here.

mod common;

use std::rc::Rc;

use qa_guest::abi::adapter::X86AbiAdapter;
use qa_guest::abi::classify::{classify_system_v_aggregate, plan_guest_call, AbiLocation};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::HookState;
use qa_guest::core::contracts::{
    GuestAddress, GuestArchitecture, GuestCallResult, GuestCallSignature, GuestCallValue,
    GuestExecutionStop, GuestFieldLayout, GuestIntegerWidth, GuestLayout, GuestPermissions,
    GuestRegister, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};

use common::{addr, map, test_module};

/// Calling-convention fixture CPU: layout only, never executes.
struct FixtureCpu {
    state: GuestProcessorState,
    memory: SparseGuestMemory,
}

impl GuestCpu for FixtureCpu {
    fn parts(&mut self) -> (&mut GuestProcessorState, &mut SparseGuestMemory) {
        (&mut self.state, &mut self.memory)
    }

    fn set_hook_state(&mut self, _hooks: Option<Rc<HookState>>) {}

    fn run(
        &mut self,
        _instruction_budget: u64,
        _return_address: Option<GuestAddress>,
    ) -> GuestExecutionStop {
        panic!("ABI layout fixture must not execute instructions");
    }
}

struct Fixture {
    cpu: FixtureCpu,
    adapter: X86AbiAdapter,
}

fn fixture(abi: NativeCallAbi) -> Fixture {
    let width = abi.pointer_bytes();
    let mut memory = SparseGuestMemory::new(test_module("abi"), width, 0x10000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadExecute, Some(vec![0xc3]));
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: if width == 4 { GuestArchitecture::I386 } else { GuestArchitecture::X86_64 },
        instruction_pointer: 0x1000,
        stack_pointer: 0x20000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    Fixture { cpu: FixtureCpu { state, memory }, adapter: X86AbiAdapter::new(abi) }
}

fn signature(
    abi: NativeCallAbi,
    parameters: Vec<GuestValueLayout>,
    result: Option<GuestValueLayout>,
    variadic: bool,
) -> GuestCallSignature {
    GuestCallSignature { abi, parameters, result, variadic }
}

fn i32_layout() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Int32)
}

fn enter(
    fixture: &mut Fixture,
    call: &GuestCallSignature,
    arguments: &[GuestCallValue],
) -> u64 {
    let width = if fixture.cpu.memory.pointer_bytes() == 4 {
        GuestIntegerWidth::B32
    } else {
        GuestIntegerWidth::B64
    };
    let (target, return_address) = {
        let memory = &fixture.cpu.memory;
        (addr(memory, 0x1000), addr(memory, 0x1001))
    };
    fixture.adapter.enter(&mut fixture.cpu, target, call, arguments, return_address).unwrap();
    fixture.cpu.state.registers.read(GuestRegister::Rsp, width, false).unwrap()
}

fn record(
    pointer_bytes: usize,
    byte_length: usize,
    fields: Vec<GuestFieldLayout>,
    alignment: usize,
) -> GuestValueLayout {
    GuestValueLayout::Aggregate(GuestLayout {
        id: format!("test:record-{pointer_bytes}-{byte_length}"),
        byte_length,
        alignment,
        pointer_bytes,
        fields,
    })
}

fn field(name: &str, byte_offset: usize, storage: GuestStorage, count: usize) -> GuestFieldLayout {
    GuestFieldLayout { name: name.to_string(), byte_offset, storage, count }
}

fn location_kind(location: &AbiLocation) -> &'static str {
    match location {
        AbiLocation::Integer { .. } => "integer",
        AbiLocation::Sse { .. } => "sse",
        AbiLocation::Stack { .. } => "stack",
    }
}

fn return_value(fixture: &mut Fixture, call: &GuestCallSignature) -> GuestCallValue {
    match fixture.adapter.return_value(&mut fixture.cpu, call).unwrap() {
        GuestCallResult::Value(value) => value,
        GuestCallResult::Void => panic!("expected a return value"),
    }
}

#[test]
fn rebuilt_plans_follow_signature_and_layout_edits() {
    // This port computes owned plans instead of retaining cached ones, so
    // identity assertions become value-equality assertions.
    let mut call = signature(NativeCallAbi::MicrosoftX64, vec![i32_layout()], Some(i32_layout()), false);
    let original = plan_guest_call(&call, None).unwrap();
    assert_eq!(plan_guest_call(&call, None).unwrap(), original);
    call.parameters[0] = GuestValueLayout::Scalar(GuestStorage::Float32);
    let floating = plan_guest_call(&call, None).unwrap();
    assert_eq!(location_kind(&floating.arguments[0].locations[0]), "sse");
    call.parameters[0] = i32_layout();
    assert_eq!(location_kind(&plan_guest_call(&call, None).unwrap().arguments[0].locations[0]), "integer");
    let variadic = signature(
        NativeCallAbi::MicrosoftX64,
        vec![GuestValueLayout::Scalar(GuestStorage::Pointer)],
        None,
        true,
    );
    let fixed = plan_guest_call(&variadic, None).unwrap();
    let extended = plan_guest_call(
        &variadic,
        Some(&[
            GuestValueLayout::Scalar(GuestStorage::Pointer),
            GuestValueLayout::Scalar(GuestStorage::Float64),
        ]),
    )
    .unwrap();
    assert_eq!(extended.arguments.len(), 2);
    assert_eq!(plan_guest_call(&variadic, None).unwrap(), fixed);
    let mut layout = GuestLayout {
        id: "test:mutable".to_string(),
        byte_length: 4,
        alignment: 4,
        pointer_bytes: 8,
        fields: vec![field("value", 0, GuestStorage::Int32, 1)],
    };
    let aggregate = signature(
        NativeCallAbi::SystemVX86_64,
        vec![GuestValueLayout::Aggregate(layout.clone())],
        Some(i32_layout()),
        false,
    );
    assert_eq!(
        location_kind(&plan_guest_call(&aggregate, None).unwrap().arguments[0].locations[0]),
        "integer"
    );
    layout.byte_length = 24;
    let aggregate = signature(
        NativeCallAbi::SystemVX86_64,
        vec![GuestValueLayout::Aggregate(layout)],
        Some(i32_layout()),
        false,
    );
    assert_eq!(
        location_kind(&plan_guest_call(&aggregate, None).unwrap().arguments[0].locations[0]),
        "stack"
    );
}

#[test]
fn microsoft_x64_mixed_arguments_use_positional_registers_and_shadow_space() {
    let mut fixture = fixture(NativeCallAbi::MicrosoftX64);
    let call = signature(
        NativeCallAbi::MicrosoftX64,
        vec![
            i32_layout(),
            GuestValueLayout::Scalar(GuestStorage::Float64),
            i32_layout(),
            GuestValueLayout::Scalar(GuestStorage::Float32),
            i32_layout(),
            GuestValueLayout::Scalar(GuestStorage::Float32),
        ],
        Some(i32_layout()),
        false,
    );
    let values = vec![
        GuestCallValue::Int32(11),
        GuestCallValue::Float64(2.5),
        GuestCallValue::Int32(33),
        GuestCallValue::Float32(4.5),
        GuestCallValue::Int32(55),
        GuestCallValue::Float32(6.5),
    ];
    let sp = enter(&mut fixture, &call, &values);
    assert_eq!((sp + 8) % 16, 0);
    let (state, memory) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B64, false).unwrap(), 11);
    assert_eq!(state.registers.read(GuestRegister::R8, GuestIntegerWidth::B64, false).unwrap(), 33);
    assert_eq!(f64::from_le_bytes(state.simd.xmm[16..24].try_into().unwrap()), 2.5);
    assert_eq!(f32::from_le_bytes(state.simd.xmm[48..52].try_into().unwrap()), 4.5);
    let shadow = addr(memory, sp + 8);
    assert_eq!(memory.copy(shadow, 32).unwrap(), vec![0u8; 32]);
    assert_eq!(memory.read_i32(addr(memory, sp + 40)).unwrap(), 55);
    assert_eq!(memory.read_f32(addr(memory, sp + 48)).unwrap(), 6.5);
    drop(state);
    drop(memory);
    assert_eq!(fixture.adapter.arguments(&mut fixture.cpu, &call, &[]).unwrap(), values);
}

#[test]
fn microsoft_x64_variadic_floats_promote_and_duplicate() {
    let mut fixture = fixture(NativeCallAbi::MicrosoftX64);
    let call = signature(
        NativeCallAbi::MicrosoftX64,
        vec![GuestValueLayout::Scalar(GuestStorage::Pointer)],
        None,
        true,
    );
    enter(&mut fixture, &call, &[GuestCallValue::Pointer(None), GuestCallValue::Float32(3.25)]);
    let (state, _) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rdx, GuestIntegerWidth::B64, false).unwrap(), 0x400a_0000_0000_0000);
    assert_eq!(f64::from_le_bytes(state.simd.xmm[16..24].try_into().unwrap()), 3.25);
    drop(state);
    assert_eq!(
        fixture
            .adapter
            .arguments(&mut fixture.cpu, &call, &[GuestValueLayout::Scalar(GuestStorage::Float64)])
            .unwrap(),
        vec![GuestCallValue::Pointer(None), GuestCallValue::Float64(3.25)]
    );
}

#[test]
fn system_v_aggregates_merge_classes_and_spill_indivisible_arguments() {
    use qa_guest::abi::classify::EightbyteClass;

    let pair = record(8, 16, vec![field("a", 0, GuestStorage::Uint64, 2)], 8);
    let call = signature(
        NativeCallAbi::SystemVX86_64,
        vec![i32_layout(), i32_layout(), i32_layout(), i32_layout(), i32_layout(), pair, i32_layout()],
        Some(i32_layout()),
        false,
    );
    let plan = plan_guest_call(&call, None).unwrap();
    assert_eq!(
        plan.arguments[5].locations,
        vec![AbiLocation::Stack { stack_offset: 8, offset: 0, bytes: 16 }]
    );
    assert_eq!(
        plan.arguments[6].locations,
        vec![AbiLocation::Integer { register: GuestRegister::R9, offset: 0, bytes: 4 }]
    );
    let mixed = record(
        8,
        16,
        vec![
            field("number", 0, GuestStorage::Float64, 1),
            field("counter", 8, GuestStorage::Uint64, 1),
        ],
        8,
    );
    assert_eq!(
        classify_system_v_aggregate(&mixed).unwrap(),
        Some(vec![EightbyteClass::Sse, EightbyteClass::Integer])
    );
    let union = record(
        8,
        8,
        vec![
            field("number", 0, GuestStorage::Float64, 1),
            field("bits", 0, GuestStorage::Uint64, 1),
        ],
        8,
    );
    assert_eq!(classify_system_v_aggregate(&union).unwrap(), Some(vec![EightbyteClass::Integer]));
    let unaligned = record(8, 9, vec![field("unaligned", 1, GuestStorage::Float64, 1)], 1);
    assert_eq!(classify_system_v_aggregate(&unaligned).unwrap(), None);
}

#[test]
fn system_v_float_aggregate_returns_use_xmm0_xmm1_and_raw_bytes() {
    let mut fixture = fixture(NativeCallAbi::SystemVX86_64);
    let layout = match record(8, 12, vec![field("xyz", 0, GuestStorage::Float32, 3)], 4) {
        GuestValueLayout::Aggregate(layout) => layout,
        _ => panic!("fixture must be an aggregate"),
    };
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x7fc0_1234u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&7.5f32.to_le_bytes());
    let value = GuestCallValue::Aggregate { layout: layout.clone(), bytes: bytes.to_vec() };
    let call = signature(
        NativeCallAbi::SystemVX86_64,
        vec![GuestValueLayout::Aggregate(layout)],
        Some(record(8, 12, vec![field("xyz", 0, GuestStorage::Float32, 3)], 4)),
        false,
    );
    enter(&mut fixture, &call, &[value.clone()]);
    let (state, _) = fixture.cpu.parts();
    assert_eq!(state.simd.xmm[0..8], bytes[0..8]);
    assert_eq!(state.simd.xmm[16..20], bytes[8..12]);
    drop(state);
    fixture.adapter.leave(&mut fixture.cpu, &call, &GuestCallResult::Value(value.clone())).unwrap();
    assert_eq!(return_value(&mut fixture, &call), value);
}

#[test]
fn integer_abi_parts_round_trip_narrow_odd_and_split_widths() {
    // The donor wraps the register file to record access widths; Rust
    // register accesses are statically typed, so only the round-trip
    // values port.
    for size in 1..=16usize {
        let mut fixture = fixture(NativeCallAbi::SystemVX86_64);
        let layout = match record(8, size, vec![field("bytes", 0, GuestStorage::Uint8, size)], 1) {
            GuestValueLayout::Aggregate(layout) => layout,
            _ => panic!("expected aggregate fixture"),
        };
        let storage: Vec<u8> = (0..size + 4).map(|index| (index * 37 + 0x81) as u8).collect();
        let value = GuestCallValue::Aggregate { layout: layout.clone(), bytes: storage[2..size + 2].to_vec() };
        let call = signature(
            NativeCallAbi::SystemVX86_64,
            vec![GuestValueLayout::Aggregate(layout)],
            Some(record(8, size, vec![field("bytes", 0, GuestStorage::Uint8, size)], 1)),
            false,
        );
        enter(&mut fixture, &call, &[value.clone()]);
        assert_eq!(fixture.adapter.arguments(&mut fixture.cpu, &call, &[]).unwrap(), vec![value.clone()]);
        fixture.adapter.leave(&mut fixture.cpu, &call, &GuestCallResult::Value(value.clone())).unwrap();
        assert_eq!(return_value(&mut fixture, &call), value);
    }
}

#[test]
fn system_v_variadics_count_vector_registers_in_al() {
    let mut fixture = fixture(NativeCallAbi::SystemVX86_64);
    let call = signature(NativeCallAbi::SystemVX86_64, vec![i32_layout()], None, true);
    enter(
        &mut fixture,
        &call,
        &[
            GuestCallValue::Int32(9),
            GuestCallValue::Float32(1.5),
            GuestCallValue::Uint64(17),
            GuestCallValue::Float64(-2.0),
        ],
    );
    let (state, _) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B8, false).unwrap(), 2);
    assert_eq!(state.registers.read(GuestRegister::Rdi, GuestIntegerWidth::B64, false).unwrap(), 9);
    assert_eq!(state.registers.read(GuestRegister::Rsi, GuestIntegerWidth::B64, false).unwrap(), 17);
    assert_eq!(f64::from_le_bytes(state.simd.xmm[0..8].try_into().unwrap()), 1.5);
    assert_eq!(f64::from_le_bytes(state.simd.xmm[16..24].try_into().unwrap()), -2.0);
}

#[test]
fn win32_conventions_have_distinct_registers_and_callee_cleanup() {
    for abi in [NativeCallAbi::Cdecl, NativeCallAbi::Stdcall, NativeCallAbi::Fastcall] {
        let mut fixture = fixture(abi);
        let call = signature(abi, vec![i32_layout(), i32_layout(), i32_layout()], Some(i32_layout()), false);
        let sp = enter(
            &mut fixture,
            &call,
            &[GuestCallValue::Int32(1), GuestCallValue::Int32(2), GuestCallValue::Int32(3)],
        );
        let (state, memory) = fixture.cpu.parts();
        let expected = if abi == NativeCallAbi::Fastcall { 3 } else { 1 };
        assert_eq!(memory.read_i32(addr(memory, sp + 4)).unwrap(), expected);
        if abi == NativeCallAbi::Fastcall {
            assert_eq!(state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B32, false).unwrap(), 1);
            assert_eq!(state.registers.read(GuestRegister::Rdx, GuestIntegerWidth::B32, false).unwrap(), 2);
        }
        drop(state);
        drop(memory);
        fixture
            .adapter
            .leave(&mut fixture.cpu, &call, &GuestCallResult::Value(GuestCallValue::Int32(-7)))
            .unwrap();
        let cleanup = match abi {
            NativeCallAbi::Cdecl => 4,
            NativeCallAbi::Stdcall => 16,
            _ => 8,
        };
        let (state, _) = fixture.cpu.parts();
        assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B32, false).unwrap(), sp + cleanup);
        assert_eq!(state.instruction_pointer, 0x1001);
        drop(state);
        assert_eq!(return_value(&mut fixture, &call), GuestCallValue::Int32(-7));
    }
    let mut fixture = fixture(NativeCallAbi::Thiscall);
    let call = signature(
        NativeCallAbi::Thiscall,
        vec![GuestValueLayout::Scalar(GuestStorage::Pointer), i32_layout()],
        Some(i32_layout()),
        false,
    );
    let this = fixture.cpu.parts().1.pointer(0x18000).unwrap().unwrap();
    let sp = enter(&mut fixture, &call, &[GuestCallValue::Pointer(Some(this)), GuestCallValue::Int32(21)]);
    let (state, memory) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B32, false).unwrap(), 0x18000);
    assert_eq!(memory.read_i32(addr(memory, sp + 4)).unwrap(), 21);
}

#[test]
fn system_v_i386_hidden_return_pointer_is_callee_popped() {
    let mut fixture = fixture(NativeCallAbi::SystemVI386);
    let layout = match record(4, 12, vec![field("items", 0, GuestStorage::Uint32, 3)], 4) {
        GuestValueLayout::Aggregate(layout) => layout,
        _ => panic!("expected aggregate fixture"),
    };
    let call = signature(
        NativeCallAbi::SystemVI386,
        vec![i32_layout()],
        Some(GuestValueLayout::Aggregate(layout.clone())),
        false,
    );
    let sp = enter(&mut fixture, &call, &[GuestCallValue::Int32(77)]);
    assert_eq!((sp + 4) % 16, 0);
    let (state, memory) = fixture.cpu.parts();
    let destination = memory.read_u32(addr(memory, sp + 4)).unwrap();
    assert_eq!(memory.read_i32(addr(memory, sp + 8)).unwrap(), 77);
    drop(state);
    drop(memory);
    let value = GuestCallValue::Aggregate { layout, bytes: vec![0x5a; 12] };
    fixture.adapter.leave(&mut fixture.cpu, &call, &GuestCallResult::Value(value.clone())).unwrap();
    let (state, _) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B32, false).unwrap(), sp + 8);
    assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), u64::from(destination));
    drop(state);
    assert_eq!(return_value(&mut fixture, &call), value);
}

#[test]
fn microsoft_x64_large_aggregates_use_aligned_independent_temporaries() {
    let mut fixture = fixture(NativeCallAbi::MicrosoftX64);
    let layout = match record(8, 24, vec![field("items", 0, GuestStorage::Uint64, 3)], 8) {
        GuestValueLayout::Aggregate(layout) => layout,
        _ => panic!("expected aggregate fixture"),
    };
    let mut bytes = vec![9u8; 24];
    let value = GuestCallValue::Aggregate { layout: layout.clone(), bytes: bytes.clone() };
    let call = signature(
        NativeCallAbi::MicrosoftX64,
        vec![GuestValueLayout::Aggregate(layout), i32_layout()],
        Some(record(8, 24, vec![field("items", 0, GuestStorage::Uint64, 3)], 8)),
        false,
    );
    enter(&mut fixture, &call, &[value.clone(), GuestCallValue::Int32(31)]);
    let result_pointer = fixture.cpu.state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B64, false).unwrap();
    let copy_pointer = fixture.cpu.state.registers.read(GuestRegister::Rdx, GuestIntegerWidth::B64, false).unwrap();
    assert_eq!(result_pointer % 16, 0);
    assert_eq!(copy_pointer % 16, 0);
    assert_ne!(result_pointer, copy_pointer);
    assert_eq!(
        fixture.cpu.state.registers.read(GuestRegister::R8, GuestIntegerWidth::B64, false).unwrap(),
        31
    );
    bytes.fill(77);
    let (_, memory) = fixture.cpu.parts();
    assert_eq!(memory.copy(addr(memory, copy_pointer), 24).unwrap(), vec![9u8; 24]);
    drop(memory);
    fixture.adapter.leave(&mut fixture.cpu, &call, &GuestCallResult::Value(value.clone())).unwrap();
    assert_eq!(
        fixture.cpu.state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B64, false).unwrap(),
        result_pointer
    );
    assert_eq!(return_value(&mut fixture, &call), value);
}

#[test]
fn i386_float_returns_use_st0_and_consume_the_slot() {
    for abi in [NativeCallAbi::Cdecl, NativeCallAbi::SystemVI386] {
        let mut fixture = fixture(abi);
        let call = signature(abi, vec![], Some(GuestValueLayout::Scalar(GuestStorage::Float64)), false);
        let initial_top = fixture.cpu.state.x87.status_word >> 11 & 7;
        enter(&mut fixture, &call, &[]);
        fixture
            .adapter
            .leave(&mut fixture.cpu, &call, &GuestCallResult::Value(GuestCallValue::Float64(-0.0)))
            .unwrap();
        assert_eq!(fixture.cpu.state.x87.status_word >> 11 & 7, (initial_top + 7) & 7);
        match return_value(&mut fixture, &call) {
            GuestCallValue::Float64(value) => assert_eq!(value.to_bits(), (-0.0f64).to_bits()),
            other => panic!("expected float64 return, got {other:?}"),
        }
        assert_eq!(fixture.cpu.state.x87.status_word >> 11 & 7, initial_top);
        assert_eq!(fixture.cpu.state.x87.tag_word, 0xffff);
    }
}

#[test]
fn subword_integers_widen_before_occupying_stack_slots() {
    let mut fixture = fixture(NativeCallAbi::Cdecl);
    let call = signature(
        NativeCallAbi::Cdecl,
        vec![
            GuestValueLayout::Scalar(GuestStorage::Int8),
            GuestValueLayout::Scalar(GuestStorage::Uint16),
        ],
        Some(i32_layout()),
        false,
    );
    let memory = &mut fixture.cpu.memory;
    memory.write(addr(memory, 0x1ff00), &vec![0x5au8; 256]).unwrap();
    let sp = enter(&mut fixture, &call, &[GuestCallValue::Int32(-2), GuestCallValue::Uint32(0x1234)]);
    let (_, memory) = fixture.cpu.parts();
    assert_eq!(memory.read_u32(addr(memory, sp + 4)).unwrap(), 0xffff_fffe);
    assert_eq!(memory.read_u32(addr(memory, sp + 8)).unwrap(), 0x1234);
    drop(memory);
    assert_eq!(
        fixture.adapter.arguments(&mut fixture.cpu, &call, &[]).unwrap(),
        vec![GuestCallValue::Int32(-2), GuestCallValue::Uint32(0x1234)]
    );
}

#[test]
fn i386_64bit_results_split_across_eax_and_edx() {
    for abi in [NativeCallAbi::Cdecl, NativeCallAbi::SystemVI386] {
        let mut fixture = fixture(abi);
        let call = signature(abi, vec![], Some(GuestValueLayout::Scalar(GuestStorage::Uint64)), false);
        enter(&mut fixture, &call, &[]);
        fixture
            .adapter
            .leave(
                &mut fixture.cpu,
                &call,
                &GuestCallResult::Value(GuestCallValue::Uint64(0xf123_4567_89ab_cdef)),
            )
            .unwrap();
        let (state, _) = fixture.cpu.parts();
        assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), 0x89ab_cdef);
        assert_eq!(state.registers.read(GuestRegister::Rdx, GuestIntegerWidth::B32, false).unwrap(), 0xf123_4567);
        drop(state);
        assert_eq!(
            return_value(&mut fixture, &call),
            GuestCallValue::Uint64(0xf123_4567_89ab_cdef)
        );
    }
}

#[test]
fn win32_thiscall_uses_hidden_return_buffer_for_small_records() {
    let mut fixture = fixture(NativeCallAbi::Thiscall);
    let layout = match record(4, 4, vec![field("member", 0, GuestStorage::Int32, 1)], 4) {
        GuestValueLayout::Aggregate(layout) => layout,
        _ => panic!("fixture must be an aggregate"),
    };
    let call = signature(
        NativeCallAbi::Thiscall,
        vec![GuestValueLayout::Scalar(GuestStorage::Pointer)],
        Some(GuestValueLayout::Aggregate(layout.clone())),
        false,
    );
    let this = fixture.cpu.parts().1.pointer(0x19000).unwrap().unwrap();
    let sp = enter(&mut fixture, &call, &[GuestCallValue::Pointer(Some(this))]);
    let (state, memory) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rcx, GuestIntegerWidth::B32, false).unwrap(), 0x19000);
    let output = memory.read_u32(addr(memory, sp + 4)).unwrap();
    assert_ne!(output, 0);
    drop(state);
    drop(memory);
    let value = GuestCallValue::Aggregate { layout, bytes: vec![41, 0, 0, 0] };
    fixture.adapter.leave(&mut fixture.cpu, &call, &GuestCallResult::Value(value.clone())).unwrap();
    let (state, _) = fixture.cpu.parts();
    assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), u64::from(output));
    assert_eq!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B32, false).unwrap(), sp + 8);
    drop(state);
    assert_eq!(return_value(&mut fixture, &call), value);
}
