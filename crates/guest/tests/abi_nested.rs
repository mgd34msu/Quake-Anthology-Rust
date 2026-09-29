//! Nested guest-call tests: real CALL/RET through host callbacks,
//! exceptions, save/restore rebinding, and loading slices.
//!
//! Donor: `tests/guest/abi/nested.test.ts`. Processor snapshots use the
//! register checkpoint plus XMM bytes; reentry-while-suspended is enforced
//! by the borrow checker instead of a runtime trap.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use qa_guest::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::{GuestHostCallback, HookState};
use qa_guest::core::contracts::{
    CallbackId, GuestAddress, GuestArchitecture, GuestCallContext, GuestCallResult,
    GuestCallSignature, GuestCallValue, GuestCallbackReference, GuestExecutionStop,
    GuestIntegerWidth, GuestPermissions, GuestRegister, GuestStorage, GuestValueLayout,
    ModuleIdentity, NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use qa_guest::error::GuestError;
use qa_guest::x64::cpu::X64Cpu;
use qa_guest::x86::cpu::I386Cpu;

use common::{addr, map, test_module};

fn module() -> ModuleIdentity {
    test_module("nested-abi")
}

fn call_bytes(abi: NativeCallAbi, callback: u64, increment: u8) -> Vec<u8> {
    let width = abi.pointer_bytes();
    let immediate: Vec<u8> = (0..width).map(|index| (callback >> (index * 8)) as u8).collect();
    if width == 4 {
        let mut bytes = vec![0x83, 0xec, 8, 0xff, 0x74, 0x24, 12, 0xb8];
        bytes.extend_from_slice(&immediate);
        bytes.extend_from_slice(&[0xff, 0xd0, 0x83, 0xc4, 12, 0x83, 0xc0, increment, 0xc3]);
        bytes
    } else {
        let reserve = if abi == NativeCallAbi::MicrosoftX64 { 40 } else { 8 };
        let mut bytes = vec![0x48, 0x83, 0xec, reserve, 0x48, 0xb8];
        bytes.extend_from_slice(&immediate);
        bytes.extend_from_slice(&[0xff, 0xd0, 0x48, 0x83, 0xc4, reserve, 0x83, 0xc0, increment, 0xc3]);
        bytes
    }
}

fn processor_state(abi: NativeCallAbi) -> GuestProcessorState {
    GuestProcessorState::create(GuestProcessorInitialState {
        architecture: if abi.pointer_bytes() == 4 {
            GuestArchitecture::I386
        } else {
            GuestArchitecture::X86_64
        },
        instruction_pointer: 0x1800,
        stack_pointer: 0x20000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap()
}

fn root_context(module: &ModuleIdentity, memory: &SparseGuestMemory, abi: NativeCallAbi) -> GuestCallContext {
    let address = addr(memory, 0x1000);
    GuestCallContext {
        module: module.clone(),
        callback: GuestCallbackReference::NativeGuest { module: module.clone(), address, abi },
        parent: None,
        itself: None,
        other: None,
    }
}

fn int_signature(abi: NativeCallAbi) -> GuestCallSignature {
    GuestCallSignature {
        abi,
        parameters: vec![GuestValueLayout::Scalar(GuestStorage::Int32)],
        result: Some(GuestValueLayout::Scalar(GuestStorage::Int32)),
        variadic: false,
    }
}

fn nested_case(abi: NativeCallAbi) {
    let module = module();
    let width = abi.pointer_bytes();
    let mut memory = SparseGuestMemory::new(module.clone(), width, 0x30000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadWrite, None);
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let hooks = Rc::new(HookState::new());
    let signature = int_signature(abi);
    let depths: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let parents: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    let level: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let invoke_depths = Rc::clone(&depths);
    let invoke_parents = Rc::clone(&parents);
    let invoke_level = Rc::clone(&level);
    let invoke_module = module.clone();
    let invoke_signature = signature.clone();
    let callback = GuestHostCallback {
        id: CallbackId::new("test", "nested-host"),
        signature: signature.clone(),
        invoke: Rc::new(move |ctx, context, arguments| {
            invoke_level.set(invoke_level.get() + 1);
            invoke_depths.borrow_mut().push(invoke_level.get());
            invoke_parents.borrow_mut().push(context.parent.is_some());
            let result = (|| {
                let Some(GuestCallValue::Int32(argument)) = arguments.first() else {
                    return Err(GuestError::invalid("Wrong callback argument"));
                };
                if *argument == 41 {
                    let target = addr(ctx.memory(), 0x1100);
                    let request = GuestCallRequest {
                        target,
                        signature: invoke_signature.clone(),
                        arguments: vec![GuestCallValue::Int32(42)],
                        context: GuestCallContext {
                            module: invoke_module.clone(),
                            callback: GuestCallbackReference::NativeGuest {
                                module: invoke_module.clone(),
                                address: target,
                                abi,
                            },
                            parent: Some(Box::new(context.clone())),
                            itself: None,
                            other: None,
                        },
                        instruction_budget: 64,
                    };
                    return ctx.invoke(&request).map_err(|failure| match failure {
                        GuestCallFailure::Guest(error) => error,
                        GuestCallFailure::Stopped { stop, .. } => {
                            GuestError::callback(format!("nested call stopped: {stop:?}"))
                        }
                    });
                }
                let grandparent = context.parent.as_ref().and_then(|parent| parent.parent.as_ref());
                match grandparent.map(|parent| &parent.callback) {
                    Some(GuestCallbackReference::NativeGuest { .. }) => {}
                    other => return Err(GuestError::callback(format!("unexpected grandparent: {other:?}"))),
                }
                let slot = addr(ctx.memory(), 0x19000);
                ctx.memory().write_u32(slot, (argument * 2) as u32)?;
                Ok(GuestCallResult::Value(GuestCallValue::Int32(argument * 2)))
            })();
            invoke_level.set(invoke_level.get() - 1);
            result
        }),
    };
    let callback_address = hooks.callbacks.borrow_mut().bind(&mut memory, callback.clone()).unwrap();
    assert!(hooks.callbacks.borrow().has_bound_trap(callback_address.offset));
    assert!(!hooks.callbacks.borrow().has_bound_trap(0x1000));
    memory.write(addr(&memory, 0x1000), &call_bytes(abi, callback_address.offset, 3)).unwrap();
    memory.write(addr(&memory, 0x1100), &call_bytes(abi, callback_address.offset, 1)).unwrap();
    memory.write(addr(&memory, 0x1800), &[0xcc]).unwrap();
    memory.protect(addr(&memory, 0x1000), 4096, GuestPermissions::ReadExecute).unwrap();
    let entry = addr(&memory, 0x1800);
    let mut probe = callback.clone();
    probe.id = CallbackId::new("test", "nested-entry");
    hooks.callbacks.borrow_mut().bind_entry(&mut memory, entry, probe, Rc::new(|| true)).unwrap();
    assert!(!hooks.callbacks.borrow().has_bound_trap(0x1800));
    hooks.callbacks.borrow_mut().unhook_entry(entry);
    let state = processor_state(abi);
    let context = root_context(&module, &memory, abi);
    if width == 4 {
        let mut cpu = I386Cpu::new(state, memory).unwrap();
        run_nested_case(&mut cpu, hooks, context, signature);
    } else {
        let mut cpu = X64Cpu::new(state, memory).unwrap();
        run_nested_case(&mut cpu, hooks, context, signature);
    }
    assert_eq!(*depths.borrow(), vec![1, 2]);
    assert_eq!(*parents.borrow(), vec![true, true]);
}

fn run_nested_case(
    cpu: &mut dyn GuestCpu,
    hooks: Rc<HookState>,
    context: GuestCallContext,
    signature: GuestCallSignature,
) {
    let return_address = cpu.parts().1.pointer(0x1800).unwrap().unwrap();
    let mut runner = GuestCallRunner::new(cpu, hooks, return_address, None).unwrap();
    let target = runner.cpu_parts().1.pointer(0x1000).unwrap().unwrap();
    let request = GuestCallRequest {
        target,
        signature: signature.clone(),
        arguments: vec![GuestCallValue::Int32(41)],
        context: context.clone(),
        instruction_budget: 100,
    };
    let result = runner.invoke(&request).unwrap();
    assert_eq!(result, GuestCallResult::Value(GuestCallValue::Int32(88)));
    let (state, memory) = runner.cpu_parts();
    assert_eq!(memory.read_u32(addr(memory, 0x19000)).unwrap(), 84);
    let width = if memory.pointer_bytes() == 4 { GuestIntegerWidth::B32 } else { GuestIntegerWidth::B64 };
    assert_eq!(state.registers.read(GuestRegister::Rsp, width, false).unwrap(), 0x20000);
    drop(state);
    drop(memory);
    assert_eq!(runner.depth(), 0);
    let (saved_registers, saved_xmm) = {
        let (state, _) = runner.cpu_parts();
        (state.registers.checkpoint(), state.simd.xmm.clone())
    };
    {
        let (state, _) = runner.cpu_parts();
        state.registers.write(GuestRegister::Rax, GuestIntegerWidth::B32, 999, false).unwrap();
        state.simd.xmm.fill(123);
    }
    {
        let (state, _) = runner.cpu_parts();
        state.registers.restore(&saved_registers).unwrap();
        state.simd.xmm = saved_xmm.clone();
    }
    {
        let (state, _) = runner.cpu_parts();
        assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), 88);
        assert_eq!(state.simd.xmm, saved_xmm);
    }
}

#[test]
fn nested_calls_preserve_memory_registers_and_depth() {
    for abi in [
        NativeCallAbi::Cdecl,
        NativeCallAbi::SystemVI386,
        NativeCallAbi::MicrosoftX64,
        NativeCallAbi::SystemVX86_64,
    ] {
        nested_case(abi);
    }
}

#[test]
fn rebind_after_unbind_restores_the_same_trap() {
    let abi = NativeCallAbi::SystemVX86_64;
    let module = module();
    let mut memory = SparseGuestMemory::new(module.clone(), 8, 0x30000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadWrite, None);
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let hooks = Rc::new(HookState::new());
    let signature = int_signature(abi);
    let callback = GuestHostCallback {
        id: CallbackId::new("test", "rebind-host"),
        signature: signature.clone(),
        invoke: Rc::new(|_, _, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(0)))),
    };
    let address = hooks.callbacks.borrow_mut().bind(&mut memory, callback.clone()).unwrap();
    memory.write(addr(&memory, 0x1000), &call_bytes(abi, address.offset, 3)).unwrap();
    memory.protect(addr(&memory, 0x1000), 4096, GuestPermissions::ReadExecute).unwrap();
    let state = processor_state(abi);
    let context = root_context(&module, &memory, abi);
    let mut cpu = X64Cpu::new(state, memory).unwrap();
    let return_address = cpu.parts().1.pointer(0x1800).unwrap().unwrap();
    let mut runner = GuestCallRunner::new(&mut cpu, Rc::clone(&hooks), return_address, None).unwrap();
    hooks.callbacks.borrow_mut().unbind(&callback.id);
    assert!(!hooks.callbacks.borrow().has_bound_trap(address.offset));
    let target = runner.cpu_parts().1.pointer(0x1000).unwrap().unwrap();
    let request = GuestCallRequest {
        target,
        signature: signature.clone(),
        arguments: vec![GuestCallValue::Int32(42)],
        context,
        instruction_budget: 30,
    };
    let error = runner.invoke(&request).unwrap_err();
    assert!(format!("{error:?}").contains("unbound"), "unexpected: {error:?}");
    let rebound = hooks.callbacks.borrow_mut().bind(runner.cpu_parts().1, callback).unwrap();
    assert_eq!(rebound, address);
    assert!(hooks.callbacks.borrow().has_bound_trap(address.offset));
}

#[test]
fn processor_exception_preserves_the_guest_frame() {
    let abi = NativeCallAbi::MicrosoftX64;
    let module = module();
    let mut memory = SparseGuestMemory::new(module.clone(), 8, 0x10000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadExecute, Some(vec![0xcc]));
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let state = processor_state(abi);
    let context = root_context(&module, &memory, abi);
    let mut cpu = X64Cpu::new(state, memory).unwrap();
    let return_address = cpu.parts().1.pointer(0x1800).unwrap().unwrap();
    let hooks = Rc::new(HookState::new());
    let mut runner = GuestCallRunner::new(&mut cpu, hooks, return_address, None).unwrap();
    let target = runner.cpu_parts().1.pointer(0x1000).unwrap().unwrap();
    let request = GuestCallRequest {
        target,
        signature: GuestCallSignature { abi, parameters: vec![], result: None, variadic: false },
        arguments: vec![],
        context,
        instruction_budget: 8,
    };
    match runner.invoke(&request) {
        Err(GuestCallFailure::Stopped { stop, .. }) => assert_eq!(stop.kind(), "exception"),
        other => panic!("expected a stopped exception, got {other:?}"),
    }
    let (state, _) = runner.cpu_parts();
    assert_ne!(state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false).unwrap(), 0x20000);
    drop(state);
    assert_eq!(runner.depth(), 0);
}

#[test]
fn saved_code_and_callback_identities_rebind_before_execution() {
    let abi = NativeCallAbi::MicrosoftX64;
    let module = module();
    let mut memory = SparseGuestMemory::new(module.clone(), 8, 0x30000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadWrite, None);
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let hooks = Rc::new(HookState::new());
    let signature = int_signature(abi);
    let callback = GuestHostCallback {
        id: CallbackId::new("test", "restored-callback"),
        signature: signature.clone(),
        invoke: Rc::new(|_, _, arguments| {
            let Some(GuestCallValue::Int32(value)) = arguments.first() else {
                return Err(GuestError::invalid("Restored callback argument differs"));
            };
            Ok(GuestCallResult::Value(GuestCallValue::Int32(value + 1)))
        }),
    };
    let old_address = hooks.callbacks.borrow_mut().bind(&mut memory, callback.clone()).unwrap();
    memory.write(addr(&memory, 0x1000), &call_bytes(abi, old_address.offset, 3)).unwrap();
    memory.protect(addr(&memory, 0x1000), 4096, GuestPermissions::ReadExecute).unwrap();
    let saved_memory = memory.checkpoint();
    let saved_callbacks = hooks.callbacks.borrow().checkpoint();
    hooks.callbacks.borrow_mut().unbind(&callback.id);
    let mut restored = SparseGuestMemory::restore(module.clone(), &saved_memory).unwrap();
    let rebound_table = qa_guest::core::callbacks::GuestCallbackTable::restore(
        &mut restored,
        &saved_callbacks,
        |id| (id == &callback.id).then(|| callback.clone()),
    )
    .unwrap();
    let rebound = Rc::new(HookState {
        callbacks: RefCell::new(rebound_table),
        call_id: Cell::new(0),
        entry_rsp: Cell::new(0),
    });
    let restored_address = rebound.callbacks.borrow().address(&callback.id).expect("rebound address");
    assert_eq!(restored_address.offset, old_address.offset);
    assert!(rebound.callbacks.borrow().has_bound_trap(restored_address.offset));
    assert_ne!(restored_address.space, old_address.space);
    let state = processor_state(abi);
    let context = root_context(&module, &restored, abi);
    let mut cpu = X64Cpu::new(state, restored).unwrap();
    let return_address = cpu.parts().1.pointer(0x1800).unwrap().unwrap();
    let mut runner = GuestCallRunner::new(&mut cpu, rebound, return_address, None).unwrap();
    let target = runner.cpu_parts().1.pointer(0x1000).unwrap().unwrap();
    let request = GuestCallRequest {
        target,
        signature: signature.clone(),
        arguments: vec![GuestCallValue::Int32(5)],
        context,
        instruction_budget: 30,
    };
    assert_eq!(
        runner.invoke(&request).unwrap(),
        GuestCallResult::Value(GuestCallValue::Int32(9))
    );
    let hooks = Rc::clone(runner.hooks());
    let error = hooks.callbacks.borrow_mut().resolve(runner.cpu_parts().1, old_address).unwrap_err();
    assert!(format!("{error:?}").contains("another execution owner"), "unexpected: {error:?}");
}

#[test]
fn win32_stdcall_ret_immediate_must_match_cleanup() {
    let abi = NativeCallAbi::Stdcall;
    let module = module();
    let mut memory = SparseGuestMemory::new(module.clone(), 4, 0x10000).unwrap();
    map(
        &mut memory,
        0x1000,
        4096,
        GuestPermissions::ReadExecute,
        Some(vec![0xb8, 42, 0, 0, 0, 0xc2, 8, 0]),
    );
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let state = processor_state(abi);
    let context = root_context(&module, &memory, abi);
    let mut cpu = I386Cpu::new(state, memory).unwrap();
    let return_address = cpu.parts().1.pointer(0x1800).unwrap().unwrap();
    let hooks = Rc::new(HookState::new());
    let mut runner = GuestCallRunner::new(&mut cpu, hooks, return_address, None).unwrap();
    let target = runner.cpu_parts().1.pointer(0x1000).unwrap().unwrap();
    let signature = GuestCallSignature {
        abi,
        parameters: vec![i32_layout(), i32_layout()],
        result: Some(i32_layout()),
        variadic: false,
    };
    let request = GuestCallRequest {
        target,
        signature,
        arguments: vec![GuestCallValue::Int32(7), GuestCallValue::Int32(9)],
        context,
        instruction_budget: 8,
    };
    assert_eq!(runner.invoke(&request).unwrap(), GuestCallResult::Value(GuestCallValue::Int32(42)));
    {
        let (_, memory) = runner.cpu_parts();
        memory.protect(addr(memory, 0x1000), 4096, GuestPermissions::ReadWrite).unwrap();
        memory.write(addr(memory, 0x1005), &[0xc3]).unwrap();
        memory.protect(addr(memory, 0x1000), 4096, GuestPermissions::ReadExecute).unwrap();
    }
    let error = runner.invoke(&request).unwrap_err();
    assert!(format!("{error:?}").contains("incorrect ABI stack cleanup"), "unexpected: {error:?}");
}

fn i32_layout() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Int32)
}

#[test]
fn ret_on_the_last_budgeted_instruction_completes() {
    let abi = NativeCallAbi::Cdecl;
    let module = module();
    let mut memory = SparseGuestMemory::new(module.clone(), 4, 0x10000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadExecute, Some(vec![0xc3]));
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let state = processor_state(abi);
    let context = root_context(&module, &memory, abi);
    let mut cpu = I386Cpu::new(state, memory).unwrap();
    let return_address = cpu.parts().1.pointer(0x1800).unwrap().unwrap();
    let hooks = Rc::new(HookState::new());
    let mut runner = GuestCallRunner::new(&mut cpu, hooks, return_address, None).unwrap();
    let target = runner.cpu_parts().1.pointer(0x1000).unwrap().unwrap();
    let request = GuestCallRequest {
        target,
        signature: GuestCallSignature { abi, parameters: vec![], result: None, variadic: false },
        arguments: vec![],
        context,
        instruction_budget: 1,
    };
    assert_eq!(runner.invoke(&request).unwrap(), GuestCallResult::Void);
}

#[test]
fn loading_slices_preserve_state_and_count_instructions() {
    // Reentry-while-suspended is enforced statically: the loading call
    // holds the runner mutably, so no second invoke can compile.
    let abi = NativeCallAbi::Cdecl;
    let module = module();
    let mut memory = SparseGuestMemory::new(module.clone(), 4, 0x40000).unwrap();
    let mut code = vec![0x90u8; 20_006];
    code[20_000..20_006].copy_from_slice(&[0xb8, 42, 0, 0, 0, 0xc3]);
    map(&mut memory, 0x1000, 32768, GuestPermissions::ReadExecute, Some(code));
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let state = processor_state(abi);
    let context = root_context(&module, &memory, abi);
    let mut cpu = I386Cpu::new(state, memory).unwrap();
    let target = cpu.parts().1.pointer(0x1000).unwrap().unwrap();
    let return_address = cpu.parts().1.pointer(0x8000).unwrap().unwrap();
    let hooks = Rc::new(HookState::new());
    let mut runner = GuestCallRunner::new(&mut cpu, hooks, return_address, None).unwrap();
    let request = GuestCallRequest {
        target,
        signature: GuestCallSignature {
            abi,
            parameters: vec![],
            result: Some(i32_layout()),
            variadic: false,
        },
        arguments: vec![],
        context,
        instruction_budget: 30_000,
    };
    let mut yields = 0;
    let result = runner
        .invoke_loading(&request, &mut || {
            yields += 1;
        })
        .unwrap();
    assert_eq!(yields, 1);
    assert_eq!(result, GuestCallResult::Value(GuestCallValue::Int32(42)));
    assert_eq!(runner.instructions_executed(), 20_002);
    assert_eq!(runner.depth(), 0);
    assert_eq!(runner.invoke(&request).unwrap(), result);
    assert_eq!(runner.instructions_executed(), 40_004);
}
