use qa_compat::qvm::{SystemCalls, Trap, Vm};
use qa_formats::program::qvm::{Image, Opcode};

fn image(instructions: &[(Opcode, i32)]) -> Vec<u8> {
    let mut code = Vec::new();
    for &(op, argument) in instructions {
        code.push(op as u8);
        match op.operand_bytes() {
            1 => code.push(argument as u8),
            4 => code.extend(argument.to_le_bytes()),
            _ => {}
        }
    }
    code.resize((code.len() + 3) & !3, 0);
    let mut bytes = Vec::new();
    for word in [
        0x12721444,
        instructions.len() as i32,
        32,
        code.len() as i32,
        32 + code.len() as i32,
        64,
        0,
        65536 - 64,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(code);
    bytes.resize(bytes.len() + 64, 0);
    bytes
}
struct Reject;
impl SystemCalls for Reject {
    fn call(&mut self, _: &mut Vm, _: u32, _: &[i32]) -> Result<i32, Trap> {
        Err(Trap::Syscall)
    }
}

#[test]
fn padding_return_pcs_hooks_and_native_interpreted_complement_are_preserved() {
    use Opcode::*;
    let bytes = image(&[
        (Enter, 16),
        (Const, 10),
        (Const, 5),
        (Bcom, 0),
        (Pop, 0),
        (Leave, 16),
    ]);
    let mut vm = Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    assert_eq!(vm.call(&mut Reject, [0; 10], 100, false), Ok(!5));
    assert_eq!(vm.hooks.instructions, 0);
    assert_eq!(vm.call(&mut Reject, [0; 10], 100, true), Ok(!5));
    assert_eq!(vm.hooks.instructions, 6);
    let bytes = image(&[
        (Enter, 16),
        (Const, 4),
        (Call, 0),
        (Leave, 16),
        (Enter, 16),
        (Const, 97),
        (Leave, 16),
    ]);
    let mut vm = Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    assert_eq!(vm.call(&mut Reject, [0; 10], 100, false), Ok(97));
}

#[test]
fn static_code_errors_are_load_errors_and_runtime_faults_abort_only_that_call() {
    use Opcode::*;
    let mut bytes = image(&[(Enter, 16), (Const, 1), (Const, 2), (Eq, 100), (Leave, 16)]);
    assert!(Image::parse(&bytes).is_err());
    bytes[32] = 200;
    assert!(Image::parse(&bytes).is_err());
    let bytes = image(&[
        (Enter, 16),
        (Local, 24),
        (Load4, 0),
        (Const, 9),
        (Eq, 8),
        (Const, 1),
        (Const, 0),
        (DivI, 0),
        (Const, 83),
        (Leave, 16),
    ]);
    let mut vm = Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    assert_eq!(
        vm.call(&mut Reject, [0; 10], 100, false),
        Err(Trap::Division)
    );
    let mut args = [0; 10];
    args[0] = 9;
    assert_eq!(vm.call(&mut Reject, args, 100, false), Ok(83));
    assert_eq!(vm.call(&mut Reject, args, 0, false), Err(Trap::Budget));
    assert_eq!(vm.call(&mut Reject, args, 100, false), Ok(83));
}

struct Reentry {
    depth: usize,
}
impl SystemCalls for Reentry {
    fn call(&mut self, vm: &mut Vm, number: u32, args: &[i32]) -> Result<i32, Trap> {
        if self.depth != 0 || number != 4 || args.get(1) != Some(&17) {
            return Err(Trap::Syscall);
        }
        self.depth += 1;
        let mut nested = [0; 10];
        nested[0] = 9;
        let result = vm.call(self, nested, 100, false);
        self.depth -= 1;
        result
    }
}
#[test]
fn syscall_reentry_uses_private_operand_stack_and_restores_parent_native_stack() {
    use Opcode::*;
    let bytes = image(&[
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
    ]);
    let mut vm = Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    let mut host = Reentry { depth: 0 };
    for _ in 0..3 {
        assert_eq!(vm.call(&mut host, [0; 10], 100, false), Ok(93));
    }
}

#[test]
fn only_the_hook_loop_marks_load_sized_dirty_words() {
    use Opcode::*;
    let bytes = image(&[
        (Enter, 16),
        (Const, 3),
        (Const, 0x1234),
        (Store2, 0),
        (Const, 0),
        (Leave, 16),
    ]);
    let mut vm = Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    assert_eq!(vm.call(&mut Reject, [0; 10], 100, false), Ok(0));
    assert!(!vm.hooks.take_dirty_word(0));
    assert_eq!(vm.call(&mut Reject, [0; 10], 100, true), Ok(0));
    assert_eq!(vm.hooks.stores, 1);
    assert!(vm.hooks.take_dirty_word(0));
    assert!(!vm.hooks.take_dirty_word(0));
    assert!(!vm.hooks.take_dirty_word(usize::MAX));
}
