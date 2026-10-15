#[path = "support/quakec_program.rs"]
mod quakec_program;

use qa_compat::quakec::{BuiltinCall, Builtins, Layout, Run, Step, Trap, Vm};
use qa_formats::program::quakec::{Image, Opcode};
use quakec_program::program;

fn load(bytes: &[u8]) -> Vm {
    Vm::load(
        Image::parse(bytes, Some(5927)).unwrap(),
        Layout {
            entities: 4,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap()
}
fn builtin(vm: &mut Vm, run: &mut Run) -> BuiltinCall {
    let Step::Builtin(call) = vm.advance(run).unwrap() else {
        panic!("expected an executed builtin");
    };
    call
}
fn branching() -> Vm {
    use Opcode::*;
    let mut vm = load(&program(
        &[(Call0, 28, 0, 0), (Return, 29, 0, 0), (Return, 30, 0, 0)],
        &[0, 1, -7, 3],
    ));
    vm.image.globals[28] = 2;
    vm.image.globals[29] = 71;
    vm.image.globals[30] = 93;
    vm
}
struct Reject;
impl Builtins for Reject {
    fn call(&mut self, _: &mut Vm, _: u32, _: usize) -> Result<(), Trap> {
        Err(Trap::Builtin)
    }
}

#[test]
fn owned_builtin_arguments_and_native_locals_survive_nested_stops() {
    use Opcode::*;
    let mut vm = load(&program(
        &[
            (StoreF, 28, 40, 0),
            (Call2, 29, 0, 0),
            (StoreF, 40, 30, 0),
            (Return, 30, 0, 0),
            (StoreF, 31, 40, 0),
            (Call1, 29, 0, 0),
            (Return, 40, 0, 0),
        ],
        &[0, 1, -7, 5],
    ));
    vm.statement = 777;
    vm.argc = 6;
    vm.image.globals[28] = 101;
    vm.image.globals[29] = 2;
    vm.image.globals[31] = 202;
    vm.image.globals[40] = 77;
    vm.image.globals[4..10].copy_from_slice(&[11, 12, 13, 21, 22, 23]);
    let mut outer = vm.begin_root(1, 100, true).unwrap();
    let first = builtin(&mut vm, &mut outer);
    assert_eq!(first.number, 7);
    assert_eq!(first.argc(), 2);
    assert_eq!(first.arguments(), &[[11, 12, 13], [21, 22, 23]]);
    assert_eq!((vm.statement, vm.argc), (2, 2));
    assert_eq!(vm.image.globals[40], 101);
    vm.image.globals[4..10].copy_from_slice(&[31, 32, 33, u32::MAX, u32::MAX, u32::MAX]);
    let mut child = vm.begin_callback(3, 1, false).unwrap();
    let second = builtin(&mut vm, &mut child);
    assert_eq!(second.arguments(), &[[31, 32, 33]]);
    assert_eq!((vm.statement, vm.argc), (6, 1));
    assert_eq!(vm.resume(&mut child), Ok(Step::Returned([202, 0, 0])));
    assert_eq!((vm.statement, vm.argc), (2, 2));
    assert_eq!(vm.image.globals[40], 101);
    assert_eq!(first.arguments(), &[[11, 12, 13], [21, 22, 23]]);
    assert_eq!(vm.resume(&mut outer), Ok(Step::Returned([101, 202, 0])));
    assert_eq!((vm.statement, vm.argc), (777, 6));
    assert_eq!(vm.image.globals[40], 77);
    assert_eq!(
        (vm.hooks.instructions, vm.hooks.calls, vm.hooks.stores),
        (7, 2, 3)
    );
    assert!(!vm.is_running());
}

#[test]
fn callbacks_share_remaining_budget_and_root_hook_selection() {
    for (budget, expected) in [(3, Err(Trap::Budget)), (4, Ok(Step::Returned([71, 93, 0])))] {
        let mut vm = branching();
        let mut outer = vm.begin_root(1, budget, true).unwrap();
        builtin(&mut vm, &mut outer);
        let mut child = vm.begin_callback(3, 0, false).unwrap();
        assert_eq!(vm.advance(&mut child), Ok(Step::Returned([93, 0, 0])));
        assert_eq!(vm.resume(&mut outer), expected);
        assert_eq!(vm.hooks.instructions, budget - 1);
        assert!(!vm.is_running());
        let mut fresh = vm.begin_callback(3, 2, false).unwrap();
        assert_eq!(vm.advance(&mut fresh), Ok(Step::Returned([93, 0, 0])));
        assert_eq!(vm.hooks.instructions, budget - 1);
    }
    let mut vm = branching();
    let mut outer = vm.begin_root(1, 4, false).unwrap();
    builtin(&mut vm, &mut outer);
    let mut child = vm.begin_callback(3, 100, true).unwrap();
    assert_eq!(vm.advance(&mut child), Ok(Step::Returned([93, 0, 0])));
    assert_eq!(vm.resume(&mut outer), Ok(Step::Returned([71, 93, 0])));
    assert_eq!(vm.hooks.instructions, 0);
}

#[test]
fn explicit_calls_keep_fresh_settings_and_restore_the_suspended_parent() {
    struct Fresh;
    impl Builtins for Fresh {
        fn call(&mut self, vm: &mut Vm, number: u32, argc: usize) -> Result<(), Trap> {
            assert_eq!((number, argc), (7, 0));
            let saved = (vm.statement, vm.argc);
            assert_eq!(vm.call(&mut Reject, 3, 2, true), Ok([93, 0, 0]));
            assert_eq!((vm.statement, vm.argc), saved);
            let mut canceled = vm.begin_root(3, 999, true)?;
            vm.abort(&mut canceled)?;
            assert_eq!((vm.statement, vm.argc), saved);
            Ok(())
        }
    }
    let mut vm = branching();
    vm.statement = 881;
    vm.argc = 5;
    // Two outer instructions fit this budget. The explicit nested return uses
    // its own budget, unlike begin_callback's shared-budget path above.
    assert_eq!(vm.call(&mut Fresh, 1, 3, false), Ok([71, 93, 0]));
    assert_eq!(vm.hooks.instructions, 1);
    assert_eq!((vm.statement, vm.argc), (881, 5));
    assert!(!vm.is_running());
}

#[test]
fn foreign_expired_non_top_and_wrong_phase_operations_preserve_live_runs() {
    let mut vm = branching();
    let mut other = branching();
    let mut outer = vm.begin_root(1, 100, false).unwrap();
    let mut peer = other.begin_root(1, 100, false).unwrap();
    assert_eq!(vm.resume(&mut outer), Err(Trap::Continuation));
    assert!(matches!(
        vm.begin_callback(3, 100, false),
        Err(Trap::Continuation)
    ));
    builtin(&mut vm, &mut outer);
    builtin(&mut other, &mut peer);
    assert_eq!(vm.advance(&mut outer), Err(Trap::Continuation));
    assert_eq!(other.advance(&mut outer), Err(Trap::Continuation));
    assert_eq!(other.resume(&mut outer), Err(Trap::Continuation));
    assert_eq!(other.abort(&mut outer), Err(Trap::Continuation));
    assert!(matches!(
        vm.begin_callback(999, 100, false),
        Err(Trap::Function)
    ));
    let mut child = vm.begin_callback(3, 100, false).unwrap();
    assert_eq!(vm.advance(&mut outer), Err(Trap::Continuation));
    assert_eq!(vm.resume(&mut outer), Err(Trap::Continuation));
    assert_eq!(vm.abort(&mut outer), Err(Trap::Continuation));
    assert_eq!(vm.advance(&mut child), Ok(Step::Returned([93, 0, 0])));
    assert_eq!(vm.abort(&mut child), Err(Trap::Continuation));
    assert_eq!(vm.resume(&mut outer), Ok(Step::Returned([71, 93, 0])));
    assert_eq!(other.resume(&mut peer), Ok(Step::Returned([71, 93, 0])));
    let mut replacement = vm.begin_callback(3, 2, false).unwrap();
    assert_eq!(vm.abort(&mut outer), Err(Trap::Continuation));
    assert_eq!(vm.resume(&mut child), Err(Trap::Continuation));
    assert_eq!(vm.advance(&mut replacement), Ok(Step::Returned([93, 0, 0])));
}

#[test]
fn child_fault_restores_parent_locals_and_keeps_admitted_entity_writes() {
    use Opcode::*;
    let mut vm = load(&program(
        &[
            (Call0, 28, 0, 0),
            (Return, 29, 0, 0),
            (StorePF, 31, 32, 0),
            (StoreF, -1, 40, 0),
            (Return, 30, 0, 0),
        ],
        &[0, 1, -7, 3, 5],
    ));
    vm.image.globals[28] = 2;
    vm.image.globals[29] = 71;
    vm.image.globals[31] = 0x8000_0042;
    vm.image.globals[32] = 64;
    vm.image.globals[40] = 123;
    let mut outer = vm.begin_root(1, 100, true).unwrap();
    builtin(&mut vm, &mut outer);
    let mut child = vm.begin_callback(3, 1, false).unwrap();
    assert_eq!(vm.advance(&mut child), Err(Trap::Statement));
    assert_eq!(vm.entities.read_word(64).unwrap() as u32, 0x8000_0042);
    assert_eq!(vm.image.globals[40], 123);
    assert_eq!((vm.statement, vm.argc), (1, 0));
    assert_eq!(vm.abort(&mut child), Err(Trap::Continuation));
    assert_eq!(
        vm.resume(&mut outer),
        Ok(Step::Returned([71, 0, 0x8000_0042]))
    );
    assert!(!vm.is_running());
    let mut safe = vm.begin_root(4, 100, false).unwrap();
    assert!(matches!(vm.advance(&mut safe), Ok(Step::Returned(_))));
}

#[test]
fn void_resume_keeps_raw_return_cells_and_builtin_failure_unwinds() {
    use Opcode::*;
    let mut vm = load(&program(
        &[(Call0, 28, 0, 0), (Return, 1, 0, 0)],
        &[0, 1, -7],
    ));
    vm.image.globals[28] = 2;
    let words = [(-0.0f32).to_bits(), 0x8000_0001, 0xffff_ffff];
    vm.image.globals[1..4].copy_from_slice(&words);
    let mut run = vm.begin_root(1, 100, false).unwrap();
    let call = builtin(&mut vm, &mut run);
    assert_eq!(call.argc(), 0);
    assert!(call.arguments().is_empty());
    assert_eq!(vm.resume(&mut run), Ok(Step::Returned(words)));
    vm.statement = 77;
    vm.argc = 8;
    assert_eq!(vm.call(&mut Reject, 1, 100, true), Err(Trap::Builtin));
    assert_eq!((vm.statement, vm.argc), (77, 8));
    assert!(!vm.is_running());
    let mut recovered = vm.begin_root(1, 100, false).unwrap();
    builtin(&mut vm, &mut recovered);
    vm.abort(&mut recovered).unwrap();
    assert!(!vm.is_running());
}

#[test]
fn native_return_cells_are_read_after_restoring_the_callees_locals() {
    let mut bytes = program(&[(Opcode::Return, 28, 0, 0)], &[0, 1]);
    let table = i32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
    bytes[table + 36 + 4..table + 36 + 8].copy_from_slice(&1i32.to_le_bytes());
    bytes[table + 36 + 8..table + 36 + 12].copy_from_slice(&3i32.to_le_bytes());
    let mut vm = load(&bytes);
    vm.image.globals[1..4].copy_from_slice(&[7, 8, 9]);
    vm.image.globals[28..31].copy_from_slice(&[100, 200, 300]);
    let mut run = vm.begin_root(1, 2, false).unwrap();
    assert_eq!(vm.advance(&mut run), Ok(Step::Returned([7, 8, 9])));
    assert_eq!(&vm.image.globals[1..4], &[7, 8, 9]);
}

#[test]
fn synchronous_adapter_delivers_the_owned_window_to_a_stopped_override() {
    struct Stopped;
    impl Builtins for Stopped {
        fn call(&mut self, _: &mut Vm, _: u32, _: usize) -> Result<(), Trap> {
            panic!("the synchronous adapter must use its original snapshot");
        }
        fn stopped(&mut self, vm: &mut Vm, call: &BuiltinCall) -> Result<(), Trap> {
            assert_eq!((call.number, call.argc()), (7, 1));
            assert_eq!(call.arguments(), &[[17, 0x8000_0000, 0xffff_ffff]]);
            vm.image.globals[4..7].copy_from_slice(&[99, 98, 97]);
            assert_eq!(call.arguments(), &[[17, 0x8000_0000, 0xffff_ffff]]);
            vm.image.globals[1] = call.arguments()[0][0];
            Ok(())
        }
    }
    let mut vm = load(&program(
        &[(Opcode::Call1, 28, 0, 0), (Opcode::Return, 1, 0, 0)],
        &[0, 1, -7],
    ));
    vm.image.globals[28] = 2;
    vm.image.globals[4..7].copy_from_slice(&[17, 0x8000_0000, 0xffff_ffff]);
    assert_eq!(vm.call(&mut Stopped, 1, 3, false), Ok([17, 0, 0]));
    assert_eq!(&vm.image.globals[4..7], &[99, 98, 97]);
    assert!(!vm.is_running());
}

#[test]
fn existing_native_frame_and_local_limits_reject_only_the_new_entry() {
    let mut vm = branching();
    assert_eq!(vm.activation_capacity(), 32);
    let mut runs = Vec::new();
    for _ in 0..31 {
        let mut run = vm.begin_callback(1, 100, false).unwrap();
        builtin(&mut vm, &mut run);
        runs.push(run);
    }
    assert!(matches!(vm.begin_callback(1, 100, false), Err(Trap::Stack)));
    while let Some(mut run) = runs.pop() {
        vm.abort(&mut run).unwrap();
    }
    assert!(!vm.is_running());
    assert_eq!(vm.call(&mut Reject, 3, 2, false), Ok([93, 0, 0]));

    let mut bytes = quakec_program::program_with_definitions(
        &[(Opcode::Call0, 28, 0, 0), (Opcode::Return, 29, 0, 0)],
        &[0, 1, -7],
        &[],
        &[],
        8,
        4096,
    );
    let function_table = i32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
    bytes[function_table + 36 + 8..function_table + 36 + 12]
        .copy_from_slice(&2048i32.to_le_bytes());
    let mut vm = load(&bytes);
    vm.image.globals[28] = 2;
    let mut outer = vm.begin_root(1, 100, false).unwrap();
    builtin(&mut vm, &mut outer);
    assert!(matches!(
        vm.begin_callback(1, 100, false),
        Err(Trap::Locals)
    ));
    vm.abort(&mut outer).unwrap();
    assert!(!vm.is_running());
}
