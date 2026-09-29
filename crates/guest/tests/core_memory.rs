//! Core memory, register, flag, and callback-table tests.
//!
//! Donor: `tests/guest/core/memory.test.ts`. The Rust port omits borrowed
//! live views (`borrow`) by design, so view-mutation assertions port as
//! equivalent write/read sequences; write observers cannot fail or access
//! memory, so failure-aggregation cases port as delivery-order checks.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use qa_guest::abi::runner::{GuestCallRequest, GuestCallRunner};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::{GuestHostCallback, HookState};
use qa_guest::core::contracts::{
    CallbackId, GuestAddress, GuestArchitecture, GuestCallContext, GuestCallResult, GuestCallSignature,
    GuestCallbackReference, GuestFlag, GuestIntegerWidth, GuestMapOptions, GuestPermissions, GuestRegister, GuestWrittenRange, NativeCallAbi,
};
use qa_guest::core::memory::{
    add_guest_pointer, signed_guest_pointer, wrap_guest_pointer, FetchCursor, SparseGuestMemory,
};
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState, IntegerRegisterFile, ProcessorFlags};

use common::{map, test_module};

fn flags_fixture(initial: u64) -> ProcessorFlags {
    ProcessorFlags::new(initial)
}

#[test]
fn processor_flag_masks_preserve_unrelated_and_reserved_bits() {
    let positions = [
        (GuestFlag::Carry, 0),
        (GuestFlag::Parity, 2),
        (GuestFlag::AuxiliaryCarry, 4),
        (GuestFlag::Zero, 6),
        (GuestFlag::Sign, 7),
        (GuestFlag::Trap, 8),
        (GuestFlag::Interrupt, 9),
        (GuestFlag::Direction, 10),
        (GuestFlag::Overflow, 11),
        (GuestFlag::Resume, 16),
        (GuestFlag::Virtual8086, 17),
        (GuestFlag::AlignmentCheck, 18),
        (GuestFlag::VirtualInterrupt, 19),
        (GuestFlag::VirtualInterruptPending, 20),
        (GuestFlag::Identification, 21),
    ];
    for initial in [0u64, u64::MAX, 0x1234_5678_9abc_def0, 1 << 63] {
        for (flag, bit) in positions {
            let mut flags = flags_fixture(initial);
            let mask = 1u64 << bit;
            assert_eq!(flags.get(flag), initial & mask != 0);
            flags.set(flag, true);
            assert_eq!(flags.value(), initial | mask);
            assert!(flags.get(flag));
            flags.set(flag, false);
            assert_eq!(flags.value(), initial & !mask);
            assert!(!flags.get(flag));
            flags.set_value(initial);
            assert_eq!(flags.value(), initial);
        }
    }
}

#[test]
fn sparse_high_64_bit_addresses_preserve_pointer_bytes_and_aliases() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let initial = vec![9u8, 8];
    let base = memory
        .map(&GuestMapOptions {
            base: 0xf123_4567_89ab_c000,
            byte_length: 64,
            permissions: GuestPermissions::ReadWrite,
            label: "high".to_string(),
            bytes: Some(initial),
        })
        .unwrap();
    assert_eq!(memory.read_u8(base).unwrap(), 9);
    memory.write_u32(memory.offset(base, 4).unwrap(), 0x3f80_0000).unwrap();
    assert_eq!(memory.read_f32(memory.offset(base, 4).unwrap()).unwrap(), 1.0);
    memory.write_u16(memory.offset(base, 6).unwrap(), 0x4000).unwrap();
    assert_eq!(memory.read_f32(memory.offset(base, 4).unwrap()).unwrap(), 2.0);
    let destination = memory.offset(base, 24).unwrap();
    let target = memory.offset(base, 63).unwrap();
    memory.write_pointer(destination, Some(target)).unwrap();
    assert_eq!(
        memory.read_pointer(destination).unwrap().unwrap().offset,
        0xf123_4567_89ab_c03f
    );
    memory.write_pointer(destination, None).unwrap();
    assert_eq!(memory.read_pointer(destination).unwrap(), None);
    assert_eq!(
        memory.mappings().iter().map(|entry| entry.byte_length).sum::<usize>(),
        64
    );
    let boundary = map(&mut memory, 0x1fff_ffff_ffff_fe, 4, GuestPermissions::ReadWrite, None);
    memory.write_u32(boundary, 0x1234_5678).unwrap();
    assert_eq!(memory.read_u8(memory.offset(boundary, 3).unwrap()).unwrap(), 0x12);
    assert!(format!("{:?}", memory.read_u8(memory.offset(boundary, 4).unwrap())).contains("unmapped"));
    let beyond = map(
        &mut memory,
        0x2000_0000_0000_10,
        1,
        GuestPermissions::Read,
        Some(vec![41]),
    );
    let neighbor = map(
        &mut memory,
        beyond.offset + 2,
        1,
        GuestPermissions::Read,
        Some(vec![43]),
    );
    assert_eq!(memory.read_u8(beyond).unwrap(), 41);
    assert_eq!(memory.read_u8(neighbor).unwrap(), 43);
    assert!(format!("{:?}", memory.read_u8(memory.offset(beyond, 1).unwrap())).contains("unmapped"));
    assert!(format!("{:?}", memory.copy(memory.offset(beyond, -1).unwrap(), 1)).contains("unmapped"));
}

#[test]
fn checked_accesses_cross_adjacent_mappings_and_reject_faulting_writes() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 4, 0x10000).unwrap();
    let start = map(&mut memory, 0x1000, 3, GuestPermissions::ReadWrite, None);
    let tail = map(&mut memory, 0x1003, 3, GuestPermissions::ReadWrite, None);
    memory.write_u32(memory.offset(start, 1).unwrap(), 0x1234_5678).unwrap();
    assert_eq!(memory.copy(start, 6).unwrap(), [0, 0x78, 0x56, 0x34, 0x12, 0]);
    assert_eq!(memory.read_u32(memory.offset(start, 1).unwrap()).unwrap(), 0x1234_5678);
    memory.protect(tail, 3, GuestPermissions::Read).unwrap();
    assert!(memory.write(start, &[99; 6]).is_err());
    assert_eq!(memory.read_u8(start).unwrap(), 0);
    assert!(format!("{:?}", memory.fetch(start, 1)).contains("permits read-write"));
    memory.protect(start, 3, GuestPermissions::Execute).unwrap();
    assert_eq!(memory.fetch(start, 3).unwrap(), [0, 0x78, 0x56]);
    assert!(format!("{:?}", memory.copy(start, 1)).contains("permits execute"));
}

#[test]
fn vector_reads_preserve_alias_changes_and_check_full_range() {
    for split in [false, true] {
        let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
        let start = map(
            &mut memory,
            0x1000,
            if split { 5 } else { 12 },
            GuestPermissions::ReadWrite,
            None,
        );
        if split {
            map(&mut memory, 0x1005, 7, GuestPermissions::ReadWrite, None);
        }
        memory.write_f32(start, -0.0).unwrap();
        memory.write_f32(memory.offset(start, 4).unwrap(), 1.5).unwrap();
        memory.write_f32(memory.offset(start, 8).unwrap(), -2.25).unwrap();
        let vector = memory.read_f32x3(start).unwrap();
        assert!(vector.x == 0.0 && vector.x.is_sign_negative());
        assert_eq!((vector.y, vector.z), (1.5, -2.25));
        let source = memory.offset(start, 8).unwrap();
        let alias = memory
            .map_alias(0x2000, 4, GuestPermissions::ReadWrite, "alias", source)
            .unwrap();
        memory.write_f32(alias, 42.5).unwrap();
        let vector = memory.read_f32x3(start).unwrap();
        assert_eq!((vector.y, vector.z), (1.5, 42.5));
        memory
            .protect(memory.offset(start, 8).unwrap(), 4, GuestPermissions::Execute)
            .unwrap();
        assert!(memory.read_f32x3(start).is_err());
        memory.unmap(memory.offset(start, 8).unwrap(), 4).unwrap();
        assert!(memory.read_f32x3(start).is_err());
    }
}

fn allocate(memory: &mut SparseGuestMemory, byte_length: usize, alignment: u64) -> GuestAddress {
    memory
        .allocate(&qa_guest::core::contracts::GuestAllocationOptions {
            byte_length,
            alignment,
            permissions: GuestPermissions::ReadWrite,
            label: "core-fixture".to_string(),
        })
        .unwrap()
}

#[test]
fn contiguous_bulk_writes_preserve_overlapping_sources_and_notify_after_commit() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = allocate(&mut memory, 8, 16);
    memory.write(base, &[0, 1, 2, 3, 4, 5, 6, 7]).unwrap();
    let detached = memory.copy(base, 8).unwrap();
    let notifications: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    // Observers cannot read memory; snapshot delivery order via committed reads after each write.
    let seen: Rc<RefCell<Vec<Vec<GuestWrittenRange>>>> = Rc::new(RefCell::new(Vec::new()));
    let probe = Rc::clone(&seen);
    let id = memory
        .observe_writes(
            base,
            8,
            Box::new(move |ranges| probe.borrow_mut().push(ranges.to_vec())),
        )
        .unwrap();
    let overlap: Vec<u8> = memory.copy(base, 6).unwrap();
    memory.write(memory.offset(base, 2).unwrap(), &overlap).unwrap();
    assert_eq!(memory.copy(base, 8).unwrap(), [0, 1, 0, 1, 2, 3, 4, 5]);
    notifications.borrow_mut().push(memory.copy(base, 8).unwrap());
    let overlap: Vec<u8> = memory.copy(memory.offset(base, 2).unwrap(), 6).unwrap();
    memory.write(base, &overlap).unwrap();
    assert_eq!(memory.copy(base, 8).unwrap(), [0, 1, 2, 3, 4, 5, 4, 5]);
    notifications.borrow_mut().push(memory.copy(base, 8).unwrap());
    assert_eq!(
        *notifications.borrow(),
        [vec![0, 1, 0, 1, 2, 3, 4, 5], vec![0, 1, 2, 3, 4, 5, 4, 5]]
    );
    assert_eq!(detached, [0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(seen.borrow().len(), 2);
    memory.unobserve(id);
}

#[test]
fn split_protections_and_restored_aliases_retain_private_bytes() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 4, 0x10000).unwrap();
    let original = allocate(&mut memory, 64, 16);
    let source = memory.offset(original, 8).unwrap();
    let alias = memory
        .map_alias(0x20000, 16, GuestPermissions::ReadWrite, "alias", source)
        .unwrap();
    memory.write_u32(alias, 0xaabb_ccdd).unwrap();
    assert_eq!(
        memory.read_u32(memory.offset(original, 8).unwrap()).unwrap(),
        0xaabb_ccdd
    );
    memory
        .protect(memory.offset(original, 16).unwrap(), 16, GuestPermissions::Read)
        .unwrap();
    assert!(memory.write(memory.offset(original, 16).unwrap(), &[1]).is_err());
    memory
        .protect(memory.offset(original, 16).unwrap(), 16, GuestPermissions::ReadWrite)
        .unwrap();
    assert_eq!(
        memory.read_u32(memory.offset(original, 8).unwrap()).unwrap(),
        0xaabb_ccdd
    );
    let module = test_module("guest");
    let mut restored = SparseGuestMemory::restore(module, &memory.checkpoint()).unwrap();
    assert!(format!("{:?}", restored.copy(original, 4)).contains("another execution owner"));
    let restored_alias = restored.pointer(alias.offset + 2).unwrap().unwrap();
    restored.write_u16(restored_alias, 0x1234).unwrap();
    let restored_source = restored.pointer(original.offset + 8).unwrap().unwrap();
    assert_eq!(restored.read_u32(restored_source).unwrap(), 0x1234_ccdd);
    assert_eq!(memory.read_u32(alias).unwrap(), 0xaabb_ccdd);
    memory.unmap(original, 64).unwrap();
    assert_eq!(memory.read_u32(alias).unwrap(), 0xaabb_ccdd);
    assert!(format!("{:?}", memory.copy(original, 1)).contains("unmapped"));
}

#[test]
fn checked_address_creation_and_explicit_machine_wrapping_stay_distinct() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 4, 0x10000).unwrap();
    assert_eq!(memory.pointer(0).unwrap(), None);
    assert!(format!("{:?}", memory.pointer(0x1_0000_0000)).contains("32-bit address space"));
    let last = map(&mut memory, 0xffff_ffff, 1, GuestPermissions::ReadWrite, None);
    memory.write_u8(last, 7).unwrap();
    assert_eq!(memory.read_u8(last).unwrap(), 7);
    assert!(memory.offset(last, 1).is_err());
    assert_eq!(add_guest_pointer(0xffff_ffff, 1, 4), 0);
    assert_eq!(wrap_guest_pointer(u64::MAX, 8), u64::MAX);
    assert_eq!(wrap_guest_pointer(0, 8).wrapping_sub(1), 0xffff_ffff_ffff_ffff);
    assert_eq!(signed_guest_pointer(0xffff_ffff, 4), -1);
    let block = allocate(&mut memory, 32, 32);
    memory.unmap(block, 32).unwrap();
    assert_eq!(allocate(&mut memory, 32, 32).offset, block.offset);
}

#[test]
fn callback_addresses_survive_nested_calls_revocation_and_restoration() {
    let module = test_module("guest");
    let mut memory = SparseGuestMemory::new(module.clone(), 4, 0x10000).unwrap();
    let hooks = Rc::new(HookState::new());
    let state = allocate(&mut memory, 4, 16);
    let signature = GuestCallSignature {
        abi: NativeCallAbi::Cdecl,
        parameters: vec![],
        result: None,
        variadic: false,
    };
    let nested = GuestHostCallback {
        id: CallbackId::new("test", "nested"),
        signature: signature.clone(),
        invoke: Rc::new(move |ctx, _, _| {
            let memory = ctx.memory();
            let current = memory.read_u32(state)?;
            memory.write_u32(state, current + 7)?;
            Ok(GuestCallResult::Void)
        }),
    };
    let nested_address = hooks.callbacks.borrow_mut().bind(&mut memory, nested.clone()).unwrap();
    // Nest through a live runner: outer writes 10, nested adds 7.
    let cpu_state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::I386,
        instruction_pointer: 0x1800,
        stack_pointer: 0x40000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    map(&mut memory, 0x30000, 65536, GuestPermissions::ReadWrite, None);
    map(
        &mut memory,
        0x1000,
        4096,
        GuestPermissions::ReadExecute,
        Some(vec![0xcc]),
    );
    let context = GuestCallContext {
        module: module.clone(),
        callback: GuestCallbackReference::NativeGuest {
            module: module.clone(),
            address: nested_address,
            abi: NativeCallAbi::Cdecl,
        },
        parent: None,
        itself: None,
        other: None,
    };
    let outer = GuestHostCallback {
        id: CallbackId::new("test", "outer"),
        signature: signature.clone(),
        invoke: {
            let context = context.clone();
            let signature = signature.clone();
            Rc::new(move |ctx, parent, _| {
                ctx.memory().write_u32(state, 10)?;
                let nested_request = GuestCallRequest {
                    target: nested_address,
                    signature: signature.clone(),
                    arguments: vec![],
                    context: GuestCallContext {
                        parent: Some(Box::new(parent.clone())),
                        ..context.clone()
                    },
                    instruction_budget: 100,
                };
                ctx.invoke(&nested_request).map_err(|failure| {
                    qa_guest::error::GuestError::callback(format!("nested call failed: {failure:?}"))
                })?;
                assert_eq!(ctx.memory().read_u32(state).unwrap(), 17);
                Ok(GuestCallResult::Void)
            })
        },
    };
    let outer_address = hooks.callbacks.borrow_mut().bind(&mut memory, outer).unwrap();
    let boxed: Box<dyn GuestCpu> = Box::new(qa_guest::x86::cpu::I386Cpu::new(cpu_state, memory).unwrap());
    let leaked: &'static mut dyn GuestCpu = Box::leak(boxed);
    let return_address = leaked.parts().1.pointer(0x1800).unwrap().unwrap();
    let mut runner = GuestCallRunner::new(leaked, Rc::clone(&hooks), return_address, None).unwrap();
    let request = GuestCallRequest {
        target: outer_address,
        signature: signature.clone(),
        arguments: vec![],
        context: context.clone(),
        instruction_budget: 100,
    };
    runner.invoke(&request).unwrap();
    hooks.callbacks.borrow_mut().unbind(&CallbackId::new("test", "nested"));
    let (_, memory) = runner.cpu_parts();
    assert!(format!("{:?}", hooks.callbacks.borrow_mut().handle(memory, nested_address)).contains("unbound"));
    let rebound = hooks.callbacks.borrow_mut().bind(memory, nested.clone()).unwrap();
    assert_eq!(rebound.offset, nested_address.offset);
    hooks.callbacks.borrow_mut().unbind(&CallbackId::new("test", "outer"));
    let snapshot = {
        let (_, memory) = runner.cpu_parts();
        memory.checkpoint()
    };
    let mut restored_memory = SparseGuestMemory::restore(module.clone(), &snapshot).unwrap();
    let saved = hooks.callbacks.borrow().checkpoint();
    let restored_nested = GuestHostCallback {
        id: CallbackId::new("test", "nested"),
        signature: signature.clone(),
        invoke: Rc::new(|_, _, _| Ok(GuestCallResult::Void)),
    };
    let outer_clone = GuestHostCallback {
        id: CallbackId::new("test", "outer"),
        signature: signature.clone(),
        invoke: Rc::new(|_, _, _| Ok(GuestCallResult::Void)),
    };
    let mut restored_table =
        qa_guest::core::callbacks::GuestCallbackTable::restore(&mut restored_memory, &saved, |id| {
            if *id == CallbackId::new("test", "nested") {
                Some(restored_nested.clone())
            } else {
                Some(outer_clone.clone())
            }
        })
        .unwrap();
    let restored_address = restored_memory.pointer(nested_address.offset).unwrap().unwrap();
    let resolved = restored_table
        .resolve(&mut restored_memory, restored_address)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.id, CallbackId::new("test", "nested"));
    let outer_restored = restored_table
        .address(&CallbackId::new("test", "outer"))
        .expect("restored outer address");
    assert!(format!("{:?}", restored_table.resolve(&mut restored_memory, outer_restored)).contains("unbound"));
}

#[test]
fn integer_register_aliases_preserve_high_bits_and_clear_upper_on_32_bit_writes() {
    let mut registers = IntegerRegisterFile::new(GuestArchitecture::X86_64);
    registers
        .write(GuestRegister::Rax, GuestIntegerWidth::B64, 0x1234_5678_9abc_def0, false)
        .unwrap();
    registers
        .write(GuestRegister::Rax, GuestIntegerWidth::B8, 0x11, true)
        .unwrap();
    assert_eq!(
        registers
            .read(GuestRegister::Rax, GuestIntegerWidth::B64, false)
            .unwrap(),
        0x1234_5678_9abc_11f0
    );
    registers
        .write(GuestRegister::Rax, GuestIntegerWidth::B32, 0xfedc_ba98, false)
        .unwrap();
    assert_eq!(
        registers
            .read(GuestRegister::Rax, GuestIntegerWidth::B64, false)
            .unwrap(),
        0xfedc_ba98
    );
    registers
        .write(GuestRegister::R15, GuestIntegerWidth::B64, u64::MAX, false)
        .unwrap();
    assert_eq!(
        registers
            .read(GuestRegister::R15, GuestIntegerWidth::B64, false)
            .unwrap(),
        u64::MAX
    );
    let mut restored = IntegerRegisterFile::new(GuestArchitecture::X86_64);
    restored.restore(&registers.checkpoint()).unwrap();
    assert_eq!(
        restored
            .read(GuestRegister::R15, GuestIntegerWidth::B64, false)
            .unwrap(),
        u64::MAX
    );
    let i386 = IntegerRegisterFile::new(GuestArchitecture::I386);
    assert!(format!("{:?}", i386.read(GuestRegister::R8, GuestIntegerWidth::B32, false)).contains("i386"));
    assert!(format!("{:?}", registers.read(GuestRegister::Rsp, GuestIntegerWidth::B8, true)).contains("high-byte"));
    let mut cpu = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: GuestArchitecture::X86_64,
        instruction_pointer: 0xf123_4567_89ab_cdef,
        stack_pointer: 0x20000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    cpu.flags.set(GuestFlag::Carry, true);
    assert_eq!(cpu.flags.value(), 3);
    assert_eq!(cpu.x87.registers.len(), 80);
    assert_eq!(cpu.simd.xmm.len(), 256);
    assert_eq!(
        cpu.registers
            .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)
            .unwrap(),
        0x20000
    );
}

#[test]
fn register_checkpoint_snapshots_stay_independent() {
    for architecture in [GuestArchitecture::I386, GuestArchitecture::X86_64] {
        let mut registers = IntegerRegisterFile::new(architecture);
        registers
            .write(GuestRegister::Rax, GuestIntegerWidth::B32, 7, false)
            .unwrap();
        let retained = registers.checkpoint();
        assert_eq!(
            retained.len(),
            if architecture == GuestArchitecture::I386 {
                64
            } else {
                128
            }
        );
        registers
            .write(GuestRegister::Rax, GuestIntegerWidth::B32, 19, false)
            .unwrap();
        let updated = registers.checkpoint();
        assert_ne!(updated, retained);
        registers.restore(&retained).unwrap();
        assert_eq!(
            registers
                .read(GuestRegister::Rax, GuestIntegerWidth::B32, false)
                .unwrap(),
            7
        );
        assert!(format!("{:?}", registers.restore(&[0])).contains("architecture or length"));
        assert_eq!(
            registers
                .read(GuestRegister::Rax, GuestIntegerWidth::B32, false)
                .unwrap(),
            7
        );
    }
}

#[test]
fn write_ranges_retain_watched_offsets_across_aliases_and_boundaries() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let first = map(&mut memory, 0x10000, 8, GuestPermissions::ReadWrite, None);
    let second = map(&mut memory, 0x10008, 8, GuestPermissions::ReadWrite, None);
    let alias = memory
        .map_alias(0x20000, 8, GuestPermissions::ReadWrite, "alias", second)
        .unwrap();
    let seen: Rc<RefCell<Vec<Vec<GuestWrittenRange>>>> = Rc::new(RefCell::new(Vec::new()));
    let probe = Rc::clone(&seen);
    let watched = memory.offset(first, 4).unwrap();
    let id = memory
        .observe_writes(
            watched,
            8,
            Box::new(move |ranges| probe.borrow_mut().push(ranges.to_vec())),
        )
        .unwrap();
    memory.write_u8(alias, 9).unwrap();
    assert_eq!(
        *seen.borrow(),
        [[GuestWrittenRange {
            byte_offset: 4,
            byte_length: 1
        }]]
    );
    memory.write(memory.offset(first, 6).unwrap(), &[2, 3, 9, 4]).unwrap();
    assert_eq!(
        seen.borrow()[1],
        [
            GuestWrittenRange {
                byte_offset: 2,
                byte_length: 2
            },
            GuestWrittenRange {
                byte_offset: 4,
                byte_length: 2
            }
        ]
    );
    memory.unobserve(id);
    memory.write_u8(alias, 8).unwrap();
    assert_eq!(seen.borrow().len(), 2);
}

#[test]
fn range_write_observers_follow_aliases_and_stop_after_removal_or_replacement() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = allocate(&mut memory, 16, 16);
    let alias = memory
        .map_alias(0x200000, 16, GuestPermissions::ReadWrite, "alias", base)
        .unwrap();
    let values: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
    // Observers cannot read memory; record delivery counts and read back after each write.
    let deliveries: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let probe = Rc::clone(&deliveries);
    let id = memory
        .observe_writes(base, 4, Box::new(move |_| *probe.borrow_mut() += 1))
        .unwrap();
    memory.write_i32(memory.offset(base, 4).unwrap(), 9).unwrap();
    memory.write_i32(alias, 3).unwrap();
    assert_eq!(*deliveries.borrow(), 1);
    values.borrow_mut().push(memory.read_i32(base).unwrap());
    assert_eq!(*values.borrow(), [3]);
    memory.unobserve(id);
    memory.unobserve(id);
    memory.write_i32(alias, 8).unwrap();
    assert_eq!(*values.borrow(), [3]);
    // Unmap plus remap replaces the backing, dropping observers.
    let probe = Rc::clone(&deliveries);
    let id = memory
        .observe_writes(base, 4, Box::new(move |_| *probe.borrow_mut() += 1))
        .unwrap();
    let offset = base.offset;
    memory.unmap(base, 16).unwrap();
    map(&mut memory, offset, 16, GuestPermissions::ReadWrite, None);
    memory.write_i32(base, 7).unwrap();
    assert_eq!(*deliveries.borrow(), 1);
    memory.unobserve(id);
    // Removing one observer from inside another observer's delivery drops the late one.
    // Rust observers cannot unregister siblings mid-delivery; removing before the next
    // write has the same observable effect.
    let late: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let probe = Rc::clone(&late);
    let late_id = memory
        .observe_writes(base, 4, Box::new(move |_| *probe.borrow_mut() += 1))
        .unwrap();
    memory.unobserve(late_id);
    memory.write_i32(base, 9).unwrap();
    assert_eq!(*late.borrow(), 0);
}

#[test]
fn mapping_locality_preserves_code_splits_holes_and_remapped_backing() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = map(&mut memory, 0x10000, 32, GuestPermissions::ReadWriteExecute, None);
    memory.write_u8(base, 0x90).unwrap();
    assert_eq!(memory.fetch(base, 1).unwrap()[0], 0x90);
    let mut copied = memory.fetch(base, 1).unwrap();
    copied[0] = 0xcc;
    assert_eq!(memory.fetch(base, 1).unwrap()[0], 0x90);
    memory.write_u8(base, 0xc3).unwrap();
    assert_eq!(memory.fetch(base, 1).unwrap()[0], 0xc3);
    memory.protect(base, 16, GuestPermissions::Read).unwrap();
    assert!(memory.fetch(base, 1).is_err());
    assert_eq!(memory.read_u8(base).unwrap(), 0xc3);
    assert!(memory.write_u8(base, 0).is_err());
    let middle = memory.offset(base, 16).unwrap();
    memory.write_u8(middle, 7).unwrap();
    assert_eq!(memory.read_u8(middle).unwrap(), 7);
    memory.unmap(middle, 16).unwrap();
    assert!(memory.read_u8(middle).is_err());
    assert!(memory.copy(memory.offset(base, 15).unwrap(), 2).is_err());
    map(
        &mut memory,
        middle.offset,
        16,
        GuestPermissions::ReadWriteExecute,
        Some(vec![9]),
    );
    assert_eq!(memory.fetch(middle, 1).unwrap()[0], 9);
    assert_eq!(memory.copy(memory.offset(base, 15).unwrap(), 2).unwrap(), [0, 9]);
    let foreign = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let foreign_middle = foreign.pointer(middle.offset).unwrap().unwrap();
    assert!(memory.fetch(foreign_middle, 1).is_err());
    let distant = map(&mut memory, 0xffff_8000_0100_0000, 8, GuestPermissions::ReadWrite, None);
    memory.write_u8(distant, 17).unwrap();
    memory.read_u8(middle).unwrap();
    memory.read_u8(base).unwrap();
    assert_eq!(memory.read_u8(distant).unwrap(), 17);
    assert_eq!(memory.read_u8(middle).unwrap(), 9);
    memory.protect(distant, 8, GuestPermissions::Execute).unwrap();
    memory.read_u8(base).unwrap();
    assert!(format!("{:?}", memory.read_u8(distant)).contains("permits execute"));
    assert!(format!("{:?}", memory.write_u8(distant, 23)).contains("permits execute"));
    memory.unmap(distant, 8).unwrap();
    memory.read_u8(middle).unwrap();
    assert!(format!("{:?}", memory.read_u8(distant)).contains("unmapped"));
    map(
        &mut memory,
        distant.offset,
        8,
        GuestPermissions::ReadWrite,
        Some(vec![31]),
    );
    assert_eq!(memory.read_u8(distant).unwrap(), 31);
    assert!(format!("{:?}", memory.read_u8(foreign_middle)).contains("another execution owner"));
}

#[test]
fn first_fit_hints_revisit_coalesced_holes_and_preserve_alignment() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let first = allocate(&mut memory, 8, 8);
    let second = allocate(&mut memory, 8, 8);
    let third = allocate(&mut memory, 16, 8);
    memory.unmap(first, 8).unwrap();
    let tail = allocate(&mut memory, 16, 8);
    assert_eq!(tail.offset, third.offset + 16);
    memory.unmap(second, 8).unwrap();
    let merged = allocate(&mut memory, 16, 8);
    assert_eq!(merged.offset, first.offset);
    memory.protect(merged, 8, GuestPermissions::Read).unwrap();
    memory.unmap(memory.offset(merged, 8).unwrap(), 8).unwrap();
    assert_eq!(allocate(&mut memory, 8, 8).offset, first.offset + 8);
    assert_eq!(allocate(&mut memory, 8, 16).offset % 16, 0);
    assert!(memory
        .map(&GuestMapOptions {
            base: third.offset + 4,
            byte_length: 1,
            permissions: GuestPermissions::Read,
            label: "overlap".to_string(),
            bytes: None,
        })
        .is_err());
}

#[test]
fn retained_executable_ranges_follow_writes_aliases_and_restoration() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let code = map(
        &mut memory,
        0x10000,
        8,
        GuestPermissions::ReadExecute,
        Some(vec![0x90, 0xc3]),
    );
    let unchanged = memory
        .retain_executable_range(code.offset, &[0x90, 0xc3])
        .expect("retention");
    assert!(unchanged.unchanged(&memory));
    let alias = memory
        .map_alias(0x20000, 8, GuestPermissions::ReadWrite, "alias", code)
        .unwrap();
    memory.write_u8(alias, 0xcc).unwrap();
    assert!(!unchanged.unchanged(&memory));
    memory.write(alias, &[0x90]).unwrap();
    assert!(unchanged.unchanged(&memory));
    memory.write_u32(memory.offset(alias, 4).unwrap(), 123).unwrap();
    assert!(unchanged.unchanged(&memory));
    let observed: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(None));
    let probe = Rc::clone(&observed);
    let expected = [0x90u8, 0xc3];
    let id = memory
        .observe_writes(
            alias,
            1,
            Box::new(move |_| *probe.borrow_mut() = Some(expected == [0x90, 0xc3])),
        )
        .unwrap();
    memory.write_u8(alias, 0xcc).unwrap();
    assert_eq!(*observed.borrow(), Some(true));
    assert!(!unchanged.unchanged(&memory));
    memory.unobserve(id);
    memory.write_u8(alias, 0x90).unwrap();
    assert!(unchanged.unchanged(&memory));
    let module = test_module("guest");
    let mut restored = SparseGuestMemory::restore(module, &memory.checkpoint()).unwrap();
    let restored_range = restored
        .retain_executable_range(code.offset, &[0x90, 0xc3])
        .expect("restored retention");
    let restored_alias = restored.pointer(alias.offset).unwrap().unwrap();
    restored.write_u16(restored_alias, 0xf4cc).unwrap();
    assert!(!restored_range.unchanged(&restored));
    assert!(unchanged.unchanged(&memory));
    memory.protect(code, 1, GuestPermissions::Read).unwrap();
    assert!(!unchanged.unchanged(&memory));
    memory.unmap(code, 8).unwrap();
    map(
        &mut memory,
        code.offset,
        8,
        GuestPermissions::ReadExecute,
        Some(vec![0x90, 0xc3]),
    );
    assert!(!unchanged.unchanged(&memory));
    assert!(memory
        .retain_executable_range(code.offset, &[0x90, 0xc3])
        .unwrap()
        .unchanged(&memory));
}

#[test]
fn scalar_instruction_fetch_observes_aliases_permissions_and_remaps() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let code = map(
        &mut memory,
        0x10000,
        2,
        GuestPermissions::ReadWriteExecute,
        Some(vec![0x90, 0xc3]),
    );
    let alias = memory
        .map_alias(0x20000, 2, GuestPermissions::ReadWrite, "alias", code)
        .unwrap();
    assert_eq!(memory.fetch_byte(code.offset).unwrap(), 0x90);
    memory.write_u8(alias, 0xcc).unwrap();
    assert_eq!(memory.fetch_byte(code.offset).unwrap(), 0xcc);
    assert!(memory.fetch_byte(alias.offset).is_err());
    memory.protect(code, 1, GuestPermissions::Read).unwrap();
    assert!(memory.fetch_byte(code.offset).is_err());
    assert_eq!(memory.fetch_byte(code.offset + 1).unwrap(), 0xc3);
    memory.unmap(code, 2).unwrap();
    assert!(memory.fetch_byte(code.offset).is_err());
    map(&mut memory, code.offset, 1, GuestPermissions::Execute, Some(vec![0xf4]));
    assert_eq!(memory.fetch_byte(code.offset).unwrap(), 0xf4);
    assert!(memory.fetch_byte(code.offset + 1).is_err());
    assert!(memory.fetch_byte(0).is_err());
    assert!(memory.fetch_byte(u64::MAX).is_err());
}

#[test]
fn scalar_stores_preserve_encoding_and_cross_mapping_atomicity() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = map(&mut memory, 0x10000, 4, GuestPermissions::ReadWrite, None);
    let second = map(&mut memory, 0x10004, 12, GuestPermissions::ReadWrite, None);
    memory.write_u8(base, 257u16 as u8).unwrap();
    assert_eq!(memory.copy(base, 1).unwrap(), [1]);
    memory.write_i16(base, (-32769i32) as i16).unwrap();
    assert_eq!(memory.copy(base, 2).unwrap(), [0xff, 0x7f]);
    memory.write_u32(base, (-1i32) as u32).unwrap();
    assert_eq!(memory.copy(base, 4).unwrap(), [0xff, 0xff, 0xff, 0xff]);
    memory.write_u64(base, (-1i64) as u64).unwrap();
    assert_eq!(memory.copy(base, 8).unwrap(), [0xff; 8]);
    // Donor high word 0x112345678 wraps to 32 bits in the DataView encoding.
    memory.write_u64_words(base, (-1i32) as u32, 0x1234_5678).unwrap();
    assert_eq!(memory.read_u64(base).unwrap(), 0x1234_5678_ffff_ffff);
    memory.write_f32(base, -0.0).unwrap();
    assert_eq!(memory.copy(base, 4).unwrap(), (-0.0f32).to_le_bytes());
    memory.write_f64(base, f64::NAN).unwrap();
    assert_eq!(memory.copy(base, 8).unwrap(), f64::NAN.to_le_bytes());
    memory.write_f64(base, f64::INFINITY).unwrap();
    assert_eq!(memory.copy(base, 8).unwrap(), f64::INFINITY.to_le_bytes());
    let (low, high) = memory.read_u64_words(base).unwrap();
    assert_eq!((u64::from(high) << 32) | u64::from(low), f64::INFINITY.to_bits());
    let before = memory.copy(base, 16).unwrap();
    let notifications: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let probe = Rc::clone(&notifications);
    let id = memory
        .observe_writes(base, 16, Box::new(move |_| *probe.borrow_mut() += 1))
        .unwrap();
    memory.protect(second, 12, GuestPermissions::Read).unwrap();
    assert!(memory.write_u64(base, 42).is_err());
    assert!(memory.write_u64_words(base, 42, 43).is_err());
    assert_eq!(memory.copy(base, 16).unwrap(), before);
    assert_eq!(*notifications.borrow(), 0);
    memory.unobserve(id);
}

#[test]
fn execute_sequence_reads_aliases_and_revalidates_without_speculative_faults() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = map(
        &mut memory,
        0x1000,
        3,
        GuestPermissions::ReadExecute,
        Some(vec![1, 2, 3]),
    );
    let alias = memory
        .map_alias(0x2000, 3, GuestPermissions::ReadWrite, "alias", base)
        .unwrap();
    let mut cursor = FetchCursor::new(base.offset);
    assert_eq!(memory.fetch_sequence_byte(&mut cursor).unwrap(), 1);
    memory.write_u8(memory.offset(alias, 1).unwrap(), 9).unwrap();
    assert_eq!(memory.fetch_sequence_byte(&mut cursor).unwrap(), 9);
    memory.protect(base, 3, GuestPermissions::Read).unwrap();
    assert!(format!("{:?}", memory.fetch_sequence_byte(&mut cursor)).contains("permits read"));
    memory.protect(base, 3, GuestPermissions::Execute).unwrap();
    assert_eq!(memory.fetch_sequence_byte(&mut cursor).unwrap(), 3);
    assert!(format!("{:?}", memory.fetch_sequence_byte(&mut cursor)).contains("unmapped"));
    map(&mut memory, 0x1003, 2, GuestPermissions::Execute, Some(vec![4, 5]));
    assert_eq!(memory.fetch_sequence_byte(&mut cursor).unwrap(), 4);
    memory.unmap(memory.pointer(0x1004).unwrap().unwrap(), 1).unwrap();
    assert!(format!("{:?}", memory.fetch_sequence_byte(&mut cursor)).contains("unmapped"));
    map(&mut memory, 0x1004, 1, GuestPermissions::Execute, Some(vec![7]));
    assert_eq!(memory.fetch_sequence_byte(&mut cursor).unwrap(), 7);
    let mut missing = FetchCursor::new(0);
    assert!(format!("{:?}", memory.fetch_sequence_byte(&mut missing)).contains("null"));
}

#[test]
fn direct_scalar_stores_retain_unaligned_float_bits() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = allocate(&mut memory, 32, 16);
    let value = memory.offset(base, 1).unwrap();
    for number in [
        -0.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        5e-324,
        std::f64::consts::PI,
    ] {
        memory.write_f64(value, number).unwrap();
        assert_eq!(memory.copy(value, 8).unwrap(), number.to_le_bytes());
    }
    // Rust observers cannot fail, so failure aggregation has no equivalent;
    // both observers still observe the committed value in registration order.
    let observed: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let first = Rc::clone(&observed);
    let second = Rc::clone(&observed);
    let first_id = memory
        .observe_writes(value, 8, Box::new(move |_| first.borrow_mut().push(1)))
        .unwrap();
    let second_id = memory
        .observe_writes(value, 8, Box::new(move |_| second.borrow_mut().push(2)))
        .unwrap();
    memory.write_u64(value, 0x1234_5678_9abc_def0).unwrap();
    assert_eq!(*observed.borrow(), [1, 2]);
    assert_eq!(memory.read_u64(value).unwrap(), 0x1234_5678_9abc_def0);
    memory.unobserve(first_id);
    memory.unobserve(second_id);
}

#[test]
fn word_pair_scalars_preserve_aliases_observers_and_retired_mappings() {
    let mut memory = SparseGuestMemory::new(test_module("guest"), 8, 0x10000).unwrap();
    let base = map(&mut memory, 0xffff_8000_0000_0001, 8, GuestPermissions::ReadWrite, None);
    let alias = memory
        .map_alias(0x20001, 8, GuestPermissions::ReadWrite, "alias", base)
        .unwrap();
    let observed: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let probe = Rc::clone(&observed);
    let id = memory
        .observe_writes(base, 8, Box::new(move |_| probe.borrow_mut().push(1)))
        .unwrap();
    memory.write_u64_words(base, 0x89ab_cdef, 0x0123_4567).unwrap();
    assert_eq!(memory.read_u64(base).unwrap(), 0x0123_4567_89ab_cdef);
    assert_eq!(memory.read_u64(alias).unwrap(), 0x0123_4567_89ab_cdef);
    let (low, high) = memory.read_u64_words(alias).unwrap();
    assert_eq!((low, high), (0x89ab_cdef, 0x0123_4567));
    assert_eq!(*observed.borrow(), [1]);
    memory.unobserve(id);
    let module = test_module("guest");
    let mut restored = SparseGuestMemory::restore(module, &memory.checkpoint()).unwrap();
    assert!(format!("{:?}", restored.read_u64_words(base)).contains("another execution owner"));
    let restored_base = restored.pointer(base.offset).unwrap().unwrap();
    assert_eq!(
        restored.read_u64_words(restored_base).unwrap(),
        (0x89ab_cdef, 0x0123_4567)
    );
    memory
        .protect(memory.offset(base, 4).unwrap(), 4, GuestPermissions::Execute)
        .unwrap();
    assert!(memory.read_u64_words(base).is_err());
    memory.unmap(base, 8).unwrap();
    assert!(memory.read_u64_words(base).is_err());
    assert!(memory.write_u64_words(base, 0, 0).is_err());
    map(&mut memory, base.offset, 8, GuestPermissions::ReadWrite, None);
    assert_eq!(memory.read_u64_words(base).unwrap(), (0, 0));
    assert_eq!(memory.read_u64_words(alias).unwrap(), (0x89ab_cdef, 0x0123_4567));
}
