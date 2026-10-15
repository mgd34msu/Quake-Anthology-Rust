use qa_compat::qvm::{Run, Step, Syscall, SystemCalls, Trap, Vm};
use qa_formats::program::qvm::{Image, Opcode};

fn vm(instructions: &[(Opcode, i32)], memory_bytes: usize) -> Vm {
    let mut code = Vec::new();
    for &(opcode, operand) in instructions {
        code.push(opcode as u8);
        match opcode.operand_bytes() {
            1 => code.push(operand as u8),
            4 => code.extend(operand.to_le_bytes()),
            _ => {}
        }
    }
    code.resize(code.len().next_multiple_of(4), 0);
    let mut bytes = Vec::new();
    for word in [
        0x1272_1444,
        instructions.len() as i32,
        32,
        code.len() as i32,
        32 + code.len() as i32,
        64,
        0,
        memory_bytes as i32 - 64,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(code);
    bytes.resize(bytes.len() + 64, 0);
    Vm::load(Image::parse(&bytes).unwrap()).unwrap()
}

fn syscall(vm: &mut Vm, run: &mut Run) -> Syscall {
    let Step::Syscall(call) = vm.advance(run).unwrap() else {
        panic!("expected an executed syscall boundary");
    };
    call
}

fn branching_vm() -> Vm {
    use Opcode::*;
    vm(
        &[
            (Enter, 64),
            (Local, 72),
            (Load4, 0),
            (Const, 9),
            (Eq, 10),
            (Const, 17),
            (Arg, 8),
            (Const, -5),
            (Call, 0),
            (Leave, 64),
            (Const, 93),
            (Leave, 64),
        ],
        65536,
    )
}

fn nested_arguments() -> [i32; 10] {
    let mut arguments = [0; 10];
    arguments[0] = 9;
    arguments
}

#[test]
fn pause_resume_owns_operands_and_one_raw_argument_snapshot() {
    use Opcode::*;
    let mut vm = vm(
        &[
            (Enter, 64),
            (Const, 500),
            (Local, 72),
            (Load4, 0),
            (Arg, 8),
            (Const, -5),
            (Call, 0),
            (Add, 0),
            (Const, -6),
            (Call, 0),
            (Add, 0),
            (Leave, 64),
        ],
        65536,
    );
    let mut arguments = [0; 10];
    arguments[0] = 17;
    let mut run = vm.begin_root(arguments, 24, true).unwrap();
    let first = syscall(&mut vm, &mut run);
    assert_eq!(first.number, 4);
    assert_eq!(first.arguments().len(), 16);
    assert_eq!(&first.arguments()[..2], &[4, 17]);
    assert_eq!(vm.hooks.instructions, 7);
    let native_stack = vm.memory.len() - 48 - 64;
    vm.memory.write_word(native_stack as u64 + 8, 999).unwrap();
    assert_eq!(first.arguments()[1], 17);
    let mut callback_arguments = [0; 10];
    callback_arguments[0] = 41;
    let mut callback = vm.begin_callback(callback_arguments, 1, false).unwrap();
    assert_eq!(syscall(&mut vm, &mut callback).arguments()[1], 41);
    assert!(matches!(vm.resume(&mut callback, 30), Ok(Step::Syscall(_))));
    assert_eq!(vm.resume(&mut callback, 40), Ok(Step::Returned(570)));
    assert_eq!(first.arguments()[1], 17);
    let Step::Syscall(second) = vm.resume(&mut run, 3).unwrap() else {
        panic!("the second import must suspend separately");
    };
    assert_eq!(second.number, 5);
    assert_eq!(second.arguments()[1], 999);
    assert_eq!(vm.resume(&mut run, 4), Ok(Step::Returned(507)));
    assert_eq!((vm.hooks.instructions, vm.hooks.calls), (24, 4));
    assert_eq!(vm.abort(&mut run), Err(Trap::Continuation));
}

#[test]
fn callbacks_share_active_budget_and_hooks_without_replacing_outer_operands() {
    for budget in [16, 17] {
        let mut vm = branching_vm();
        let mut outer = vm.begin_root([0; 10], budget, true).unwrap();
        assert_eq!(syscall(&mut vm, &mut outer).arguments()[1], 17);
        let mut callback = vm.begin_callback(nested_arguments(), 1, false).unwrap();
        assert_eq!(vm.advance(&mut callback), Ok(Step::Returned(93)));
        assert_eq!(vm.hooks.instructions, 16);
        assert_eq!(
            vm.resume(&mut outer, 93),
            if budget == 17 {
                Ok(Step::Returned(93))
            } else {
                Err(Trap::Budget)
            }
        );
        assert_eq!(vm.hooks.instructions, budget);
        let mut fresh = vm.begin_callback(nested_arguments(), 7, false).unwrap();
        assert_eq!(vm.advance(&mut fresh), Ok(Step::Returned(93)));
        assert_eq!(vm.hooks.instructions, budget);
    }
}

#[test]
fn independent_roots_restore_suspended_budget_and_hooks_after_return_and_abort() {
    let mut vm = branching_vm();
    let mut outer = vm.begin_root([0; 10], 10, false).unwrap();
    syscall(&mut vm, &mut outer);
    let mut inner = vm.begin_root(nested_arguments(), 7, true).unwrap();
    assert_eq!(vm.advance(&mut inner), Ok(Step::Returned(93)));
    assert_eq!(vm.hooks.instructions, 7);
    let mut canceled = vm.begin_root([0; 10], 99, true).unwrap();
    vm.abort(&mut canceled).unwrap();
    assert_eq!(vm.resume(&mut outer, 93), Ok(Step::Returned(93)));
    assert_eq!(vm.hooks.instructions, 7);
}

#[test]
fn child_opcode_fault_restores_the_suspended_parent_without_erasing_guest_writes() {
    use Opcode::*;
    let mut vm = vm(
        &[
            (Enter, 64),
            (Local, 72),
            (Load4, 0),
            (Const, 9),
            (Eq, 10),
            (Const, 17),
            (Arg, 8),
            (Const, -5),
            (Call, 0),
            (Leave, 64),
            (Const, 0),
            (Const, 123),
            (Store4, 0),
            (Const, 1),
            (Const, 0),
            (DivI, 0),
            (Leave, 64),
        ],
        65536,
    );
    let mut outer = vm.begin_root([0; 10], 100, false).unwrap();
    syscall(&mut vm, &mut outer);
    let mut callback = vm.begin_callback(nested_arguments(), 1, true).unwrap();
    assert_eq!(vm.advance(&mut callback), Err(Trap::Division));
    assert_eq!(vm.memory.read_word(0).unwrap(), 123);
    assert_eq!(vm.abort(&mut callback), Err(Trap::Continuation));
    assert_eq!(vm.resume(&mut outer, 77), Ok(Step::Returned(77)));
}

#[test]
fn wrong_owner_wrong_order_and_waiting_operations_keep_the_valid_run() {
    let mut vm = branching_vm();
    let mut other = branching_vm();
    let mut outer = vm.begin_root([0; 10], 100, false).unwrap();
    assert_eq!(vm.resume(&mut outer, 0), Err(Trap::Continuation));
    assert!(matches!(
        vm.begin_callback(nested_arguments(), 7, false),
        Err(Trap::Continuation)
    ));
    syscall(&mut vm, &mut outer);
    assert_eq!(vm.advance(&mut outer), Err(Trap::Continuation));
    assert_eq!(other.advance(&mut outer), Err(Trap::Continuation));
    assert_eq!(other.resume(&mut outer, 0), Err(Trap::Continuation));
    assert_eq!(other.abort(&mut outer), Err(Trap::Continuation));
    let mut child = vm.begin_callback(nested_arguments(), 7, false).unwrap();
    assert_eq!(vm.abort(&mut outer), Err(Trap::Continuation));
    assert_eq!(vm.resume(&mut outer, 0), Err(Trap::Continuation));
    assert_eq!(vm.advance(&mut child), Ok(Step::Returned(93)));
    vm.abort(&mut outer).unwrap();
    let mut replacement = vm.begin_root(nested_arguments(), 7, false).unwrap();
    assert_eq!(vm.abort(&mut outer), Err(Trap::Continuation));
    assert_eq!(vm.resume(&mut child, 0), Err(Trap::Continuation));
    assert_eq!(vm.advance(&mut replacement), Ok(Step::Returned(93)));
    let mut other_run = other.begin_root(nested_arguments(), 7, false).unwrap();
    assert_eq!(other.advance(&mut other_run), Ok(Step::Returned(93)));
}

#[test]
fn resume_rereads_the_native_byte_pc_and_faults_finalize_only_that_activation() {
    use Opcode::*;
    let mut vm = vm(
        &[
            (Enter, 64),
            (Const, -1),
            (Call, 0),
            (Const, 100),
            (Add, 0),
            (Leave, 64),
            (Const, 200),
            (Add, 0),
            (Leave, 64),
        ],
        65536,
    );
    let native_stack = vm.memory.len() - 48 - 64;
    let mut run = vm.begin_root([0; 10], 20, false).unwrap();
    syscall(&mut vm, &mut run);
    let target = vm.image.instructions[6].byte_pc as i32;
    vm.memory.write_word(native_stack as u64, target).unwrap();
    assert_eq!(vm.resume(&mut run, 7), Ok(Step::Returned(207)));
    let mut broken = vm.begin_root([0; 10], 20, false).unwrap();
    syscall(&mut vm, &mut broken);
    vm.memory.write_word(native_stack as u64, i32::MAX).unwrap();
    assert_eq!(vm.resume(&mut broken, 7), Err(Trap::Target));
    assert_eq!(vm.abort(&mut broken), Err(Trap::Continuation));
    let mut recovered = vm.begin_callback([0; 10], 20, false).unwrap();
    syscall(&mut vm, &mut recovered);
    assert_eq!(vm.resume(&mut recovered, 7), Ok(Step::Returned(107)));
}

#[test]
fn executed_upward_leave_cannot_escape_its_activation_and_recovery_is_balanced() {
    use Opcode::*;
    let mut vm = vm(
        &[
            (Enter, 16),
            (Local, 24),
            (Load4, 0),
            (Const, 9),
            (Eq, 7),
            (Const, 1),
            (Leave, 20),
            (Const, 83),
            (Leave, 16),
        ],
        65536,
    );
    let mut bad = vm.begin_root([0; 10], 100, false).unwrap();
    assert_eq!(vm.advance(&mut bad), Err(Trap::ProgramStack));
    let mut good = vm.begin_root(nested_arguments(), 7, false).unwrap();
    assert_eq!(vm.advance(&mut good), Ok(Step::Returned(83)));
    assert_eq!(vm.advance(&mut bad), Err(Trap::Continuation));
}

#[test]
fn deepest_scalar_import_remains_valid_but_callback_needs_its_own_entry_frame() {
    use Opcode::*;
    let mut vm = vm(
        &[
            (Enter, 65536 - 48),
            (Const, -1),
            (Call, 0),
            (Leave, 65536 - 48),
        ],
        131072,
    );
    for _ in 0..2 {
        let mut root = vm.begin_root([0; 10], 4, false).unwrap();
        assert_eq!(syscall(&mut vm, &mut root).number, 0);
        assert!(matches!(
            vm.begin_callback([0; 10], 100, true),
            Err(Trap::ProgramStack)
        ));
        assert_eq!(vm.resume(&mut root, 99), Ok(Step::Returned(99)));
    }
}

struct RejectImport {
    reject_entry: bool,
}

struct EntryWrites {
    entries: u32,
}
impl SystemCalls for EntryWrites {
    fn begin_call(&mut self, vm: &mut Vm) -> Result<(), Trap> {
        self.entries += 1;
        vm.memory.write_word(0, 456)?;
        Ok(())
    }
    fn call(&mut self, _: &mut Vm, _: u32, _: &[i32]) -> Result<i32, Trap> {
        Ok(0)
    }
}

#[test]
fn rejected_unsuspended_entry_does_not_run_projection_hooks_or_mutate_guest_memory() {
    let mut vm = branching_vm();
    let mut outer = vm.begin_root(nested_arguments(), 7, true).unwrap();
    vm.memory.write_word(0, 123).unwrap();
    let mut calls = EntryWrites { entries: 0 };
    assert_eq!(
        vm.call(&mut calls, nested_arguments(), 99, false),
        Err(Trap::Continuation)
    );
    assert_eq!(calls.entries, 0);
    assert_eq!(vm.memory.read_word(0).unwrap(), 123);
    assert_eq!(vm.advance(&mut outer), Ok(Step::Returned(93)));
    assert_eq!(vm.hooks.instructions, 7);
    assert_eq!(vm.call(&mut calls, nested_arguments(), 7, false), Ok(93));
    assert_eq!(calls.entries, 1);
    assert_eq!(vm.memory.read_word(0).unwrap(), 456);
    assert_eq!(vm.hooks.instructions, 7);
}
impl SystemCalls for RejectImport {
    fn begin_call(&mut self, _: &mut Vm) -> Result<(), Trap> {
        if self.reject_entry {
            Err(Trap::Syscall)
        } else {
            Ok(())
        }
    }
    fn call(&mut self, _: &mut Vm, _: u32, _: &[i32]) -> Result<i32, Trap> {
        Err(Trap::Syscall)
    }
}

#[test]
fn synchronous_wrapper_rejections_restore_the_same_runner_for_later_callbacks() {
    let mut vm = branching_vm();
    let mut calls = RejectImport { reject_entry: true };
    assert_eq!(vm.call(&mut calls, [0; 10], 99, true), Err(Trap::Syscall));
    assert_eq!(vm.hooks.instructions, 0);
    calls.reject_entry = false;
    assert_eq!(vm.call(&mut calls, [0; 10], 99, false), Err(Trap::Syscall));
    let mut recovered = vm.begin_callback(nested_arguments(), 7, true).unwrap();
    assert_eq!(vm.advance(&mut recovered), Ok(Step::Returned(93)));
    assert_eq!(vm.hooks.instructions, 7);
}
