#[path = "support/quakec_program.rs"]
mod quakec_program;
use qa_compat::quakec::{Builtins, Layout, Trap, Vm};
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
        },
    )
    .unwrap()
}
struct Reject;
impl Builtins for Reject {
    fn call(&mut self, _: &mut Vm, _: u32, _: usize) -> Result<(), Trap> {
        Err(Trap::Builtin)
    }
}

#[test]
fn vector_dot_keeps_binary32_order_and_vector_add_keeps_overlap_order() {
    use Opcode::*;
    let bytes = program(&[(MulV, 28, 31, 30), (Return, 30, 0, 0)], &[0, 1]);
    let mut vm = load(&bytes);
    for (a, b) in [
        ([1.0e20f32, 1.0, -1.0e20], [1.0, 1.0, 1.0]),
        ([-0.0f32, -0.0, -0.0], [1.0, 2.0, 3.0]),
        (
            [f32::from_bits(1), f32::from_bits(2), 0.0],
            [2.0, -1.0, 1.0],
        ),
        ([1.25f32, -2.5, 7.75], [0.1, 0.2, -0.3]),
    ] {
        vm.image.globals[28..31].copy_from_slice(&a.map(f32::to_bits));
        vm.image.globals[31..34].copy_from_slice(&b.map(f32::to_bits));
        let expected = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert_eq!(
            vm.call(&mut Reject, 1, 100, false).unwrap()[0],
            expected.to_bits()
        );
    }
    let bytes = program(&[(AddV, 28, 31, 29), (Return, 29, 0, 0)], &[0, 1]);
    let mut vm = load(&bytes);
    vm.image.globals[28..34].copy_from_slice(&[1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0].map(f32::to_bits));
    assert_eq!(
        vm.call(&mut Reject, 1, 100, false).unwrap(),
        [5.0f32, 10.0, 16.0].map(f32::to_bits)
    );
}

#[test]
fn bad_operands_load_but_only_executed_statements_trap_and_the_next_call_still_works() {
    use Opcode::*;
    let bytes = program(
        &[(Goto, 2, 0, 0), (StoreF, -1, 36, 0), (Return, 28, 0, 0)],
        &[0, 1, 2],
    );
    let mut vm = load(&bytes);
    assert_eq!(vm.image.trapped_statements, 1);
    vm.image.globals[28] = 71;
    assert_eq!(vm.call(&mut Reject, 1, 100, false), Ok([71, 0, 0]));
    assert_eq!(vm.call(&mut Reject, 2, 100, false), Err(Trap::Statement));
    assert_eq!(vm.call(&mut Reject, 1, 100, false), Ok([71, 0, 0]));
    assert_eq!(vm.call(&mut Reject, 1, 1, false), Err(Trap::Budget));
    assert_eq!(vm.call(&mut Reject, 1, 100, false), Ok([71, 0, 0]));
    let mut wrong_crc = bytes;
    wrong_crc[4..8].copy_from_slice(&0i32.to_le_bytes());
    assert!(Image::parse(&wrong_crc, Some(5927)).is_err());
    assert!(Image::parse(&wrong_crc, None).is_ok());
}

struct Nested {
    depth: usize,
}
impl Builtins for Nested {
    fn call(&mut self, vm: &mut Vm, number: u32, argc: usize) -> Result<(), Trap> {
        if number != 1 || argc != 0 || self.depth != 0 {
            return Err(Trap::Builtin);
        }
        self.depth += 1;
        let result = vm.call(self, 3, 100, false);
        self.depth -= 1;
        vm.image.globals[1] = result?[0] + 17;
        Ok(())
    }
}
#[test]
fn builtin_reentry_and_native_local_restore_are_independent_for_each_call() {
    use Opcode::*;
    let bytes = program(
        &[(Call0, 28, 0, 0), (Return, 1, 0, 0), (Return, 29, 0, 0)],
        &[0, 1, -1, 3],
    );
    let mut vm = load(&bytes);
    vm.image.globals[28] = 2;
    vm.image.globals[29] = 93;
    vm.image.globals[40] = 99;
    for _ in 0..3 {
        assert_eq!(
            vm.call(&mut Nested { depth: 0 }, 1, 100, false),
            Ok([110, 0, 0])
        );
        assert_eq!(vm.image.globals[40], 99);
    }
}

#[test]
fn entity_bytes_are_live_native_offsets_and_world_assignment_is_call_scoped() {
    use Opcode::*;
    let bytes = program(
        &[
            (Address, 30, 29, 31),
            (StorePF, 28, 31, 0),
            (LoadF, 30, 29, 36),
            (Return, 36, 0, 0),
        ],
        &[0, 1],
    );
    let mut vm = load(&bytes);
    vm.image.globals[28] = 2.0f32.to_bits();
    vm.image.globals[29] = 0;
    vm.image.globals[30] = 48;
    assert_eq!(
        vm.call(&mut Reject, 1, 100, false).unwrap()[0],
        2.0f32.to_bits()
    );
    assert_eq!(vm.entities.read_word(64).unwrap() as u32, 2.0f32.to_bits());
    vm.entities.write_word(64, 3.0f32.to_bits() as i32).unwrap();
    assert_eq!(
        vm.entities
            .read_word(vm.field_address(48, 0, 1).unwrap())
            .unwrap() as u32,
        3.0f32.to_bits()
    );
    assert_eq!(vm.field_address(1, 0, 1), Err(Trap::Memory));
    assert_eq!(vm.field_address(48, 8, 1), Err(Trap::Memory));
    vm.active = true;
    vm.image.globals[30] = 0;
    assert_eq!(vm.call(&mut Reject, 1, 100, false), Err(Trap::World));
    vm.image.globals[30] = 48;
    assert!(vm.call(&mut Reject, 1, 100, false).is_ok());
}

#[test]
fn state_uses_cached_native_fields_preserves_frame_signed_zero_and_keeps_full_function_width() {
    use Opcode::*;
    let bytes = program(&[(State, 28, 29, 0), (Return, 36, 0, 0)], &[0, 1]);
    let mut vm = load(&bytes);
    vm.image.globals[28] = (-0.0f32).to_bits();
    vm.image.globals[29] = 0x01000001;
    vm.call(&mut Reject, 1, 100, false).unwrap();
    assert_eq!(
        vm.entities.read_word(64).unwrap() as u32,
        ((3.0f64 + 0.1f64) as f32).to_bits()
    );
    assert_eq!(vm.entities.read_word(68).unwrap(), 0);
    assert_eq!(vm.entities.read_word(72).unwrap(), 0x01000001);
    assert_eq!(vm.hooks.instructions, 0);
    vm.call(&mut Reject, 1, 100, true).unwrap();
    assert_eq!(vm.hooks.instructions, 2);
    assert!(vm.hooks.take_dirty_word(vm.image.globals.len() + 16));
    assert!(!vm.hooks.take_dirty_word(vm.image.globals.len() + 16));
}
