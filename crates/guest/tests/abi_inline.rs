//! Inline-region tests: entry probes, execute/skip continuations, and
//! sticky interceptor failures.
//!
//! Donor: `tests/guest/abi/runner-inline.test.ts`. The donor interceptors
//! can reenter the runner, which Rust continuations forbid by construction;
//! those cases port as single-level equivalents.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use qa_guest::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner, GuestInlineContinuation};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::HookState;
use qa_guest::core::contracts::{
    GuestAccess, GuestArchitecture, GuestCallContext, GuestCallResult, GuestCallSignature,
    GuestCallValue, GuestCallbackReference, GuestExecutionStop, GuestIntegerWidth, GuestPermissions,
    GuestRegister, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use qa_guest::x64::cpu::X64Cpu;
use qa_guest::x86::cpu::I386Cpu;

use common::{addr, map, test_module};

const CODE: [u8; 12] = [0xb8, 1, 0, 0, 0, 0x83, 0xc0, 2, 0x83, 0xc0, 4, 0xc3];

fn abi_for(wide: bool) -> NativeCallAbi {
    if wide { NativeCallAbi::MicrosoftX64 } else { NativeCallAbi::Cdecl }
}

fn signature(abi: NativeCallAbi) -> GuestCallSignature {
    GuestCallSignature {
        abi,
        parameters: vec![],
        result: Some(GuestValueLayout::Scalar(GuestStorage::Int32)),
        variadic: false,
    }
}

fn build_cpu(wide: bool) -> (Box<dyn GuestCpu>, Rc<HookState>) {
    let width = abi_for(wide).pointer_bytes();
    let module = test_module("inline");
    let mut memory = SparseGuestMemory::new(module, width, 0x10000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadWrite, None);
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    memory.write(addr(&memory, 0x1000), &CODE).unwrap();
    memory
        .protect(addr(&memory, 0x1000), 4096, GuestPermissions::ReadExecute)
        .unwrap();
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: if wide { GuestArchitecture::X86_64 } else { GuestArchitecture::I386 },
        instruction_pointer: 0x1000,
        stack_pointer: 0x20000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    let hooks = Rc::new(HookState::new());
    let mut cpu: Box<dyn GuestCpu> = if wide {
        Box::new(X64Cpu::new(state, memory).unwrap())
    } else {
        Box::new(I386Cpu::new(state, memory).unwrap())
    };
    cpu.set_hook_state(Some(Rc::clone(&hooks)));
    (cpu, hooks)
}

fn stop_summary(stop: &GuestExecutionStop) -> (String, u64) {
    let instructions = match stop {
        GuestExecutionStop::Budget { instructions }
        | GuestExecutionStop::Return { instructions, .. }
        | GuestExecutionStop::HostCall { instructions, .. }
        | GuestExecutionStop::Halt { instructions, .. }
        | GuestExecutionStop::Exception { instructions, .. }
        | GuestExecutionStop::Unsupported { instructions, .. } => *instructions,
    };
    (format!("{} {stop:?}", stop.kind()), instructions)
}

#[test]
fn callback_probes_retain_execute_faults_and_observer_ownership() {
    for wide in [false, true] {
        let (mut cpu, hooks) = build_cpu(wide);
        let notifications = Rc::new(Cell::new(0u32));
        let notify = Rc::clone(&notifications);
        let entry = {
            let (_, memory) = cpu.parts();
            memory.pointer(0x1000).unwrap().unwrap()
        };
        {
            let (_, memory) = cpu.parts();
            hooks
                .callbacks
                .borrow_mut()
                .observe_entry(memory, entry, Rc::new(move || notify.set(notify.get() + 1)))
                .unwrap();
        }
        // The execute check fails before observers notify, so the hooked
        // stop carries the fault with no notification.
        {
            let (state, memory) = cpu.parts();
            state.instruction_pointer = 0x1000;
            memory.protect(entry, 4096, GuestPermissions::Read).unwrap();
        }
        let hooked = cpu.run(1, None);
        assert!(matches!(hooked, GuestExecutionStop::Exception { .. }), "{hooked:?}");
        assert_eq!(notifications.get(), 0);
        hooks.callbacks.borrow_mut().unobserve_entry(entry, 0);
        let ordinary = cpu.run(1, None);
        assert_eq!(stop_summary(&ordinary), stop_summary(&hooked));
        assert_eq!(cpu.parts().0.instruction_pointer, 0x1000);

        // Entry probes belong to their address space: entering the observed
        // address against a foreign memory is rejected.
        let mut foreign =
            SparseGuestMemory::new(test_module("foreign"), abi_for(wide).pointer_bytes(), 0x10000)
                .unwrap();
        let rejected = hooks.callbacks.borrow_mut().enter(&mut foreign, entry);
        assert!(rejected.is_err());
        assert!(format!("{:?}", rejected.unwrap_err()).contains("another execution owner"));

        {
            let (_, memory) = cpu.parts();
            memory.protect(entry, 4096, GuestPermissions::ReadExecute).unwrap();
            memory.check(entry, 1, GuestAccess::Execute).unwrap();
        }
        let budgeted = cpu.run(1, None);
        assert!(matches!(budgeted, GuestExecutionStop::Budget { .. }), "{budgeted:?}");
        let (state, _) = cpu.parts();
        assert_eq!(state.registers.read(GuestRegister::Rax, GuestIntegerWidth::B32, false).unwrap(), 1);
    }
}

fn build_runner(wide: bool) -> (GuestCallRunner<'static>, GuestCallRequest, NativeCallAbi) {
    let abi = abi_for(wide);
    let module = test_module("inline-runner");
    let width = abi.pointer_bytes();
    let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
    map(&mut memory, 0x1000, 4096, GuestPermissions::ReadWrite, None);
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    memory.write(addr(&memory, 0x1000), &CODE).unwrap();
    memory
        .protect(addr(&memory, 0x1000), 4096, GuestPermissions::ReadExecute)
        .unwrap();
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: if wide { GuestArchitecture::X86_64 } else { GuestArchitecture::I386 },
        instruction_pointer: 0x1800,
        stack_pointer: 0x20000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    let address = addr(&memory, 0x1000);
    let context = GuestCallContext {
        module: module.clone(),
        callback: GuestCallbackReference::NativeGuest { module, address, abi },
        parent: None,
        itself: None,
        other: None,
    };
    let request = GuestCallRequest {
        target: address,
        signature: signature(abi),
        arguments: vec![],
        context,
        instruction_budget: 30,
    };
    let boxed: Box<dyn GuestCpu> = if wide {
        Box::new(X64Cpu::new(state, memory).unwrap())
    } else {
        Box::new(I386Cpu::new(state, memory).unwrap())
    };
    let leaked: &'static mut dyn GuestCpu = Box::leak(boxed);
    let return_address = leaked.parts().1.pointer(0x1800).unwrap().unwrap();
    let runner = GuestCallRunner::new(leaked, Rc::new(HookState::new()), return_address, None).unwrap();
    (runner, request, abi)
}

fn int_result(result: &GuestCallResult) -> i128 {
    match result {
        GuestCallResult::Value(GuestCallValue::Int32(value)) => i128::from(*value),
        GuestCallResult::Value(GuestCallValue::Int64(value)) => i128::from(*value),
        GuestCallResult::Value(GuestCallValue::Uint32(value)) => i128::from(*value),
        GuestCallResult::Value(GuestCallValue::Uint64(value)) => i128::from(*value),
        other => panic!("expected int result, got {other:?}"),
    }
}

fn result_register(runner: &mut GuestCallRunner<'_>, abi: NativeCallAbi) -> i128 {
    let width = if abi.pointer_bytes() == 8 { GuestIntegerWidth::B64 } else { GuestIntegerWidth::B32 };
    let register = GuestRegister::Rax;
    let (state, _) = runner.cpu_parts();
    state.registers.read(register, width, false).unwrap() as i128
}

#[test]
fn inline_continuations_execute_or_skip_the_region_body() {
    for wide in [false, true] {
        for skip in [false, true] {
            let (mut runner, request, abi) = build_runner(wide);
            let entries = Rc::new(Cell::new(0u32));
            let probe = Rc::clone(&entries);
            {
                let target = runner.cpu_parts().1.pointer(0x1005).unwrap().unwrap();
                let join = runner.cpu_parts().1.pointer(0x1008).unwrap().unwrap();
                let unbind = runner
                    .bind_inline_region(
                        target,
                        join,
                        abi,
                        move |continuation: &mut dyn GuestInlineContinuation| {
                            probe.set(probe.get() + 1);
                            if skip {
                                continuation.skip();
                            } else {
                                continuation.execute();
                            }
                            Ok(())
                        },
                        Rc::new(|| true),
                    )
                    .unwrap();
                let expected = if skip { 5 } else { 7 };
                let result = runner.invoke(&request).unwrap();
                assert_eq!(int_result(&result), expected);
                assert_eq!(result_register(&mut runner, abi), expected);
                assert_eq!(entries.get(), 1);
                assert_eq!(runner.depth(), 0);
                assert_eq!(runner.instructions_executed(), if skip { 3 } else { 4 });
                let (state, _) = runner.cpu_parts();
                assert_eq!(state.instruction_pointer, 0x1800);
                let rsp = state
                    .registers
                    .read(
                        GuestRegister::Rsp,
                        if wide { GuestIntegerWidth::B64 } else { GuestIntegerWidth::B32 },
                        false,
                    )
                    .unwrap();
                assert_eq!(rsp, 0x20000);
                unbind();
            }
            // After removal the region body always runs.
            let result = runner.invoke(&request).unwrap();
            assert_eq!(int_result(&result), 7);
            assert_eq!(entries.get(), 1);
            assert_eq!(runner.depth(), 0);
        }
    }
}

#[test]
fn inline_intercepts_only_run_when_the_region_admits() {
    for wide in [false, true] {
        let (mut runner, request, abi) = build_runner(wide);
        let entries = Rc::new(Cell::new(0u32));
        let probe = Rc::clone(&entries);
        let target = runner.cpu_parts().1.pointer(0x1005).unwrap().unwrap();
        let join = runner.cpu_parts().1.pointer(0x1008).unwrap().unwrap();
        let unbind = runner
            .bind_inline_region(
                target,
                join,
                abi,
                move |continuation: &mut dyn GuestInlineContinuation| {
                    probe.set(probe.get() + 1);
                    continuation.execute();
                    Ok(())
                },
                Rc::new(|| false),
            )
            .unwrap();
        // A rejecting admission predicate runs the body as plain guest code
        // without invoking the intercept.
        let result = runner.invoke(&request).unwrap();
        assert_eq!(int_result(&result), 7);
        assert_eq!(entries.get(), 0);
        unbind();
    }
    // Rejection path: no region bound, so the body runs inline with no
    // intercept entries and the native result stands.
    let (mut runner, request, _) = build_runner(false);
    let result = runner.invoke(&request).unwrap();
    assert_eq!(int_result(&result), 7);
    assert_eq!(runner.depth(), 0);
}

#[test]
fn inline_failures_are_sticky_and_release_the_active_frame() {
    for wide in [false, true] {
        let (mut runner, request, _) = build_runner(wide);
        let target = runner.cpu_parts().1.pointer(0x1005).unwrap().unwrap();
        let join = runner.cpu_parts().1.pointer(0x1008).unwrap().unwrap();
        let _unbind = runner
            .bind_inline_region(
                target,
                join,
                abi_for(wide),
                |continuation: &mut dyn GuestInlineContinuation| {
                    continuation.execute();
                    continuation.skip();
                    Ok(())
                },
                Rc::new(|| true),
            )
            .unwrap();
        let failure = runner.invoke(&request).unwrap_err();
        match failure {
            GuestCallFailure::Guest(error) => {
                assert!(format!("{error:?}").contains("already consumed"), "{error:?}");
            }
            other => panic!("expected guest failure, got {other:?}"),
        }
        assert_eq!(runner.depth(), 0);

        // A budget stop inside the region reports the join point and frees
        // the frame for later calls.
        let (mut runner, request, _) = build_runner(wide);
        let target = runner.cpu_parts().1.pointer(0x1005).unwrap().unwrap();
        let join = runner.cpu_parts().1.pointer(0x1008).unwrap().unwrap();
        let _unbind = runner
            .bind_inline_region(
                target,
                join,
                abi_for(wide),
                |continuation: &mut dyn GuestInlineContinuation| {
                    continuation.execute();
                    Ok(())
                },
                Rc::new(|| true),
            )
            .unwrap();
        let mut budgeted = request.clone();
        budgeted.instruction_budget = 3;
        let failure = runner.invoke(&budgeted).unwrap_err();
        match failure {
            GuestCallFailure::Stopped { stop, .. } => {
                assert!(matches!(stop, GuestExecutionStop::Budget { .. }), "{stop:?}");
            }
            other => panic!("expected budget stop, got {other:?}"),
        }
        assert_eq!(runner.depth(), 0);
        {
            let (state, _) = runner.cpu_parts();
            assert_eq!(state.instruction_pointer, 0x1008);
        }
        let recovered = runner.invoke(&request).unwrap();
        assert_eq!(int_result(&recovered), 7);
        assert_eq!(runner.depth(), 0);
    }
}
