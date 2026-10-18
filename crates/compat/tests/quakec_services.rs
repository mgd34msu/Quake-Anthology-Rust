#[path = "support/quakec_program.rs"]
mod quakec_program;

use qa_compat::{
    abi::{QUAKEC, QuakeCCalls, UnknownCalls},
    quakec::{Builtins, Layout, Trap, Vm},
    services::{
        CallContext, CallError, ENGINE_CALLS, EngineServices, ModelLoader, ModelLookup,
        ServiceStorage, SignonCursor,
    },
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{names::NameTable, primitives::*, sys_events::EventTime};
use qa_formats::program::quakec::{Image, Opcode};
use qa_world::{
    area::LinkFlags,
    collision::{CollisionStore, brushes::BrushTree, surfaces::SurfaceTable},
    entities::AllocationPolicy,
};

const FIELDS: &[(u16, u16, &[u8])] = &[
    (3, 0, b"origin"),
    (3, 3, b"velocity"),
    (3, 6, b"angles"),
    (3, 9, b"mins"),
    (3, 12, b"maxs"),
    (3, 15, b"size"),
    (3, 18, b"absmin"),
    (3, 21, b"absmax"),
    (2, 24, b"solid"),
    (2, 25, b"flags"),
    (4, 26, b"owner"),
    (1, 27, b"model"),
    (2, 28, b"modelindex"),
    (2, 29, b"frame"),
    (2, 30, b"skin"),
    (2, 31, b"effects"),
    (2, 32, b"takedamage"),
    (2, 33, b"colormap"),
    (2, 34, b"nextthink"),
    (6, 35, b"think"),
    (4, 36, b"groundentity"),
];
const GLOBALS: &[(u16, u16, &[u8])] = &[
    (4, 34, b"self"),
    (2, 35, b"time"),
    (2, 60, b"trace_allsolid"),
    (2, 61, b"trace_startsolid"),
    (2, 62, b"trace_fraction"),
    (2, 63, b"trace_inwater"),
    (2, 64, b"trace_inopen"),
    (2, 65, b"trace_plane_dist"),
    (3, 66, b"trace_endpos"),
    (3, 69, b"trace_plane_normal"),
    (4, 72, b"trace_ent"),
];
fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|value| value.0.map(f32::to_bits))
}
fn vm(capacity: usize) -> Vm {
    vm_with_definitions(capacity, GLOBALS, FIELDS)
}
fn vm_with_definitions(
    capacity: usize,
    globals: &[(u16, u16, &[u8])],
    fields: &[(u16, u16, &[u8])],
) -> Vm {
    let bytes = quakec_program::program_with_definitions(
        &[
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePV, 28, 31, 0),
            (Opcode::Call4, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePF, 28, 31, 0),
            (Opcode::Call4, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::State, 28, 29, 0),
            (Opcode::Call4, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePF, 28, 31, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePV, 28, 31, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::State, 28, 29, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePV, 28, 31, 0),
            (Opcode::Call1, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
        ],
        &[0, 1, 2, 5, 8, -16, 11, 13, 15, 17, -41],
        globals,
        fields,
        fields
            .iter()
            .map(|(kind, offset, _)| usize::from(*offset) + if *kind == 3 { 3 } else { 1 })
            .max()
            .unwrap_or(37)
            .max(37),
        96,
    );
    Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: capacity,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap()
}
fn parameters(vm: &mut Vm, parameters: &[&[u32]]) {
    for (index, words) in parameters.iter().enumerate() {
        vm.image.globals[4 + index * 3..4 + index * 3 + words.len()].copy_from_slice(words);
    }
}
fn write(vm: &mut Vm, value: u32, name: &[u8], words: &[u32]) {
    let field = vm.image.field(name).unwrap() as u32;
    let at = vm.field_address(value, field, words.len()).unwrap();
    for (index, word) in words.iter().enumerate() {
        vm.entities
            .write_word(at + index as u64 * 4, *word as i32)
            .unwrap();
    }
}
fn read(vm: &Vm, value: u32, name: &[u8], width: usize) -> Vec<u32> {
    let at = vm
        .field_address(value, vm.image.field(name).unwrap() as u32, width)
        .unwrap();
    (0..width)
        .map(|index| vm.entities.read_word(at + index as u64 * 4).unwrap() as u32)
        .collect()
}
fn store_and_trace(
    calls: &mut QuakeCCalls<'_, '_>,
    vm: &mut Vm,
    value: u32,
    name: &[u8],
    words: &[u32],
) -> Result<[u32; 3], Trap> {
    vm.image.globals[28..28 + words.len()].copy_from_slice(words);
    vm.image.globals[31] = vm
        .field_address(value, vm.image.field(name).unwrap() as u32, words.len())
        .unwrap() as u32;
    vm.image.globals[32] = 5;
    vm.call(calls, if words.len() == 3 { 2 } else { 3 }, 100, false)
}
fn entity(calls: &QuakeCCalls<'_, '_>, vm: &Vm, value: u32) -> EntityId {
    let native = NativeEntity {
        module: calls.context.module,
        slot: vm.entity_number(value).unwrap() as i32,
    };
    calls
        .services
        .server
        .entities
        .active()
        .find(|id| {
            calls.services.server.entities.columns.native_entity[id.slot as usize] == Some(native)
        })
        .unwrap()
}
fn with_services(rules: RuleSetId, run: impl FnOnce(&mut QuakeCCalls<'_, '_>)) {
    with_model_services(rules, None, run);
}

fn support_vm(
    builtin: i32,
    operations: &[(Opcode, i16, i16, i16)],
    fields: &[(u16, u16, &[u8])],
) -> Vm {
    let bytes = quakec_program::program_with_definitions(
        operations,
        &[0, 1, -builtin, operations.len() as i32],
        &[(4, 34, b"self")],
        fields,
        fields
            .iter()
            .map(|(kind, offset, _)| usize::from(*offset) + if *kind == 3 { 3 } else { 1 })
            .max()
            .unwrap_or(1),
        96,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    vm.image.globals[32] = 2;
    vm.image.globals[34] = 0;
    vm
}

#[test]
fn owned_builtin_import_uses_stopped_arguments_and_native_return_widths() {
    use qa_compat::{abi::QcImportStep, quakec::Step};
    let tail = [0x1234_5678, 0x7fc2_3456];
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(source, |calls| {
            for (number, input, expected) in [
                (
                    12,
                    [3.0f32, 4.0, 12.0],
                    [13.0f32.to_bits(), tail[0], tail[1]],
                ),
                (9, [0.0, 2.0, 0.0], [0, 1.0f32.to_bits(), 0]),
                (13, [0.0, 1.0, 0.0], [90.0f32.to_bits(), tail[0], tail[1]]),
            ] {
                let mut vm = support_vm(
                    number,
                    &[(Opcode::Call1, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                    FIELDS,
                );
                let raw = input.map(f32::to_bits);
                parameters(&mut vm, &[&raw]);
                vm.image.globals[1..4].copy_from_slice(&[0xdead_beef, tail[0], tail[1]]);
                let mut run = vm.begin_root(1, 64, false).unwrap();
                let Step::Builtin(builtin) = vm
                    .advance(&mut run, &qa_compat::quakec::EntityAccess::native_only())
                    .unwrap()
                else {
                    panic!("expected an executed builtin");
                };
                vm.image.globals[4..7].fill(99.0f32.to_bits());
                assert!(matches!(
                    calls.import(&mut vm, &builtin).unwrap(),
                    QcImportStep::Returned
                ));
                assert_eq!(builtin.arguments(), &[raw]);
                assert_eq!(
                    vm.resume(&mut run, &qa_compat::quakec::EntityAccess::native_only()),
                    Ok(Step::Returned(expected))
                );
                assert!(!vm.is_running());
            }
        });
    }
}

#[test]
fn changeyaw_executes_native_ties_clamps_and_only_the_yaw_scalar_without_body_requirements() {
    let fields = [
        (3, 0, b"angles".as_slice()),
        (2, 3, b"ideal_yaw"),
        (2, 4, b"yaw_speed"),
    ];
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(source, |calls| {
            for (current, ideal, speed, expected) in [
                (0.0f32, 180.0f32, 90.0f32, 270.0f32),
                (180.0, 0.0, 90.0, 270.0),
                (0.0, 90.0, 30.0, 29.998_169),
                (0.0, 90.0, -10.0, 350.002_44),
                (-0.0, 0.0, f32::NAN, -0.0),
            ] {
                let mut vm = support_vm(
                    49,
                    &[(Opcode::Call0, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                    &fields,
                );
                let native = vm.stride as u32 * 3;
                vm.image.globals[34] = native;
                write(
                    &mut vm,
                    native,
                    b"angles",
                    &[0x7fc1_2345, current.to_bits(), 0x8000_0000],
                );
                write(&mut vm, native, b"ideal_yaw", &[ideal.to_bits()]);
                write(&mut vm, native, b"yaw_speed", &[speed.to_bits()]);
                vm.image.globals[1..4].copy_from_slice(&[0xdead_beef, 0x1234_5678, 0x7fc2_3456]);
                let before = vm.entities.read(0, vm.entities.len()).unwrap().to_vec();
                assert_eq!(
                    vm.call(calls, 1, 32, false),
                    Ok([0xdead_beef, 0x1234_5678, 0x7fc2_3456])
                );
                let yaw = vm.field_address(native, 1, 1).unwrap() as usize;
                let mut expected_bytes = before;
                expected_bytes[yaw..yaw + 4].copy_from_slice(&expected.to_bits().to_le_bytes());
                assert_eq!(
                    vm.entities.read(0, vm.entities.len()).unwrap(),
                    expected_bytes
                );
                assert!(solid_order(calls).is_empty());
            }
        });
    }
}

#[test]
fn changeyaw_reads_overlapping_fields_before_store_and_rejects_only_executed_bad_operands() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(source, |calls| {
            let fields = [
                (3, 0, b"angles".as_slice()),
                (2, 2, b"ideal_yaw"), // aliases roll
                (2, 1, b"yaw_speed"), // aliases current yaw
            ];
            let mut vm = support_vm(
                49,
                &[(Opcode::Call0, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                &fields,
            );
            write(
                &mut vm,
                0,
                b"angles",
                &[11.0f32.to_bits(), 90.0f32.to_bits(), 180.0f32.to_bits()],
            );
            vm.call(calls, 1, 32, false).unwrap();
            assert_eq!(
                read(&vm, 0, b"angles", 3),
                [11.0f32.to_bits(), 180.0f32.to_bits(), 180.0f32.to_bits()]
            );
            vm.image.globals[34] = 1; // raw byte offsets must name a whole edict
            assert_eq!(vm.call(calls, 1, 32, false), Err(Trap::Builtin));
            let mut missing = support_vm(
                49,
                &[(Opcode::Call0, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                &fields[..2],
            );
            missing.image.globals[1] = 73.0f32.to_bits();
            assert_eq!(
                missing.call(calls, 3, 32, false).unwrap()[0],
                73.0f32.to_bits()
            );
            assert_eq!(missing.call(calls, 1, 32, false), Err(Trap::Builtin));
            assert_eq!(missing.image.globals[1], 73.0f32.to_bits());
            // Bad argc is checked before even the missing entity fields execute.
            let mut argc = support_vm(
                49,
                &[(Opcode::Call1, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                &[],
            );
            assert_eq!(argc.call(calls, 1, 32, false), Err(Trap::Builtin));
        });
    }
}

#[test]
fn changeyaw_reaches_the_existing_common_entity_publication_without_relinking() {
    with_services(RuleSetId::Quake, |calls| {
        let mut fields = FIELDS.to_vec();
        fields.extend([(2, 37, b"ideal_yaw".as_slice()), (2, 38, b"yaw_speed")]);
        let raw = support_vm(
            49,
            &[(Opcode::Call0, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
            &fields,
        );
        let mut vm = seed_vm(calls, raw);
        calls.call(&mut vm, 14, 0).unwrap();
        let native = vm.image.globals[1];
        let id = entity(calls, &vm, native);
        vm.image.globals[34] = native;
        write(&mut vm, native, b"ideal_yaw", &[90.0f32.to_bits()]);
        write(&mut vm, native, b"yaw_speed", &[90.0f32.to_bits()]);
        vm.call(calls, 1, 32, false).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.angles[id.slot as usize],
            Vec3::default()
        );
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.angles[id.slot as usize],
            Vec3([0.0, 90.0, 0.0])
        );
        assert!(solid_order(calls).is_empty());
    });
}

#[test]
fn checkbottom_requires_only_its_executed_pose_fields_and_fixed_entity_argument() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -64.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    let fields = [
        (3, 0, b"origin".as_slice()),
        (3, 3, b"mins"),
        (3, 6, b"maxs"),
    ];
    for complete in [true, false] {
        with_collision_services(RuleSetId::Quake, None, &[floor], |calls| {
            let raw = support_vm(
                40,
                &[(Opcode::Call1, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                if complete { &fields } else { &fields[..2] },
            );
            let mut vm = seed_vm(calls, raw);
            vm.image.globals[1] = 73.0f32.to_bits();
            assert_eq!(vm.call(calls, 3, 32, false).unwrap()[0], 73.0f32.to_bits());
            parameters(&mut vm, &[&[0]]);
            if complete {
                write(&mut vm, 0, b"mins", &[0; 3]);
                write(&mut vm, 0, b"maxs", &[1.0f32.to_bits(); 3]);
                assert_eq!(vm.call(calls, 1, 32, false).unwrap()[0], 1.0f32.to_bits());
                // Above the floor, the slow path traces only world geometry.
                // Missing owner fields remain dormant without a linked target.
                write(&mut vm, 0, b"origin", &[0, 0, 4.0f32.to_bits()]);
                assert_eq!(vm.call(calls, 1, 32, false).unwrap()[0], 1.0f32.to_bits());
            } else {
                assert_eq!(vm.call(calls, 1, 32, false), Err(Trap::Builtin));
                assert_eq!(vm.image.globals[1], 73.0f32.to_bits());
            }
        });
    }
}

#[test]
fn checkbottom_runs_real_world_support_probes_without_mutating_native_or_link_state() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -64.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for z in [0.0f32, 4.0] {
            // first is the contents fast path; second requires traces
            with_collision_services(source, None, &[floor], |calls| {
                let raw = support_vm(
                    40,
                    &[(Opcode::Call1, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                    FIELDS,
                );
                let mut vm = seed_vm(calls, raw);
                let native = vm.stride as u32 * 5; // valid unlinked native backing
                write(&mut vm, native, b"origin", &[0, 0, z.to_bits()]);
                write(
                    &mut vm,
                    native,
                    b"mins",
                    &[(-2.0f32).to_bits(), (-2.0f32).to_bits(), 0],
                );
                write(
                    &mut vm,
                    native,
                    b"maxs",
                    &[2.0f32.to_bits(), 2.0f32.to_bits(), 4.0f32.to_bits()],
                );
                parameters(&mut vm, &[&[native]]);
                vm.image.globals[1..4].copy_from_slice(&[0xdead_beef, 0x1234_5678, 0x7fc2_3456]);
                vm.image.globals[60..73].fill(0xabcdef01);
                let raw_before = vm.entities.read(0, vm.entities.len()).unwrap().to_vec();
                let entities_before = calls.services.server.entities.active().count();
                assert_eq!(
                    vm.call(calls, 1, 64, false),
                    Ok([1.0f32.to_bits(), 0x1234_5678, 0x7fc2_3456])
                );
                assert_eq!(vm.entities.read(0, vm.entities.len()).unwrap(), raw_before);
                assert_eq!(&vm.image.globals[60..73], &[0xabcdef01; 13]);
                assert_eq!(
                    calls.services.server.entities.active().count(),
                    entities_before
                );
                assert!(solid_order(calls).is_empty());
                for bad in [1, (vm.stride * 64) as u32] {
                    parameters(&mut vm, &[&[bad]]);
                    vm.image.globals[1] = 73.0f32.to_bits();
                    assert_eq!(vm.call(calls, 1, 64, false), Err(Trap::Builtin));
                    assert_eq!(vm.image.globals[1], 73.0f32.to_bits());
                }
            });
        }
    }
}

#[test]
fn checkbottom_detects_a_native_step_ledge_and_missing_floor_through_shared_collision() {
    for lower in [-18.0f32, -18.125] {
        let floors = [
            Bounds {
                mins: Vec3([-100.0, -100.0, -64.0]),
                maxs: Vec3([0.0, 100.0, 0.0]),
            },
            Bounds {
                mins: Vec3([0.0, -100.0, -64.0]),
                maxs: Vec3([100.0, 100.0, lower]),
            },
        ];
        with_collision_services(RuleSetId::Quake, None, &floors, |calls| {
            let raw = support_vm(
                40,
                &[(Opcode::Call1, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                FIELDS,
            );
            let mut vm = seed_vm(calls, raw);
            write(&mut vm, 0, b"origin", &[0, 0, 4.0f32.to_bits()]);
            write(
                &mut vm,
                0,
                b"mins",
                &[(-2.0f32).to_bits(), (-2.0f32).to_bits(), 0],
            );
            write(
                &mut vm,
                0,
                b"maxs",
                &[2.0f32.to_bits(), 2.0f32.to_bits(), 4.0f32.to_bits()],
            );
            parameters(&mut vm, &[&[0]]); // world0 is an admissible raw argument
            let expected = if lower == -18.0 { 1.0f32 } else { 0.0f32 };
            assert_eq!(vm.call(calls, 1, 64, false).unwrap()[0], expected.to_bits());
        });
    }
    with_services(RuleSetId::Quake, |calls| {
        let raw = support_vm(
            40,
            &[(Opcode::Call1, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
            FIELDS,
        );
        let mut vm = seed_vm(calls, raw);
        parameters(&mut vm, &[&[0]]);
        assert_eq!(vm.call(calls, 1, 64, false).unwrap()[0], 0.0f32.to_bits());
    });
}

#[test]
fn checkbottom_keeps_foreign_trace_selection_and_does_not_flush_unrelated_dirty_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let raw = support_vm(
            40,
            &[
                (Opcode::StorePV, 28, 31, 0),
                (Opcode::Call1, 32, 0, 0),
                (Opcode::Return, 1, 0, 0),
            ],
            FIELDS,
        );
        let mut vm = seed_vm(calls, raw);
        calls.call(&mut vm, 14, 0).unwrap();
        let obstacle = vm.image.globals[1];
        write(&mut vm, obstacle, b"solid", &[2.0f32.to_bits()]);
        parameters(
            &mut vm,
            &[
                &[obstacle],
                &[(-4.0f32).to_bits(); 3],
                &[4.0f32.to_bits(), 4.0f32.to_bits(), 0],
            ],
        );
        calls.call(&mut vm, 4, 3).unwrap();
        calls.call(&mut vm, 14, 0).unwrap();
        let unrelated = vm.image.globals[1];
        let id = entity(calls, &vm, unrelated);
        let body_before = body_bits(
            calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize),
        );
        vm.image.globals[28..31].copy_from_slice(&[9.0f32.to_bits(); 3]);
        vm.image.globals[31] = vm
            .field_address(unrelated, vm.image.field(b"mins").unwrap() as u32, 3)
            .unwrap() as u32;
        write(
            &mut vm,
            0,
            b"mins",
            &[(-2.0f32).to_bits(), (-2.0f32).to_bits(), 0],
        );
        write(
            &mut vm,
            0,
            b"maxs",
            &[2.0f32.to_bits(), 2.0f32.to_bits(), 4.0f32.to_bits()],
        );
        parameters(&mut vm, &[&[0]]);
        let order = solid_order(calls);
        assert_eq!(vm.call(calls, 1, 64, false).unwrap()[0], 0.0f32.to_bits());
        calls.context.trace_rules = RuleSetId::Quake2;
        // The chosen foreign policy includes linked BODY contents. Its one
        // Q1 projection now maps that solid family to native CONTENTS_SOLID.
        assert_eq!(vm.call(calls, 1, 64, false).unwrap()[0], 1.0f32.to_bits());
        // Candidate-local bounds are now executed only by the foreign linked
        // contents policy. Q1 world contents and IgnoreBoxes never read them.
        write(&mut vm, obstacle, b"mins", &[9.0f32.to_bits(); 3]);
        calls.context.trace_rules = RuleSetId::Quake;
        assert_eq!(vm.call(calls, 1, 64, false).unwrap()[0], 0.0f32.to_bits());
        calls.context.trace_rules = RuleSetId::Quake2;
        vm.image.globals[1] = 73.0f32.to_bits();
        assert_eq!(vm.call(calls, 1, 64, false), Err(Trap::Builtin));
        assert_eq!(vm.image.globals[1], 73.0f32.to_bits());
        assert_eq!(
            body_bits(
                calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize)
            ),
            body_before
        );
        assert_eq!(read(&vm, unrelated, b"mins", 3), [9.0f32.to_bits(); 3]);
        assert_eq!(solid_order(calls), order);
    });
}

fn random_vm(random_seed: u32) -> Vm {
    let bytes = quakec_program::program(
        &[
            (Opcode::Call0, 28, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::Call1, 28, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::Call0, 28, 0, 0),
            (Opcode::Invalid, 0, 0, 0),
        ],
        &[0, 1, -7, 3, 5],
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 4,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed,
        },
    )
    .unwrap();
    vm.image.globals[28] = 2;
    vm
}

#[test]
fn qc_random_uses_seeded_native_stream_and_binary32_return_for_both_sources() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            for (seed, expected) in [
                (0, [0x3f0a_cf16, 0x3e8f_191e, 0x3e43_4987]),
                (1, [0x3f0a_cf16, 0x3e8f_191e, 0x3e43_4987]),
                (2, [0x3e3f_d180, 0x3f72_ffe6, 0x3e99_1132]),
                (42, [0x3ef9_19f2, 0x3f08_c912, 0x3f00_6301]),
            ] {
                let mut vm = random_vm(seed);
                for word in expected {
                    assert_eq!(vm.call(calls, 1, 32, false), Ok([word, 0, 0]));
                }
            }
        });
    }
}

#[test]
fn qc_random_streams_remain_vm_owned_across_module_and_source_interleaving() {
    with_services(RuleSetId::Quake, |calls| {
        let mut first = random_vm(1);
        let mut second = random_vm(42);
        assert_eq!(first.call(calls, 1, 32, false).unwrap()[0], 0x3f0a_cf16);
        calls.context.module = ModuleId(9);
        calls.context.native_rules = RuleSetId::QuakeWorld;
        assert_eq!(second.call(calls, 1, 32, false).unwrap()[0], 0x3ef9_19f2);
        assert_eq!(first.call(calls, 1, 32, false).unwrap()[0], 0x3e8f_191e);
        calls.context.module = ModuleId(1);
        calls.context.native_rules = RuleSetId::Quake;
        assert_eq!(second.call(calls, 1, 32, false).unwrap()[0], 0x3f08_c912);
        assert_eq!(first.random_word(), 1681692777);
        let mut fresh = random_vm(1);
        assert_eq!(fresh.call(calls, 1, 32, false).unwrap()[0], 0x3f0a_cf16);
    });
}

#[test]
fn qc_random_rejects_wrong_argc_before_draw_and_keeps_draw_before_guest_trap() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = random_vm(1);
            vm.image.globals[1] = 0xdead_beef;
            assert_eq!(vm.call(calls, 3, 32, false), Err(Trap::Builtin));
            assert_eq!(vm.image.globals[1], 0xdead_beef);
            assert_eq!(vm.call(calls, 4, 32, false), Err(Trap::Statement));
            assert_eq!(vm.image.globals[1], 0x3f0a_cf16);
            assert_eq!(vm.call(calls, 1, 32, false), Ok([0x3e8f_191e, 0, 0]));
        });
    }
}

#[test]
fn qc_random_includes_one_and_writes_only_the_scalar_return_word() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            // This seed's first native word is 2113667071, whose low15 bits are all set.
            let mut vm = random_vm(70403);
            vm.image.globals[2..4].copy_from_slice(&[0x7fc1_2345, 0xdead_beef]);
            assert_eq!(
                vm.call(calls, 1, 32, false),
                Ok([1.0f32.to_bits(), 0x7fc1_2345, 0xdead_beef])
            );
        });
    }
}

#[test]
fn native_qc_particle_uses_four_arguments_catalog_ids_and_delivered_output() {
    use qa_core::events::{FrameEvent, OutputSubmission};
    with_services(RuleSetId::Quake, |calls| {
        let bytes = quakec_program::program(
            &[(Opcode::Call4, 28, 0, 0), (Opcode::Return, 1, 0, 0)],
            &[0, 1, -48],
        );
        let mut vm = Vm::load(
            Image::parse(&bytes, Some(5927)).unwrap(),
            Layout {
                entities: 4,
                header_bytes: 16,
                extra_string_bytes: 128,
                state_step: 0.1,
                random_seed: 1,
            },
        )
        .unwrap();
        vm.image.globals[28] = 2;
        vm.image.globals[1] = 0x12345678;
        let client = calls
            .services
            .server
            .connect(qa_session::clients::Connection::Remote, ModuleId(2), None)
            .unwrap();
        let consumer = calls.services.server.clients[client.0 as usize]
            .output
            .unwrap();
        let presentation = calls.services.server.presentation;
        calls.context.console.source = RuleSetId::Quake2;
        calls.context.trace_rules = RuleSetId::Quake3;
        calls.platform_time = EventTime::from_milliseconds(4321);
        let binding = calls
            .services
            .registry
            .effects
            .native_particles(RuleSetId::Quake)
            .unwrap();
        let origin = Vec3([-0.25, 2.125, 17.0]);
        let direction = Vec3([-100.0, 8.0, -0.0625]);
        for (color, count, color_byte, count_byte, particles) in [
            (515.875f32, 255.75f32, 3, 255, 1024),
            (-1.875f32, 258.75f32, 255, 2, 2),
        ] {
            parameters(
                &mut vm,
                &[
                    &origin.0.map(f32::to_bits),
                    &direction.0.map(f32::to_bits),
                    &[color.to_bits(), 0xfeedface, 0xdeadbeef],
                    &[count.to_bits(), 0xabcdef01, 0x76543210],
                ],
            );
            assert_eq!(calls.call(&mut vm, 48, 3), Err(Trap::Builtin));
            assert_eq!(vm.call(calls, 1, 32, false).unwrap()[0], 0x12345678);
            let ring = &mut calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            let record = ring.next(&mut batch).unwrap();
            let FrameEvent::Effect(event) = record.event else {
                panic!("common particle effect")
            };
            let expected = binding.event(origin, direction, color_byte, count_byte);
            assert_eq!(event.effect, expected.effect);
            assert_eq!(event.count, particles);
            assert_eq!(event.position, origin);
            assert_eq!(event.direction, direction);
            assert_eq!(event.color, color_byte);
            assert_eq!(event.time, EventTime(0));
            assert_eq!((event.client, event.client_lifetime), (None, None));
            let delivery = record.delivery.unwrap();
            assert_eq!(delivery.module, ModuleId(1));
            assert_eq!(delivery.source, RuleSetId::Quake);
            assert_eq!(delivery.target, NativeTarget::All);
            assert_eq!(delivery.origin, origin);
            assert!(!delivery.reliable);
            let mut local = ring.batch(presentation).unwrap();
            assert!(ring.next(&mut local).is_none());
            assert!(ring.next(&mut batch).is_none());
            assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
            assert!(ring.is_empty());
        }
        assert_eq!(vm.image.globals[1], 0x12345678);
    });
}

#[test]
fn native_qc_particle_rejects_other_native_roles_only_when_executed() {
    use qa_core::events::OutputTarget;
    for source in [
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        with_services(source, |calls| {
            let mut vm = vm(4);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            calls.context.console.source = RuleSetId::Quake;
            parameters(
                &mut vm,
                &[&[0; 3], &[0; 3], &[7.0f32.to_bits()], &[4.0f32.to_bits()]],
            );
            assert_eq!(calls.call(&mut vm, 48, 4), Err(Trap::Builtin));
            let mut batch = calls.services.server.events.batch(consumer).unwrap();
            assert!(calls.services.server.events.next(&mut batch).is_none());
            assert_eq!(calls.call(&mut vm, 36, 1), Ok(()));
        });
    }
}
fn with_model_services(
    rules: RuleSetId,
    model_loader: Option<&mut Models>,
    run: impl FnOnce(&mut QuakeCCalls<'_, '_>),
) {
    with_collision_services(rules, model_loader, &[], run);
}
fn with_collision_services(
    rules: RuleSetId,
    model_loader: Option<&mut Models>,
    world_boxes: &[Bounds],
    run: impl FnOnce(&mut QuakeCCalls<'_, '_>),
) {
    let brushes = world_boxes
        .iter()
        .map(|&bounds| (bounds, qa_world::collision::Contents::SOLID))
        .collect::<Vec<_>>();
    with_brush_services(rules, model_loader, &brushes, run);
}
fn with_brush_services(
    rules: RuleSetId,
    mut model_loader: Option<&mut Models>,
    world_brushes: &[(Bounds, qa_world::collision::Contents)],
    run: impl FnOnce(&mut QuakeCCalls<'_, '_>),
) {
    let mut names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
    let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
    let mut server = qa_session::clients::Server::load(1, 32, 1, 1, 1, 1).unwrap();
    let mut cvars = Cvars::new().unwrap();
    let console = Context {
        source: rules,
        ..Context::default()
    };
    let max_clients = cvars.bind("sv_maxclients", console).unwrap();
    cvars.force_write(max_clients, "1").unwrap();
    let mut input = qa_input::Input::load();
    let mut commands = CommandBuffer::new();
    let vfs = qa_content::vfs::Vfs::default();
    let localization = qa_content::localization::Catalog::default();
    let config_count = qa_core::configstrings::ConfigLayout::for_rules(rules).count;
    let mut storage =
        ServiceStorage::load_native(&[(ModuleId(1), config_count, rules)], 0, &cvars).unwrap();
    let mut geometry = CollisionStore::new();
    let mut planes = Vec::new();
    let mut brushes = Vec::new();
    for (bounds, contents) in world_brushes {
        let first = planes.len() as u32;
        for axis in 0..3 {
            let mut normal = [0.0; 3];
            normal[axis] = 1.0;
            planes.push(Plane::oriented(Vec3(normal), bounds.maxs.0[axis]));
            normal[axis] = -1.0;
            planes.push(Plane::oriented(Vec3(normal), -bounds.mins.0[axis]));
        }
        brushes.push(qa_world::collision::brushes::Brush {
            sides: IndexSpan { first, count: 6 },
            contents: *contents,
        });
    }
    let mut tree = BrushTree::direct(brushes.len()).unwrap();
    tree.models
        .push(qa_world::collision::brushes::ModelRoot::Leaf(0));
    let world = geometry
        .load_brushes(
            planes,
            brushes,
            Vec::new(),
            SurfaceTable::flags(vec![SurfaceFlags::default(); world_brushes.len() * 6]),
            tree,
            vec![
                Bounds {
                    mins: Vec3([-100.0; 3]),
                    maxs: Vec3([100.0; 3]),
                },
                Bounds {
                    mins: Vec3([-4.0, -8.0, -12.0]),
                    maxs: Vec3([8.0, 16.0, 24.0]),
                },
            ],
        )
        .unwrap();
    let mut scratch = geometry.scratch();
    let developer = cvars.find("developer").unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut services = EngineServices {
        server: &mut server,
        callbacks: None,
        client_disconnect: None,
        input: &mut input,
        editor: None,
        cvars: &mut cvars,
        commands: &mut commands,
        client_connection: None,
        client_collision: None,
        renderer: None,
        scene: None,
        client_audio: None,
        vfs: &vfs,
        model_loader: model_loader
            .as_deref_mut()
            .map(|loader| loader as &mut dyn ModelLoader),
        model_lookup: None,
        sound_loader: None,
        sound_bank: None,
        localization: &localization,
        registry: &registry,
        names: &mut names,
        storage: &mut storage,
        geometry: &geometry,
        world: Some((world, 0)),
        native_geometry: Some(world),
        foreign_trace: None,
        visibility: None,
        portals: None,
        scratch: &mut scratch,
    };
    let mut calls = QuakeCCalls {
        services: &mut services,
        table: &QUAKEC,
        context: CallContext {
            native_rules: rules,
            trace_rules: rules,
            sound_policy: qa_gameplay::rules::sound_policy(rules),
            module: ModuleId(1),
            clock: ThinkTime::Seconds(3.0),
            server_frame: 0,
            console,
            allocation: AllocationPolicy::for_rules(rules, 0),
            link_order: qa_gameplay::rules::link_order(rules),
        },
        platform_time: EventTime(0),
        developer,
        unknown: &mut unknown,
    };
    run(&mut calls);
}

#[derive(Default)]
struct Models {
    loaded: Vec<(Vec<u8>, RuleSetId)>,
    loaded_searches: Vec<Option<qa_content::vfs::SearchView>>,
    expected_search: Option<qa_content::vfs::SearchView>,
    order: qa_render::material::resources::ImageSearchOrder,
    loaded_orders: Vec<qa_render::material::resources::ImageSearchOrder>,
    replacement_load: (bool, bool),
    loaded_replacements: Vec<(bool, bool)>,
    fail: bool,
}
impl ModelLookup for Models {
    fn model_has_tags(&self, _: i32) -> bool {
        false
    }
    fn loaded_world(
        &self,
        _: &[u8],
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<qa_compat::services::LoadedWorldBinding> {
        None
    }
    fn lerp_tag(
        &self,
        _: i32,
        _: i32,
        _: i32,
        _: f32,
        _: &[u8],
    ) -> Option<qa_core::primitives::Orientation> {
        None
    }
    fn model_handle(&self, _: &[u8], _: RuleSetId) -> Option<i32> {
        None
    }
    fn model_bounds(&self, _: i32) -> Option<Bounds> {
        None
    }
    fn shader_handle(&self, _: &[u8], _: RuleSetId) -> Option<i32> {
        None
    }
    fn bounds(
        &self,
        path: &[u8],
        source: RuleSetId,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<Bounds> {
        (self
            .expected_search
            .as_ref()
            .is_none_or(|expected| _search.is_some_and(|search| search.same_selection(expected)))
            && !self.fail
            && self
                .loaded
                .iter()
                .any(|(name, rules)| name == path && *rules == source))
        .then_some(Bounds {
            mins: Vec3([-16.0; 3]),
            maxs: Vec3([16.0; 3]),
        })
    }
}
impl ModelLoader for Models {
    fn lookup(&self) -> &dyn ModelLookup {
        self
    }
    fn set_image_search_order(&mut self, order: qa_render::material::resources::ImageSearchOrder) {
        self.order = order;
    }

    fn set_model_replacement_load(&mut self, enhanced: bool, skeletal: bool) {
        self.replacement_load = (enhanced, skeletal);
    }

    fn register_shader(
        &mut self,
        _: &qa_content::vfs::Vfs,
        _: &[u8],
        _: RuleSetId,
        _: bool,
        _lighting: qa_compat::services::ShaderLighting,
    ) -> Result<i32, CallError> {
        Err(CallError::Unsupported)
    }
    fn precache(
        &mut self,
        _: &qa_content::vfs::Vfs,
        path: &[u8],
        source: RuleSetId,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Result<Bounds, CallError> {
        self.loaded.push((path.to_vec(), source));
        self.loaded_searches.push(_search.cloned());
        self.loaded_orders.push(self.order);
        self.loaded_replacements.push(self.replacement_load);
        if self.fail {
            Err(CallError::File)
        } else {
            self.bounds(path, source, _search)
                .ok_or(CallError::Geometry)
        }
    }
}

#[test]
fn cold_lightstyle_builtin_uses_shared_rows_raw_strings_and_native_index_conversion() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = vm(1);
            let layout = qa_core::configstrings::ConfigLayout::for_rules(rules);
            let return_words = [0xa55a_1234, 0x1122_3344, 0x5566_7788];
            vm.image.globals[1..4].copy_from_slice(&return_words);
            let text = vm.insert_string(b"aM\x80\xffz").unwrap() as u32;
            for (style, ordinal) in [(-0.5f32, 0usize), (31.9, 31), (63.9, 63)] {
                parameters(&mut vm, &[&[style.to_bits()], &[text]]);
                calls.call(&mut vm, 35, 2).unwrap();
                assert_eq!(vm.image.globals[1..4], return_words);
                assert_eq!(
                    calls
                        .services
                        .storage
                        .configstring(calls.context.module, layout.style_index(ordinal).unwrap())
                        .unwrap()
                        .0,
                    b"aM\x80\xffz",
                );
            }
            let index = layout.style_index(31).unwrap();
            let revision = calls
                .services
                .storage
                .configstring(calls.context.module, index)
                .unwrap()
                .1;
            for style in [-1.0f32, 64.0, f32::INFINITY, f32::NAN] {
                parameters(&mut vm, &[&[style.to_bits()], &[text]]);
                assert_eq!(calls.call(&mut vm, 35, 2), Err(Trap::Builtin));
                assert_eq!(
                    calls
                        .services
                        .storage
                        .configstring(calls.context.module, index)
                        .unwrap(),
                    (b"aM\x80\xffz".as_slice(), revision)
                );
            }
            let long = vm.insert_string(&[b'm'; 64]).unwrap() as u32;
            parameters(&mut vm, &[&[31.0f32.to_bits()], &[long]]);
            assert_eq!(calls.call(&mut vm, 35, 2), Err(Trap::Builtin));
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, index)
                    .unwrap(),
                (b"aM\x80\xffz".as_slice(), revision)
            );
            assert_eq!(calls.call(&mut vm, 35, 1), Err(Trap::Builtin));
            parameters(&mut vm, &[&[31.0f32.to_bits()], &[0]]);
            calls.call(&mut vm, 35, 2).unwrap();
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, index)
                    .unwrap()
                    .0,
                b""
            );
            assert!(calls.services.server.events.is_empty());
            assert_eq!(calls.unknown.calls, 0);
            assert_eq!(
                calls.services.storage.resources(calls.context.module),
                Some(layout.resources)
            );
        });
    }
}

#[test]
fn precaches_preserve_words_exact_names_and_native_loading_rules() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut models = Models::default();
        with_model_services(rules, Some(&mut models), |calls| {
            let mut vm = vm(4);
            let first = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
            let second = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
            let capital = vm.insert_string(b"progs/Test.mdl").unwrap() as u32;
            assert_ne!(first, second);
            for (builtin, word) in [(20, first), (75, second), (20, capital)] {
                parameters(&mut vm, &[&[word]]);
                calls.call(&mut vm, builtin, 1).unwrap();
                assert_eq!(vm.image.globals[1], word);
            }
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, 1)
                    .unwrap()
                    .0,
                b"progs/test.mdl"
            );
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, 2)
                    .unwrap()
                    .0,
                b"progs/Test.mdl"
            );
            let sound = vm.insert_string(b"misc/ready.wav").unwrap() as u32;
            parameters(&mut vm, &[&[sound]]);
            calls.call(&mut vm, 19, 1).unwrap();
            calls.call(&mut vm, 76, 1).unwrap();
            assert_eq!(vm.image.globals[1], sound);
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, 257)
                    .unwrap()
                    .0,
                b"misc/ready.wav"
            );
            vm.active = true;
            for builtin in [19, 20, 75, 76] {
                assert_eq!(calls.call(&mut vm, builtin, 1), Err(Trap::Builtin));
            }
            for builtin in [68, 77] {
                calls.call(&mut vm, builtin, 1).unwrap();
                assert_eq!(vm.image.globals[1], sound);
            }
        });
        assert_eq!(
            models.loaded.len(),
            if rules == RuleSetId::Quake { 2 } else { 0 }
        );
        assert!(models.loaded.iter().all(|(_, source)| *source == rules));
    }
}

#[test]
fn synchronous_quakec_precache_samples_formats_after_cvar_set_in_the_same_service_borrow() {
    use qa_render::material::resources::ImageSearchOrder;
    let mut models = Models::default();
    with_model_services(RuleSetId::Quake, Some(&mut models), |calls| {
        let mut vm = vm(1);
        let name = vm.insert_string(b"r_texture_formats").unwrap() as u32;
        for (formats, path) in [
            (b"tga".as_slice(), b"progs/test.mdl".as_slice()),
            (b"jpeg", b"progs/next.mdl"),
        ] {
            let value = vm.insert_string(formats).unwrap() as u32;
            parameters(&mut vm, &[&[name], &[value]]);
            calls.call(&mut vm, 72, 2).unwrap();
            let model = vm.insert_string(path).unwrap() as u32;
            parameters(&mut vm, &[&[model]]);
            calls.call(&mut vm, 20, 1).unwrap();
            assert_eq!(vm.image.globals[1], model);
        }
        let malformed = vm.insert_string(b"tga \"bad").unwrap() as u32;
        parameters(&mut vm, &[&[name], &[malformed]]);
        calls.call(&mut vm, 72, 2).unwrap();
        let model = vm.insert_string(b"progs/fail.mdl").unwrap() as u32;
        parameters(&mut vm, &[&[model]]);
        assert_eq!(calls.call(&mut vm, 20, 1), Err(Trap::Builtin));
    });
    assert_eq!(models.loaded.len(), 2);
    assert_eq!(
        models.loaded_orders,
        [
            ImageSearchOrder::parse(b"tga").unwrap(),
            ImageSearchOrder::parse(b"jpeg").unwrap()
        ]
    );
    assert_eq!(models.order, ImageSearchOrder::parse(b"jpeg").unwrap());
}

#[test]
fn synchronous_precache_reads_updated_replacement_controls_with_native_numeric_truth() {
    let mut models = Models::default();
    with_model_services(RuleSetId::Quake, Some(&mut models), |calls| {
        let mut vm = vm(1);
        let enhanced = vm.insert_string(b"r_enhancedmodels").unwrap() as u32;
        let load = vm.insert_string(b"gl_md5_load").unwrap() as u32;
        for (index, value) in [b"0".as_slice(), b"0.5", b"-1"].into_iter().enumerate() {
            let value = vm.insert_string(value).unwrap() as u32;
            for name in [enhanced, load] {
                parameters(&mut vm, &[&[name], &[value]]);
                calls.call(&mut vm, 72, 2).unwrap();
            }
            let path = format!("progs/policy{index}.mdl");
            let model = vm.insert_string(path.as_bytes()).unwrap() as u32;
            parameters(&mut vm, &[&[model]]);
            calls.call(&mut vm, 20, 1).unwrap();
            assert_eq!(vm.image.globals[1], model);
        }
    });
    assert_eq!(models.loaded.len(), 3);
    assert_eq!(
        models.loaded_replacements,
        [(false, false), (true, false), (true, true)]
    );
}

#[test]
fn precache_empty_overflow_and_failed_native_load_have_original_side_effects() {
    let mut models = Models {
        fail: true,
        ..Models::default()
    };
    with_model_services(RuleSetId::Quake, Some(&mut models), |calls| {
        let mut vm = vm(4);
        let name = vm.insert_string(b"progs/missing.mdl").unwrap() as u32;
        parameters(&mut vm, &[&[name]]);
        assert_eq!(calls.call(&mut vm, 20, 1), Err(Trap::Builtin));
        assert_eq!(
            calls
                .services
                .storage
                .configstring(calls.context.module, 1)
                .unwrap()
                .0,
            b"progs/missing.mdl"
        );
        // Native registration already happened; repeat registration does not
        // retry Mod_ForName or replace the failed module's ordinal.
        calls.call(&mut vm, 20, 1).unwrap();
        for name in [b"".as_slice(), b" space", b"\tpath"] {
            let word = vm.insert_string(name).unwrap() as u32;
            parameters(&mut vm, &[&[word]]);
            assert_eq!(calls.call(&mut vm, 19, 1), Err(Trap::Builtin));
        }
        let sounds = calls
            .services
            .storage
            .resources(calls.context.module)
            .unwrap()[1];
        for ordinal in 1..256 {
            let path = format!("s{ordinal}.wav");
            assert_eq!(
                (ENGINE_CALLS.resource_index)(
                    calls.services,
                    calls.context.module,
                    sounds,
                    path.as_bytes()
                )
                .unwrap(),
                ordinal
            );
        }
        let overflow = vm.insert_string(b"extra.wav").unwrap() as u32;
        parameters(&mut vm, &[&[overflow]]);
        assert_eq!(calls.call(&mut vm, 19, 1), Err(Trap::Builtin));
    });
    assert_eq!(models.loaded.len(), 1);
}

#[test]
fn qc_inline_precache_setmodel_and_publication_use_owner_geometry_without_a_query_world() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let owner = calls.services.native_geometry.unwrap();
            calls.services.world = None;
            let mut vm = seeded_vm(calls, 8);
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let model = vm.insert_string(b"*1").unwrap() as u32;
            parameters(&mut vm, &[&[model]]);
            calls.call(&mut vm, 20, 1).unwrap();
            write(&mut vm, value, b"solid", &[4.0_f32.to_bits()]);
            parameters(&mut vm, &[&[value], &[model]]);
            calls.call(&mut vm, 3, 2).unwrap();
            let bounds = calls.services.geometry.model_bounds(owner, 1).unwrap();
            assert_eq!(
                read(&vm, value, b"mins", 3),
                bounds.mins.0.map(f32::to_bits)
            );
            assert_eq!(
                read(&vm, value, b"maxs", 3),
                bounds.maxs.0.map(f32::to_bits)
            );
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                calls.services.server.entities.columns.collision_shape[id.slot as usize],
                CollisionShape::Model {
                    geometry: owner,
                    index: 1
                }
            );

            calls.services.native_geometry = None;
            let missing = vm.insert_string(b"*2").unwrap() as u32;
            parameters(&mut vm, &[&[missing]]);
            assert_eq!(
                calls.call(&mut vm, 20, 1),
                if rules == RuleSetId::QuakeWorld {
                    Ok(())
                } else {
                    Err(Trap::Builtin)
                }
            );
            let models = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[0];
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, models.get(2).unwrap())
                    .unwrap()
                    .0,
                b"*2"
            );
            parameters(&mut vm, &[&[value], &[missing]]);
            assert_eq!(calls.call(&mut vm, 3, 2), Err(Trap::Builtin));
            assert_eq!(read(&vm, value, b"model", 1), [missing]);
            assert_eq!(read(&vm, value, b"modelindex", 1), [2.0_f32.to_bits()]);
            assert_eq!(
                calls.services.server.entities.columns.collision_shape[id.slot as usize],
                CollisionShape::Model {
                    geometry: owner,
                    index: 1
                }
            );
        });
    }
}

#[test]
fn setmodel_only_uses_precached_ordinals_and_native_bounds_policy() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut models = Models::default();
        with_model_services(rules, Some(&mut models), |calls| {
            let mut vm = seeded_vm(calls, 4);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            let id = entity(calls, &vm, value);
            let initial = Bounds {
                mins: Vec3([-2.0; 3]),
                maxs: Vec3([3.0; 3]),
            };
            write(&mut vm, value, b"mins", &initial.mins.0.map(f32::to_bits));
            write(&mut vm, value, b"maxs", &initial.maxs.0.map(f32::to_bits));
            let name = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
            parameters(&mut vm, &[&[value], &[name]]);
            assert_eq!(calls.call(&mut vm, 3, 2), Err(Trap::Builtin));
            assert_eq!(read(&vm, value, b"modelindex", 1), [0]);
            parameters(&mut vm, &[&[name]]);
            calls.call(&mut vm, 20, 1).unwrap();
            parameters(&mut vm, &[&[value], &[name]]);
            calls.call(&mut vm, 3, 2).unwrap();
            assert_eq!(read(&vm, value, b"model", 1), [name]);
            assert_eq!(read(&vm, value, b"modelindex", 1), [1.0f32.to_bits()]);
            let expected = if rules == RuleSetId::Quake {
                Vec3([-16.0; 3])
            } else {
                initial.mins
            };
            assert_eq!(read(&vm, value, b"mins", 3), expected.0.map(f32::to_bits));
            assert_eq!(
                calls.services.server.entities.columns.appearance[id.slot as usize].models[0],
                ModelRef::native(ResourceNamespace::Module(calls.context.module), 1)
            );
            assert_eq!(
                calls.services.server.area.bounds(id).is_some(),
                rules == RuleSetId::Quake
            );
            parameters(&mut vm, &[&[value], &[0]]);
            calls.call(&mut vm, 3, 2).unwrap();
            assert_eq!(
                read(&vm, value, b"mins", 3),
                if rules == RuleSetId::Quake {
                    [0; 3]
                } else {
                    initial.mins.0.map(f32::to_bits)
                }
            );
            let inline = vm.insert_string(b"*1").unwrap() as u32;
            parameters(&mut vm, &[&[inline]]);
            calls.call(&mut vm, 20, 1).unwrap();
            parameters(&mut vm, &[&[value], &[inline]]);
            calls.call(&mut vm, 3, 2).unwrap();
            assert_eq!(
                read(&vm, value, b"mins", 3),
                [-4.0f32, -8.0, -12.0].map(f32::to_bits)
            );
            assert_eq!(
                read(&vm, value, b"size", 3),
                [12.0f32, 24.0, 36.0].map(f32::to_bits)
            );
            assert!(calls.services.server.area.bounds(id).is_some());
        });
    }
}

#[test]
fn qc_sound_uses_registered_bank_ring_native_origin_and_independent_sound_rules() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 4);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            let id = entity(calls, &vm, value);
            write(
                &mut vm,
                value,
                b"origin",
                &[11.0f32, 22.0, 33.0].map(f32::to_bits),
            );
            write(
                &mut vm,
                value,
                b"mins",
                &[-2.0f32, -4.0, -6.0].map(f32::to_bits),
            );
            write(
                &mut vm,
                value,
                b"maxs",
                &[6.0f32, 12.0, 18.0].map(f32::to_bits),
            );
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            let name = vm.insert_string(b"misc/test.wav").unwrap() as u32;
            parameters(&mut vm, &[&[name]]);
            calls.call(&mut vm, 19, 1).unwrap();
            let rate = std::num::NonZeroU32::new(44100).unwrap();
            let bank = std::sync::Arc::new(
                qa_audio::Bank::load(
                    rate,
                    vec![(
                        "sound/misc/test.wav".into(),
                        Pcm {
                            rate,
                            channels: PcmChannels::Mono,
                            samples: vec![1000; 4],
                            loop_start: None,
                            loop_end: None,
                            loop_invalid: false,
                        },
                    )],
                )
                .unwrap(),
            );
            let sound = bank.find("sound/misc/test.wav").unwrap();
            let sounds = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[1];
            calls
                .services
                .storage
                .bind_sound_bank(
                    calls.context.module,
                    sounds,
                    bank.as_ref(),
                    Some(qa_audio::SoundSource::DEFAULT),
                )
                .unwrap();
            calls.context.sound_policy = qa_gameplay::rules::sound_policy(RuleSetId::Quake3);
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Remote,
                    ModuleId::default(),
                    None,
                )
                .unwrap();
            let consumer = calls.services.server.clients[client.0 as usize]
                .output
                .unwrap();
            let channel = if rules == RuleSetId::QuakeWorld {
                9.0f32
            } else {
                1.0f32
            };
            calls.platform_time = EventTime::from_milliseconds(4321);
            parameters(
                &mut vm,
                &[
                    &[value],
                    &[channel.to_bits()],
                    &[name],
                    &[0.501f32.to_bits()],
                    &[1.0f32.to_bits()],
                ],
            );
            calls.call(&mut vm, 8, 5).unwrap();
            let mut batch = calls.services.server.events.batch(consumer).unwrap();
            let record = calls.services.server.events.next(&mut batch).unwrap();
            let qa_core::events::FrameEvent::Sound(event) = record.event else {
                panic!("sound record")
            };
            assert_eq!(event.sound, sound);
            assert_eq!(event.entity, Some(SoundEmitter::Entity(id)));
            assert_eq!(
                event.spatial,
                qa_gameplay::rules::sound_policy(RuleSetId::Quake3)
            );
            assert_eq!(
                event.position,
                if rules == RuleSetId::Quake {
                    Vec3([13.0, 26.0, 39.0])
                } else {
                    Vec3([11.0, 22.0, 33.0])
                }
            );
            assert_eq!(event.volume.to_bits(), (127.0f32 / 255.0).to_bits());
            assert_eq!(event.channel, 1);
            let delivery = record.delivery.unwrap();
            assert_eq!(delivery.source, rules);
            assert_eq!(delivery.target, NativeTarget::All);
            assert_eq!(delivery.reliable, rules == RuleSetId::QuakeWorld);
            assert!(calls.services.server.area.bounds(id).is_none());
            assert_eq!(event.queued_at, calls.platform_time);

            // Entity water transitions use this byte-name entry; the builtin
            // string-offset entry must produce the same ring-owned event.
            vm.start_sound(
                calls.services,
                calls.context,
                calls.platform_time,
                value,
                b"misc/test.wav",
                channel as i32,
                127,
                1.0,
            )
            .unwrap();
            let mut direct = calls.services.server.events.batch(consumer).unwrap();
            assert_eq!(
                calls
                    .services
                    .server
                    .events
                    .next(&mut direct)
                    .unwrap()
                    .sequence,
                record.sequence
            );
            let direct = calls.services.server.events.next(&mut direct).unwrap();
            let qa_core::events::FrameEvent::Sound(direct_event) = direct.event else {
                panic!("direct start-sound record")
            };
            assert_eq!(direct.delivery, record.delivery);
            assert_eq!(
                (
                    direct_event.sound,
                    direct_event.source_ordinal,
                    direct_event.spatial,
                    direct_event.entity,
                    direct_event.position,
                    direct_event.positioned,
                    direct_event.channel,
                    direct_event.queued_at,
                ),
                (
                    event.sound,
                    event.source_ordinal,
                    event.spatial,
                    event.entity,
                    event.position,
                    event.positioned,
                    event.channel,
                    event.queued_at,
                )
            );
            assert_eq!(direct_event.volume.to_bits(), event.volume.to_bits());
            assert_eq!(
                direct_event.attenuation.to_bits(),
                event.attenuation.to_bits()
            );
            assert_eq!(direct_event.action, event.action);
            assert_eq!(direct_event.native_fields, event.native_fields);
            assert_eq!(direct_event.client_lifetime, event.client_lifetime);
            assert_eq!(direct_event.client, event.client);
            let before = calls.services.server.events.sequence_range();
            for (channel, volume, attenuation) in [
                (-1, 255, 1.0),
                (16, 255, 1.0),
                (0, -1, 1.0),
                (0, 256, 1.0),
                (0, 255, -1.0),
                (0, 255, 5.0),
                (0, 255, f32::NAN),
            ] {
                assert_eq!(
                    vm.start_sound(
                        calls.services,
                        calls.context,
                        calls.platform_time,
                        u32::MAX,
                        b"unregistered.wav",
                        channel,
                        volume,
                        attenuation,
                    ),
                    Err(CallError::Text)
                );
                assert_eq!(calls.services.server.events.sequence_range(), before);
            }
            if rules == RuleSetId::QuakeWorld {
                let phs = calls
                    .services
                    .cvars
                    .bind("sv_phs", calls.context.console)
                    .unwrap();
                for (enabled, target) in [("1", NativeTarget::Hearing), ("0", NativeTarget::All)] {
                    calls.services.cvars.force_write(phs, enabled).unwrap();
                    parameters(
                        &mut vm,
                        &[
                            &[value],
                            &[1.0f32.to_bits()],
                            &[name],
                            &[1.0f32.to_bits()],
                            &[1.0f32.to_bits()],
                        ],
                    );
                    calls.call(&mut vm, 8, 5).unwrap();
                    let mut batch = calls.services.server.events.batch(consumer).unwrap();
                    let records =
                        std::iter::from_fn(|| calls.services.server.events.next(&mut batch))
                            .collect::<Vec<_>>();
                    let delivery = records.last().unwrap().delivery.unwrap();
                    assert_eq!(delivery.target, target);
                    assert!(!delivery.reliable);
                }
            }
        });
    }
}
fn seeded_vm(calls: &mut QuakeCCalls<'_, '_>, capacity: usize) -> Vm {
    let vm = vm(if calls.context.native_rules == RuleSetId::QuakeWorld {
        capacity + 31
    } else {
        capacity
    });
    seed_vm(calls, vm)
}
fn seed_vm(calls: &mut QuakeCCalls<'_, '_>, mut vm: Vm) -> Vm {
    let anchor = calls
        .services
        .server
        .entities
        .allocate(
            calls.context.clock,
            calls.context.module,
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    calls.services.server.entities.columns.native_entity[anchor.slot as usize] =
        Some(NativeEntity {
            module: calls.context.module,
            slot: 0,
        });
    vm.seed_entities(
        &calls.services.server.entities,
        calls.context.module,
        calls.context.native_rules,
        calls
            .services
            .storage
            .max_clients(calls.services.cvars, calls.context.native_rules),
    )
    .unwrap();
    vm
}

#[test]
fn push_native_movetype_reaches_provider_scan_with_native_decode_policy() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut fields = FIELDS.to_vec();
            fields.push((2, 37, b"movetype".as_slice()));
            let capacity = if rules == RuleSetId::QuakeWorld {
                39
            } else {
                8
            };
            let mut vm = seed_vm(calls, vm_with_definitions(capacity, GLOBALS, &fields));
            let native = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, native);
            for (value, expected) in [
                (7.0f32, Motion::Push),
                (
                    7.75f32,
                    if rules == RuleSetId::QuakeWorld {
                        Motion::Push
                    } else {
                        Motion::Unsupported
                    },
                ),
            ] {
                write(&mut vm, native, b"movetype", &[value.to_bits()]);
                vm.project_entities(calls.services, calls.context).unwrap();
                let mut cursor = 1;
                let current = vm
                    .physics_entity(&calls.services.server.entities, &mut cursor)
                    .unwrap()
                    .unwrap();
                assert_eq!(current.entity, id);
                assert_eq!(
                    current.native.slot as u32,
                    vm.entity_number(native).unwrap()
                );
                assert_eq!(current.phase, expected);
            }
        });
    }
}

#[test]
fn push_native_scan_includes_clients_freed_rows_and_later_spawns() {
    with_services(RuleSetId::QuakeWorld, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let first = vm.spawn_entity(calls.services, calls.context).unwrap();
        let native = NativeEntity {
            module: calls.context.module,
            slot: vm.entity_number(first).unwrap() as i32,
        };
        assert_eq!(native.slot, 33);
        vm.remove_entity(calls.services, calls.context, first)
            .unwrap();
        let mut cursor = 1;
        for slot in 1..=33 {
            let observed = vm.push_candidate(&mut cursor).unwrap().unwrap();
            assert_eq!(
                observed,
                NativeEntity {
                    module: calls.context.module,
                    slot
                }
            );
            assert_eq!(
                vm.push_free(&calls.services.server.entities, observed)
                    .unwrap(),
                slot == 33
            );
        }
        assert_eq!(vm.push_candidate(&mut cursor).unwrap(), None);
        let later = vm.spawn_entity(calls.services, calls.context).unwrap();
        assert_eq!(vm.entity_number(later).unwrap(), 34);
        assert_eq!(vm.push_candidate(&mut cursor).unwrap().unwrap().slot, 34);
        assert_eq!(vm.push_candidate(&mut cursor).unwrap(), None);
    });
}

#[test]
fn push_native_scan_skips_aliases_without_skipping_primary_freed_or_later_rows() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let first = vm.spawn_entity(calls.services, calls.context).unwrap();
            let first_ordinal = vm.entity_number(first).unwrap() as i32;
            let foreign = alias_actor(calls, ModuleId(2), 7);
            let alias = vm
                .admit_entity(&calls.services.server.entities, calls.context, foreign)
                .unwrap();
            assert_eq!(
                vm.entity_number(alias.offset()).unwrap() as i32,
                first_ordinal + 1
            );
            let last = vm.spawn_entity(calls.services, calls.context).unwrap();
            let last_ordinal = vm.entity_number(last).unwrap() as i32;
            assert_eq!(last_ordinal, first_ordinal + 2);
            vm.remove_entity(calls.services, calls.context, first)
                .unwrap();
            let mut cursor = 1;
            for slot in (1..=first_ordinal).chain([last_ordinal]) {
                let observed = vm.push_candidate(&mut cursor).unwrap().unwrap();
                assert_eq!(
                    observed,
                    NativeEntity {
                        module: calls.context.module,
                        slot
                    }
                );
                assert_eq!(
                    vm.push_free(&calls.services.server.entities, observed)
                        .unwrap(),
                    slot == first_ordinal
                );
            }
            assert_eq!(vm.push_candidate(&mut cursor).unwrap(), None);
            let foreign = alias_actor(calls, ModuleId(3), 7);
            let later_alias = vm
                .admit_entity(&calls.services.server.entities, calls.context, foreign)
                .unwrap();
            assert_eq!(
                vm.entity_number(later_alias.offset()).unwrap() as i32,
                last_ordinal + 1
            );
            let later = vm.spawn_entity(calls.services, calls.context).unwrap();
            let later_ordinal = vm.entity_number(later).unwrap() as i32;
            assert_eq!(later_ordinal, last_ordinal + 2);
            assert_eq!(
                vm.push_candidate(&mut cursor).unwrap().unwrap().slot,
                later_ordinal
            );
            assert_eq!(vm.push_candidate(&mut cursor).unwrap(), None);
            vm.release_entity(alias).unwrap();
            vm.release_entity(later_alias).unwrap();
        });
    }
}

#[test]
fn push_saved_primary_backing_cannot_write_an_alias_after_slot_takeover() {
    use qa_session::physics::{MotionWrite, PushNumber};

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let saved = vm.spawn_entity(calls.services, calls.context).unwrap();
            vm.remove_entity(calls.services, calls.context, saved)
                .unwrap();
            calls.context.clock = ThinkTime::Seconds(100.0);
            let foreign = alias_actor(calls, ModuleId(2), 7);
            let alias = vm
                .admit_entity(&calls.services.server.entities, calls.context, foreign)
                .unwrap();
            assert_eq!(alias.offset(), saved);
            let backing = vm
                .entities
                .read(u64::from(saved), vm.stride)
                .unwrap()
                .to_vec();
            let body = calls
                .services
                .server
                .entities
                .columns
                .body(foreign.slot as usize);
            assert_eq!(
                vm.push_store(
                    &mut calls.services.server.entities,
                    calls.context,
                    saved,
                    PushNumber::Motion(MotionWrite::Origin(0)),
                    91.0,
                ),
                Err(CallError::Unsupported)
            );
            assert_eq!(
                vm.push_number(saved, PushNumber::Motion(MotionWrite::Origin(0))),
                Err(CallError::Unsupported)
            );
            assert_eq!(vm.push_flags(saved), Err(CallError::Unsupported));
            assert_eq!(vm.blocked_binding(saved), Err(CallError::Unsupported));
            assert_eq!(
                vm.entities.read(u64::from(saved), vm.stride).unwrap(),
                backing
            );
            assert_eq!(
                body_bits(
                    calls
                        .services
                        .server
                        .entities
                        .columns
                        .body(foreign.slot as usize)
                ),
                body_bits(body)
            );
            assert_eq!(
                vm.resolve_entity(&calls.services.server.entities, calls.context.module, saved)
                    .unwrap(),
                Some(foreign)
            );
            vm.release_entity(alias).unwrap();
        });
    }
}

#[test]
fn pushed_client_pose_refresh_preserves_selected_bounds_and_rejects_old_lifetimes() {
    use qa_session::clients::Connection;

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let native = NativeEntity {
                module: calls.context.module,
                slot: 1,
            };
            let client = calls
                .services
                .server
                .connect(Connection::Local, calls.context.module, Some(native))
                .unwrap();
            let mut vm = seeded_vm(calls, 8);
            let value = vm.stride as u32;
            let captured = calls.services.server.clients[client.0 as usize].entity;
            write(
                &mut vm,
                value,
                b"origin",
                &[100.0f32.to_bits(), 200.0f32.to_bits(), 300.0f32.to_bits()],
            );
            write(
                &mut vm,
                value,
                b"velocity",
                &[4.0f32.to_bits(), 5.0f32.to_bits(), 6.0f32.to_bits()],
            );
            write(
                &mut vm,
                value,
                b"mins",
                &[
                    (-16.0f32).to_bits(),
                    (-16.0f32).to_bits(),
                    (-24.0f32).to_bits(),
                ],
            );
            // The narrow pose boundary must not parse unrelated model/owner or
            // require absent health, view, environment and tuning fields.
            write(&mut vm, value, b"model", &[u32::MAX]);
            write(&mut vm, value, b"owner", &[u32::MAX]);
            let player = &mut calls.services.server.clients[client.0 as usize].player;
            player.body.mins = Vec3([-15.0, -15.0, -12.0]);
            player.body.maxs = Vec3([15.0, 15.0, 20.0]);
            player.movement_rules = RuleSetId::Quake3;
            player.trace_rules = RuleSetId::Quake2;
            player.movement.mode = MovementMode::Fly;
            player.movement.water_level = 3;
            player.movement.tuning.gravity_multiplier = 1.75;
            player.view_angles = Vec3([11.0, 22.0, 33.0]);
            player.view.accept_offset(Vec3([1.0, 2.0, 3.0]), None);

            vm.refresh_client_pose(calls.services, calls.context, value, captured)
                .unwrap();
            let player = &calls.services.server.clients[client.0 as usize].player;
            assert_eq!(
                player.body.position,
                if rules == RuleSetId::QuakeWorld {
                    Vec3([99.0, 199.0, 288.0])
                } else {
                    Vec3([100.0, 200.0, 300.0])
                }
            );
            assert_eq!(player.body.velocity, Vec3([4.0, 5.0, 6.0]));
            assert_eq!(player.body.mins, Vec3([-15.0, -15.0, -12.0]));
            assert_eq!(player.body.maxs, Vec3([15.0, 15.0, 20.0]));
            assert_eq!(player.movement_rules, RuleSetId::Quake3);
            assert_eq!(player.trace_rules, RuleSetId::Quake2);
            assert_eq!(player.movement.mode, MovementMode::Fly);
            assert_eq!(player.movement.water_level, 3);
            assert_eq!(player.movement.tuning.gravity_multiplier, 1.75);
            assert_eq!(player.view_angles, Vec3([11.0, 22.0, 33.0]));
            assert_eq!(
                player.view.raw_offset(),
                Some((Vec3([1.0, 2.0, 3.0]), None))
            );

            let before = body_bits(player.body);
            calls.services.server.clients[client.0 as usize].module = ModuleId(17);
            assert_eq!(
                vm.refresh_client_pose(calls.services, calls.context, value, captured),
                Err(CallError::Entity)
            );
            assert_eq!(
                body_bits(calls.services.server.clients[client.0 as usize].player.body),
                before
            );
            calls.services.server.clients[client.0 as usize].module = calls.context.module;
            assert!(calls.services.server.disconnect(client));
            vm.refresh_client_pose(calls.services, calls.context, value, captured)
                .unwrap();
            let reconnected = calls
                .services
                .server
                .connect(Connection::Local, calls.context.module, Some(native))
                .unwrap();
            assert_eq!(reconnected, client);
            let replacement = calls.services.server.clients[client.0 as usize].entity;
            assert_ne!(replacement, captured);
            vm.bind_client(&calls.services.server.entities, native, replacement)
                .unwrap();
            calls.services.server.clients[client.0 as usize]
                .player
                .body
                .position = Vec3([777.0; 3]);
            let before = body_bits(calls.services.server.clients[client.0 as usize].player.body);
            vm.refresh_client_pose(calls.services, calls.context, value, captured)
                .unwrap();
            assert_eq!(
                body_bits(calls.services.server.clients[client.0 as usize].player.body),
                before
            );
        });
    }
}

#[test]
fn qc_trace_targets_validate_models_lazily_in_their_owners_geometry_namespace() {
    use qa_world::collision::{NativeTraceTarget, brushes::ModelRoot};

    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        let native = NativeEntity {
            module: calls.context.module,
            slot: vm.entity_number(value).unwrap() as i32,
        };
        let columns = &calls.services.server.entities.columns;
        let mut cached = NativeTraceTarget {
            body: columns.body(id.slot as usize),
            angles: Vec3::default(),
            shape: CollisionShape::Box,
            rules: columns.model_rules[id.slot as usize],
            contents_pose: None,
        };
        let mut store = CollisionStore::new();
        let load = |store: &mut CollisionStore, inline: Bounds| {
            let mut tree = BrushTree::direct(0).unwrap();
            tree.models.push(ModelRoot::Leaf(0));
            store
                .load_brushes(
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    SurfaceTable::flags(Vec::new()),
                    tree,
                    vec![Bounds::default(), inline],
                )
                .unwrap()
        };
        let query_geometry = load(
            &mut store,
            Bounds {
                mins: Vec3([-4.0; 3]),
                maxs: Vec3([4.0; 3]),
            },
        );
        let owner_geometry = load(
            &mut store,
            Bounds {
                mins: Vec3([-8.0; 3]),
                maxs: Vec3([8.0; 3]),
            },
        );
        assert_ne!(query_geometry, owner_geometry);
        cached.shape = CollisionShape::Model {
            geometry: query_geometry,
            index: 1,
        };
        write(&mut vm, value, b"model", &[u32::MAX]);
        write(&mut vm, value, b"solid", &[0]);
        assert!(
            !vm.with_trace_view(calls.context, value, None, |view, _| {
                Ok(view.fields(native, false).unwrap().enabled)
            })
            .unwrap()
        );
        write(&mut vm, value, b"solid", &[2.0_f32.to_bits()]);
        let target = vm
            .with_trace_view(calls.context, value, None, |view, _| {
                Ok(view.target(native, cached, false, &store, None))
            })
            .unwrap()
            .unwrap();
        assert_eq!(target.shape, CollisionShape::Box);
        write(&mut vm, value, b"solid", &[4.0_f32.to_bits()]);
        assert!(
            vm.with_trace_view(calls.context, value, Some(owner_geometry), |view, _| {
                Ok(view.fields(native, false).unwrap().enabled)
            })
            .unwrap()
        );
        assert!(matches!(
            vm.with_trace_view(calls.context, value, Some(owner_geometry), |view, _| {
                Ok(view.target(native, cached, false, &store, None))
            }),
            Err(CallError::Memory)
        ));
        let inline = vm.insert_string(b"*1").unwrap() as u32;
        write(&mut vm, value, b"model", &[inline]);
        assert!(matches!(
            vm.with_trace_view(calls.context, value, None, |view, _| {
                Ok(view.target(native, cached, false, &store, None))
            }),
            Err(CallError::Geometry)
        ));
        let target = vm
            .with_trace_view(calls.context, value, Some(owner_geometry), |view, _| {
                assert!(view.supports(native.module));
                assert!(!view.supports(ModuleId(2)));
                assert!(
                    view.fields(
                        NativeEntity {
                            module: ModuleId(2),
                            ..native
                        },
                        false
                    )
                    .is_none()
                );
                Ok(view.target(native, cached, false, &store, None))
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            target.shape,
            CollisionShape::Model {
                geometry: owner_geometry,
                index: 1
            }
        );
    });
}

#[test]
fn shared_foreign_trace_errors_keep_the_first_failure_until_the_operation_drains_it() {
    use qa_compat::{entities::TraceError, services::ForeignTraceRead};

    with_services(RuleSetId::Quake, |calls| {
        let vm = seeded_vm(calls, 4);
        vm.with_trace_view(calls.context, 0, None, |view, _| {
            let error = TraceError::default();
            let read = Some(ForeignTraceRead {
                view,
                error: &error,
            });
            assert!(!error.failed());
            assert_eq!(error.capture(|| Err::<(), _>(CallError::Memory)), None);
            assert_eq!(
                error.capture(|| panic!("later reads stop after the first error")),
                None::<()>
            );
            assert!(error.failed());
            assert_eq!(error.finish(), Err(CallError::Memory));
            assert_eq!(error.finish(), Err(CallError::Memory));
            assert_eq!(
                ForeignTraceRead::complete(read, Err::<(), _>(CallError::Geometry)),
                Err(CallError::Memory)
            );
            assert_eq!(error.finish(), Ok(()));
            assert!(!error.failed());
            assert_eq!(
                ForeignTraceRead::complete(read, Err::<(), _>(CallError::Geometry)),
                Err(CallError::Geometry)
            );
            assert_eq!(error.capture(|| Ok(7)), Some(7));
            assert_eq!(error.capture(|| Err::<(), _>(CallError::Entity)), None);
            assert_eq!(
                ForeignTraceRead::complete(read, Ok(7)),
                Err(CallError::Entity)
            );
            assert_eq!(ForeignTraceRead::complete(read, Ok(7)), Ok(7));
            assert_eq!(ForeignTraceRead::complete(None, Ok(7)), Ok(7));
            Ok(())
        })
        .unwrap();
    });
}

#[test]
fn push_ground_decodes_foreign_aliases_without_rebinding_stale_lifetimes() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let rider = vm.spawn_entity(calls.services, calls.context).unwrap();
            let ordinal = vm.entity_number(rider).unwrap() as i32;
            let foreign = alias_actor(calls, ModuleId(2), 7);
            let alias = vm
                .admit_entity(&calls.services.server.entities, calls.context, foreign)
                .unwrap();
            let weak = CollisionOwner::Native(NativeEntity {
                module: ModuleId(2),
                slot: 7,
            });
            write(&mut vm, rider, b"groundentity", &[alias.offset()]);
            write(&mut vm, rider, b"owner", &[alias.offset()]);
            // Entity-word interpretation never executes an unrelated model read.
            write(&mut vm, rider, b"model", &[u32::MAX]);
            let ground = vm.push_ground(calls.context, rider).unwrap();
            assert_eq!(ground, CollisionOwner::Lifetime(foreign));
            assert!(calls.services.server.entities.same_owner(ground, weak));
            assert_eq!(
                vm.with_trace_view(
                    calls.context,
                    rider,
                    calls.services.native_geometry,
                    |view, pass| {
                        assert_eq!(
                            pass,
                            CollisionOwner::Native(NativeEntity {
                                module: calls.context.module,
                                slot: ordinal,
                            })
                        );
                        Ok(view.owner(NativeEntity {
                            module: calls.context.module,
                            slot: ordinal,
                        }))
                    }
                )
                .unwrap(),
                Some(ground)
            );
            assert_eq!(
                vm.with_trace_view(
                    calls.context,
                    alias.offset(),
                    calls.services.native_geometry,
                    |_, pass| Ok(pass)
                )
                .unwrap(),
                ground
            );
            assert!(
                calls
                    .services
                    .server
                    .free_entity(foreign, calls.context.clock)
            );
            calls.context.clock = ThinkTime::Seconds(100.0);
            let replacement = alias_actor(calls, ModuleId(2), 7);
            assert_eq!(replacement.slot, foreign.slot);
            assert_ne!(replacement, foreign);
            assert_eq!(vm.push_ground(calls.context, rider).unwrap(), ground);
            assert!(!calls.services.server.entities.same_owner(ground, weak));
            assert!(!calls.services.server.entities.same_owner(ground, ground));
            assert!(
                calls
                    .services
                    .server
                    .entities
                    .same_owner(CollisionOwner::Lifetime(replacement), weak)
            );
            vm.release_entity(alias).unwrap();

            write(&mut vm, rider, b"groundentity", &[0]);
            write(&mut vm, rider, b"owner", &[0]);
            let world = CollisionOwner::Native(NativeEntity {
                module: calls.context.module,
                slot: 0,
            });
            assert_eq!(vm.push_ground(calls.context, rider).unwrap(), world);
            assert!(
                !calls
                    .services
                    .server
                    .entities
                    .same_owner(world, CollisionOwner::None)
            );
            assert_eq!(
                vm.with_trace_view(
                    calls.context,
                    rider,
                    calls.services.native_geometry,
                    |view, _| Ok(view.owner(NativeEntity {
                        module: calls.context.module,
                        slot: ordinal
                    }))
                )
                .unwrap(),
                Some(CollisionOwner::None)
            );
            let anonymous = alias_actor(calls, ModuleId(3), 4);
            calls.services.server.entities.columns.native_entity[anonymous.slot as usize] = None;
            let alias = vm
                .admit_entity(&calls.services.server.entities, calls.context, anonymous)
                .unwrap();
            write(&mut vm, rider, b"groundentity", &[alias.offset()]);
            let ground = vm.push_ground(calls.context, rider).unwrap();
            assert_eq!(ground, CollisionOwner::Lifetime(anonymous));
            assert!(calls.services.server.entities.same_owner(ground, ground));
            assert_eq!(calls.services.server.entities.native_owner(ground), None);
            vm.release_entity(alias).unwrap();

            write(&mut vm, rider, b"groundentity", &[1]);
            assert_eq!(vm.push_ground(calls.context, rider), Err(CallError::Memory));
        });
    }
}

#[test]
fn push_origin_rollback_keeps_freed_backing_and_refreshes_only_current_lifetime() {
    use qa_session::physics::{MotionWrite, PushNumber};
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        let old = entity(calls, &vm, native);
        vm.remove_entity(calls.services, calls.context, native)
            .unwrap();
        write(
            &mut vm,
            native,
            b"velocity",
            &[11.0f32.to_bits(), 12.0f32.to_bits(), 13.0f32.to_bits()],
        );
        write(&mut vm, native, b"flags", &[256.0f32.to_bits()]);
        vm.push_store(
            &mut calls.services.server.entities,
            calls.context,
            native,
            PushNumber::Motion(MotionWrite::Origin(0)),
            91.0,
        )
        .unwrap();
        assert_eq!(read(&vm, native, b"origin", 3), [91.0f32.to_bits(), 0, 0]);
        assert_eq!(
            read(&vm, native, b"velocity", 3),
            [11.0f32.to_bits(), 12.0f32.to_bits(), 13.0f32.to_bits()]
        );
        assert_eq!(vm.push_flags(native), Ok(256));
        assert!(calls.services.server.entities.resolve(old).is_none());
        calls.context.clock = ThinkTime::Seconds(100.0);
        assert_eq!(
            vm.spawn_entity(calls.services, calls.context).unwrap(),
            native
        );
        let replacement = entity(calls, &vm, native);
        assert_ne!(old, replacement);
        write(
            &mut vm,
            native,
            b"velocity",
            &[21.0f32.to_bits(), 22.0f32.to_bits(), 23.0f32.to_bits()],
        );
        // An unrelated invalid model string must not force full projection.
        write(&mut vm, native, b"model", &[u32::MAX]);
        vm.push_store(
            &mut calls.services.server.entities,
            calls.context,
            native,
            PushNumber::Motion(MotionWrite::Origin(0)),
            37.0,
        )
        .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[replacement.slot as usize],
            Vec3([37.0, 0.0, 0.0])
        );
        assert_eq!(
            calls.services.server.entities.columns.velocity[replacement.slot as usize],
            Vec3([21.0, 22.0, 23.0])
        );
        assert_eq!(read(&vm, native, b"model", 1), [u32::MAX]);
        assert!(calls.services.server.entities.resolve(old).is_none());
        assert!(
            calls
                .services
                .server
                .entities
                .resolve(replacement)
                .is_some()
        );
    });
}

#[test]
fn push_callback_preparation_keeps_native_alias_order_and_current_function() {
    use qa_session::dispatch::{FunctionBinding, FunctionTable};
    with_services(RuleSetId::Quake, |calls| {
        let mut fields = FIELDS.to_vec();
        fields.extend([(2, 37, b"ltime".as_slice()), (6, 38, b"blocked".as_slice())]);
        let globals = [
            (4, 34, b"self".as_slice()),
            (2, 34, b"time".as_slice()),
            (4, 34, b"other".as_slice()),
        ];
        let mut vm = seed_vm(calls, vm_with_definitions(8, &globals, &fields));
        let table = FunctionTable::<qa_session::clients::Server>::load([(
            calls.context.module,
            RuleSetId::Quake,
            vec![
                FunctionBinding {
                    entry: 1,
                    call: |_, _, _, _| true,
                },
                FunctionBinding {
                    entry: 2,
                    call: |_, _, _, _| true,
                },
            ],
        )])
        .unwrap();
        let module = table.module_binding(calls.context.module);
        vm.bind_thinks(module, &[1, 2]);
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        write(&mut vm, native, b"think", &[2]);
        write(&mut vm, native, b"nextthink", &[1.0f32.to_bits()]);
        assert_eq!(
            vm.prepare_think(native, ThinkTime::Seconds(9.0)),
            Ok(module.think(Some(CallbackId(1))))
        );
        assert_eq!(read(&vm, native, b"nextthink", 1), [0]);
        // time, self, other => the aliased final word is native world0.
        assert_eq!(vm.image.globals[34], 0);
        write(&mut vm, native, b"blocked", &[1]);
        assert_eq!(
            vm.blocked_binding(native),
            Ok(Some(module.think(Some(CallbackId(0)))))
        );
        write(&mut vm, native, b"blocked", &[2]);
        assert_eq!(
            vm.blocked_binding(native),
            Ok(Some(module.think(Some(CallbackId(1)))))
        );
        for function in [3, u32::MAX] {
            write(&mut vm, native, b"blocked", &[function]);
            assert_eq!(
                vm.blocked_binding(native),
                Ok(Some(ThinkBinding {
                    callback: Some(CallbackId(function)),
                    ..module.think(None)
                }))
            );
        }
        write(&mut vm, native, b"blocked", &[0]);
        assert_eq!(vm.blocked_binding(native), Ok(None));
        vm.prepare_blocked(0, native).unwrap();
        assert_eq!(vm.image.globals[34], native);
    });
}

#[test]
fn push_think_clears_aliased_deadline_before_selecting_function() {
    with_services(RuleSetId::Quake, |calls| {
        let fields: Vec<_> = FIELDS
            .iter()
            .copied()
            .map(|(kind, offset, name)| (kind, if name == b"think" { 34 } else { offset }, name))
            .collect();
        let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
        let table = qa_session::dispatch::FunctionTable::<qa_session::clients::Server>::load([(
            calls.context.module,
            RuleSetId::Quake,
            vec![qa_session::dispatch::FunctionBinding {
                entry: 1,
                call: |_, _, _, _| true,
            }],
        )])
        .unwrap();
        let module = table.module_binding(calls.context.module);
        vm.bind_thinks(module, &[1]);
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        write(&mut vm, native, b"think", &[1]);
        assert_eq!(
            vm.prepare_think(native, ThinkTime::Seconds(6.0)),
            Ok(ThinkBinding {
                callback: Some(CallbackId(0)),
                ..module.think(None)
            })
        );
        assert_eq!(read(&vm, native, b"think", 1), [0]);
        assert_eq!(vm.image.globals[35], 6.0f32.to_bits());
        assert_eq!(vm.image.globals[34], native);
    });
}

#[test]
fn think_deadline_clear_publishes_aliased_origin_even_without_an_executed_callback() {
    use qa_session::dispatch::{FunctionBinding, FunctionTable};
    with_services(RuleSetId::Quake, |calls| {
        let fields: Vec<_> = FIELDS
            .iter()
            .copied()
            .map(|(kind, offset, name)| (kind, if name == b"nextthink" { 0 } else { offset }, name))
            .collect();
        let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
        let table = FunctionTable::<qa_session::clients::Server>::load([(
            calls.context.module,
            RuleSetId::Quake,
            vec![FunctionBinding {
                entry: 1,
                call: |_, _, _, _| true,
            }],
        )])
        .unwrap();
        let module = table.module_binding(calls.context.module);
        vm.bind_thinks(module, &[1]);
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, native);
        for function in [0, 3, u32::MAX] {
            let expected = Ok(ThinkBinding {
                callback: Some(CallbackId(function)),
                ..module.think(None)
            });
            write(
                &mut vm,
                native,
                b"origin",
                &[7.0f32.to_bits(), 8.0f32.to_bits(), 9.0f32.to_bits()],
            );
            write(&mut vm, native, b"think", &[function]);
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                calls.services.server.entities.columns.position[id.slot as usize],
                Vec3([7.0, 8.0, 9.0])
            );
            assert_eq!(vm.prepare_think(native, ThinkTime::Seconds(6.0)), expected);
            assert_eq!(
                read(&vm, native, b"origin", 3),
                [0, 8.0f32.to_bits(), 9.0f32.to_bits()]
            );
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            assert_eq!(
                calls.services.server.entities.columns.position[id.slot as usize],
                Vec3([0.0, 8.0, 9.0])
            );
            assert_eq!(
                calls.services.server.entities.columns.next_think[id.slot as usize],
                Some(ThinkTime::Seconds(0.0))
            );
        }
    });
}

#[test]
fn push_blocked_preparation_keeps_time_and_accepts_retained_freed_subject() {
    with_services(RuleSetId::Quake, |calls| {
        let globals = [
            (4, 34, b"self".as_slice()),
            (2, 35, b"time".as_slice()),
            (4, 36, b"other".as_slice()),
        ];
        let mut vm = seed_vm(calls, vm_with_definitions(8, &globals, FIELDS));
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        let old = entity(calls, &vm, native);
        vm.remove_entity(calls.services, calls.context, native)
            .unwrap();
        vm.image.globals[35] = 18.25f32.to_bits();
        vm.prepare_blocked(native, 0).unwrap();
        assert_eq!(vm.image.globals[34], native);
        assert_eq!(vm.image.globals[35], 18.25f32.to_bits());
        assert_eq!(vm.image.globals[36], 0);
        assert!(calls.services.server.entities.resolve(old).is_none());
    });
}

fn named_vm(calls: &mut QuakeCCalls<'_, '_>, defined: &[&[u8]]) -> Vm {
    let mut fields = FIELDS.to_vec();
    fields.extend(
        [b"classname".as_slice(), b"target", b"targetname"]
            .into_iter()
            .enumerate()
            .filter(|(_, name)| defined.contains(name))
            .map(|(index, name)| (1, 37 + index as u16, name)),
    );
    let bytes = quakec_program::program_with_definitions(
        &[(Opcode::StorePS, 28, 31, 0), (Opcode::Return, 1, 0, 0)],
        &[0, 1],
        GLOBALS,
        &fields,
        40,
        96,
    );
    let vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: if calls.context.native_rules == RuleSetId::QuakeWorld {
                36
            } else {
                5
            },
            header_bytes: 16,
            extra_string_bytes: 1024,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    let (names, bytes) = vm.name_history_reservation().unwrap();
    calls.services.names.reserve(names, bytes).unwrap();
    seed_vm(calls, vm)
}

#[test]
fn native_names_publish_exact_raw_bytes_and_dirty_target_changes_to_one_index() {
    use qa_core::names::NameMatch;
    use qa_world::targets::TargetIndex;
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            calls.context.console.source = RuleSetId::Quake2Rerelease;
            calls.context.trace_rules = RuleSetId::Quake3;
            let mut vm = named_vm(calls, &[b"classname", b"target", b"targetname"]);
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let class = vm.insert_string(b"Monster\xff").unwrap() as u32;
            let target = vm.insert_string(b"door\\path\xfe").unwrap() as u32;
            let name = vm.insert_string(b"Door\xff").unwrap() as u32;
            for (field, word) in [
                (b"classname".as_slice(), class),
                (b"target", target),
                (b"targetname", name),
            ] {
                write(&mut vm, value, field, &[word]);
            }
            let capacity = (
                calls.services.names.capacity(),
                calls.services.names.byte_capacity(),
            );
            vm.project_entities(calls.services, calls.context).unwrap();
            let slot = id.slot as usize;
            let class_id = calls.services.names.find(b"Monster\xff").unwrap();
            let target_id = calls.services.names.find(b"door\\path\xfe").unwrap();
            let upper = calls.services.names.find(b"Door\xff").unwrap();
            assert_eq!(
                calls.services.server.entities.columns.classname[slot],
                Some(class_id)
            );
            assert_eq!(
                calls.services.server.entities.columns.target[slot],
                Some(target_id)
            );
            assert_eq!(
                calls.services.server.entities.columns.targetname(slot),
                Some(upper)
            );
            assert_eq!(
                calls.services.names.get(target_id),
                Some(b"door\\path\xfe".as_slice())
            );
            let mut targets = TargetIndex::new(&calls.services.server.entities);
            assert!(targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert_eq!(
                targets
                    .find(upper, NameMatch::Exact, calls.services.names)
                    .collect::<Vec<_>>(),
                [id]
            );

            let lower_word = vm.insert_string(b"door\xff").unwrap() as u32;
            vm.image.globals[28] = lower_word;
            vm.image.globals[31] = vm
                .field_address(value, vm.image.field(b"targetname").unwrap() as u32, 1)
                .unwrap() as u32;
            vm.call(calls, 1, 100, false).unwrap();
            // StorePS writes the VM row; only its existing publication imports it.
            assert_eq!(
                calls.services.server.entities.columns.targetname(slot),
                Some(upper)
            );
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            let lower = calls.services.names.find(b"door\xff").unwrap();
            assert_ne!(upper, lower);
            assert!(targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert!(
                targets
                    .find(upper, NameMatch::Exact, calls.services.names)
                    .next()
                    .is_none()
            );
            assert_eq!(
                targets
                    .find(lower, NameMatch::Exact, calls.services.names)
                    .collect::<Vec<_>>(),
                [id]
            );
            assert_eq!(
                targets
                    .find(upper, NameMatch::Folded, calls.services.names)
                    .collect::<Vec<_>>(),
                [id]
            );
            vm.project_entities(calls.services, calls.context).unwrap();
            assert!(!targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert_eq!(
                (
                    calls.services.names.capacity(),
                    calls.services.names.byte_capacity()
                ),
                capacity
            );

            vm.remove_entity(calls.services, calls.context, value)
                .unwrap();
            assert!(targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert!(
                targets
                    .find(lower, NameMatch::Exact, calls.services.names)
                    .next()
                    .is_none()
            );
            calls.context.clock = ThinkTime::Seconds(4.0);
            let reused = vm.spawn_entity(calls.services, calls.context).unwrap();
            assert_eq!(reused, value);
            let next = entity(calls, &vm, reused);
            assert_ne!(next, id);
            let slot = next.slot as usize;
            assert_eq!(calls.services.server.entities.columns.classname[slot], None);
            assert_eq!(calls.services.server.entities.columns.target[slot], None);
            assert_eq!(
                calls.services.server.entities.columns.targetname(slot),
                None
            );
        });
    }
}

#[test]
fn native_name_definitions_preserve_absence_and_explicit_empty_offsets() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for present in [false, true] {
            with_services(rules, |calls| {
                let fields: &[&[u8]] = if present {
                    &[b"classname", b"target", b"targetname"]
                } else {
                    &[]
                };
                let mut vm = named_vm(calls, fields);
                let value = vm.spawn_entity(calls.services, calls.context).unwrap();
                let id = entity(calls, &vm, value);
                vm.project_entities(calls.services, calls.context).unwrap();
                let slot = id.slot as usize;
                let expected = present.then_some(NameId(0));
                assert_eq!(
                    calls.services.server.entities.columns.classname[slot],
                    expected
                );
                assert_eq!(
                    calls.services.server.entities.columns.target[slot],
                    expected
                );
                assert_eq!(
                    calls.services.server.entities.columns.targetname(slot),
                    expected
                );
                if present {
                    let empty = vm.insert_string(b"").unwrap() as u32;
                    assert_ne!(empty, 0);
                    for field in [b"classname".as_slice(), b"target", b"targetname"] {
                        write(&mut vm, value, field, &[empty]);
                    }
                    vm.project_entities(calls.services, calls.context).unwrap();
                    assert_eq!(
                        calls.services.server.entities.columns.classname[slot],
                        expected
                    );
                    assert_eq!(
                        calls.services.server.entities.columns.target[slot],
                        expected
                    );
                    assert_eq!(
                        calls.services.server.entities.columns.targetname(slot),
                        expected
                    );
                }
            });
        }
    }
}

#[test]
fn rejected_native_name_import_preserves_prior_common_names_and_skips_retired_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = named_vm(calls, &[b"classname", b"target", b"targetname"]);
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        let name = vm.insert_string(b"live").unwrap() as u32;
        for field in [b"classname".as_slice(), b"target", b"targetname"] {
            write(&mut vm, value, field, &[name]);
        }
        vm.project_entities(calls.services, calls.context).unwrap();
        let slot = id.slot as usize;
        let before = (
            calls.services.server.entities.columns.classname[slot],
            calls.services.server.entities.columns.target[slot],
            calls.services.server.entities.columns.targetname(slot),
        );
        for bad in [u32::MAX, vm.strings.len() as u32] {
            write(&mut vm, value, b"targetname", &[bad]);
            assert_eq!(
                vm.project_entities(calls.services, calls.context),
                Err(CallError::Memory)
            );
            assert_eq!(
                (
                    calls.services.server.entities.columns.classname[slot],
                    calls.services.server.entities.columns.target[slot],
                    calls.services.server.entities.columns.targetname(slot)
                ),
                before
            );
        }
        let unterminated = vm.strings.len() - 1;
        vm.strings.write(unterminated as u64, &[0xff]).unwrap();
        write(&mut vm, value, b"targetname", &[unterminated as u32]);
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Memory)
        );
        assert_eq!(
            calls.services.server.entities.columns.targetname(slot),
            before.2
        );
        write(&mut vm, value, b"targetname", &[name]);
        let next = vm.insert_string(b"not-yet-published").unwrap() as u32;
        let mut counter = 0;
        loop {
            let text = format!("history-{counter}");
            if calls.services.names.intern(text.as_bytes()).is_err() {
                break;
            }
            counter += 1;
        }
        write(&mut vm, value, b"targetname", &[next]);
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Capacity)
        );
        assert_eq!(
            (
                calls.services.server.entities.columns.classname[slot],
                calls.services.server.entities.columns.target[slot],
                calls.services.server.entities.columns.targetname(slot)
            ),
            before
        );
        assert_eq!(
            calls.services.names.get(before.0.unwrap()),
            Some(b"live".as_slice())
        );
        // ED_Free need not inspect target strings; the dead ordinal cannot
        // cause a later full-export scan to read malformed name offsets.
        write(&mut vm, value, b"targetname", &[u32::MAX]);
        vm.remove_entity(calls.services, calls.context, value)
            .unwrap();
        vm.project_entities(calls.services, calls.context).unwrap();
    });
}

#[test]
fn two_native_modules_share_exact_names_without_sharing_native_string_offsets() {
    with_services(RuleSetId::Quake, |calls| {
        let mut first = named_vm(calls, &[b"classname", b"target", b"targetname"]);
        let a = first.spawn_entity(calls.services, calls.context).unwrap();
        let a_id = entity(calls, &first, a);
        let a_word = first.insert_string(b"shared\xff").unwrap() as u32;
        write(&mut first, a, b"classname", &[a_word]);
        first
            .project_entities(calls.services, calls.context)
            .unwrap();
        let shared = calls.services.server.entities.columns.classname[a_id.slot as usize].unwrap();
        let before = calls.services.names.get(shared).unwrap().to_vec();

        calls.context.module = ModuleId(2);
        let mut second = named_vm(calls, &[b"classname", b"target", b"targetname"]);
        let b = second.spawn_entity(calls.services, calls.context).unwrap();
        let b_id = entity(calls, &second, b);
        second
            .insert_string(b"padding changes the native offset")
            .unwrap();
        let b_word = second.insert_string(b"shared\xff").unwrap() as u32;
        assert_ne!(a_word, b_word);
        write(&mut second, b, b"classname", &[b_word]);
        let different = second.insert_string(b"SHARED\xff").unwrap() as u32;
        write(&mut second, b, b"target", &[different]);
        second
            .project_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.classname[b_id.slot as usize],
            Some(shared)
        );
        let variant = calls.services.server.entities.columns.target[b_id.slot as usize].unwrap();
        assert_ne!(variant, shared);
        assert_eq!(
            calls.services.names.folded(variant),
            calls.services.names.folded(shared)
        );
        assert_eq!(calls.services.names.get(shared), Some(before.as_slice()));
        assert_eq!(read(&first, a, b"classname", 1), [a_word]);
        assert_eq!(read(&second, b, b"classname", 1), [b_word]);
    });
}

fn find(
    calls: &mut QuakeCCalls<'_, '_>,
    vm: &mut Vm,
    start: u32,
    field: u32,
    text: i32,
) -> Result<u32, Trap> {
    parameters(vm, &[&[start], &[field], &[text as u32]]);
    calls.call(vm, 18, 3)?;
    Ok(vm.image.globals[1])
}

#[test]
fn find_includes_frozen_native_clients_without_common_bindings() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let max_clients = calls
                .services
                .cvars
                .bind("sv_maxclients", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(max_clients, "3").unwrap();
            let mut vm = seeded_vm(calls, 8);
            let clients = if rules == RuleSetId::QuakeWorld {
                32
            } else {
                3
            };
            let stride = vm.stride as u32;
            let field = vm.image.field(b"model").unwrap() as u32;
            let live = calls.services.server.entities.len();
            let relinks = calls.services.server.area.relinks;
            for changed in ["0", "63"] {
                calls
                    .services
                    .cvars
                    .force_write(max_clients, changed)
                    .unwrap();
                for ordinal in 0..clients {
                    assert_eq!(
                        find(calls, &mut vm, ordinal * stride, field, 0),
                        Ok((ordinal + 1) * stride)
                    );
                }
                assert_eq!(find(calls, &mut vm, clients * stride, field, 0), Ok(0));
            }
            let mut cursor = 1;
            assert!(
                vm.physics_entity(&calls.services.server.entities, &mut cursor)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(calls.services.server.entities.len(), live);
            assert_eq!(calls.services.server.area.relinks, relinks);
        });
    }
}

#[test]
fn find_uses_native_order_exact_bytes_and_raw_field_words() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let first = if rules == RuleSetId::QuakeWorld {
                33
            } else {
                2
            };
            let mut mapped = Vec::new();
            for ordinal in [first + 2, first, first + 1] {
                let id = calls
                    .services
                    .server
                    .entities
                    .allocate(
                        calls.context.clock,
                        calls.context.module,
                        AllocationPolicy::EDICT,
                    )
                    .unwrap()
                    .id;
                calls.services.server.entities.columns.native_entity[id.slot as usize] =
                    Some(NativeEntity {
                        module: calls.context.module,
                        slot: ordinal,
                    });
                mapped.push(id);
            }
            let mut vm = seeded_vm(calls, 8);
            let stride = vm.stride as u32;
            let stored = vm.insert_string(b"Door\x80").unwrap();
            let matching = vm.insert_string(b"Door\x80").unwrap();
            let folded = vm.insert_string(b"door\x80").unwrap();
            assert_ne!(stored, matching);
            // The declared float field is deliberately used as the original
            // raw string-word operand, without a second field-type gate.
            let field = vm.image.field(b"skin").unwrap() as u32;
            for ordinal in first..first + 3 {
                write(&mut vm, ordinal as u32 * stride, b"skin", &[stored as u32]);
            }
            assert!(mapped[0].slot < mapped[1].slot);
            assert_eq!(
                find(calls, &mut vm, 0, field, matching),
                Ok(first as u32 * stride)
            );
            assert_eq!(
                find(calls, &mut vm, first as u32 * stride, field, matching),
                Ok((first + 1) as u32 * stride)
            );
            assert_eq!(
                find(calls, &mut vm, (first + 2) as u32 * stride, field, matching),
                Ok(0)
            );
            assert_eq!(find(calls, &mut vm, 0, field, folded), Ok(0));
        });
    }
}

#[test]
fn find_reads_live_vm_string_stores_without_entity_publication() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let native = vm.image.globals[1];
        let id = entity(calls, &vm, native);
        let field = vm.image.field(b"model").unwrap() as u32;
        let text = vm.insert_string(b"live\x80").unwrap();
        assert_eq!(find(calls, &mut vm, 0, field, text), Ok(0));
        vm.image.statements[11].opcode = Opcode::StorePS;
        vm.image.globals[28] = text as u32;
        vm.image.globals[31] = vm.field_address(native, field, 1).unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        let relinks = calls.services.server.area.relinks;
        let before = body_bits(
            calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize),
        );
        parameters(&mut vm, &[&[0], &[field], &[text as u32]]);
        let mut expected_globals = vm.image.globals.to_vec();
        expected_globals[1] = native;
        calls.call(&mut vm, 18, 3).unwrap();
        assert_eq!(&*vm.image.globals, expected_globals);
        assert_eq!(
            body_bits(
                calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize)
            ),
            before
        );
        assert_eq!(calls.services.server.area.relinks, relinks);
    });
}

#[test]
fn find_skips_freed_generations_and_never_uses_a_foreign_replacement() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            calls.context.clock = ThinkTime::Seconds(1.0);
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let first = vm.image.globals[1];
            let old = entity(calls, &vm, first);
            calls.call(&mut vm, 14, 0).unwrap();
            let second = vm.image.globals[1];
            let field = vm.image.field(b"model").unwrap() as u32;
            let text = vm.insert_string(b"match").unwrap();
            write(&mut vm, first, b"model", &[text as u32]);
            write(&mut vm, second, b"model", &[text as u32]);
            parameters(&mut vm, &[&[first]]);
            calls.call(&mut vm, 15, 1).unwrap();
            // Stale native bytes must not be read after canonical release.
            write(&mut vm, first, b"model", &[u32::MAX]);
            assert_eq!(find(calls, &mut vm, first, field, text), Ok(second));
            assert_eq!(find(calls, &mut vm, 0, field, text), Ok(second));
            let foreign = calls
                .services
                .server
                .entities
                .allocate(calls.context.clock, ModuleId(9), AllocationPolicy::EDICT)
                .unwrap()
                .id;
            assert_eq!(foreign.slot, old.slot);
            calls.services.server.entities.columns.native_entity[foreign.slot as usize] =
                Some(NativeEntity {
                    module: ModuleId(9),
                    slot: vm.entity_number(first).unwrap() as i32,
                });
            write(&mut vm, first, b"model", &[text as u32]);
            assert_eq!(find(calls, &mut vm, 0, field, text), Ok(second));
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.image.globals[1], first);
            let current = entity(calls, &vm, first);
            assert_ne!(current, old);
            assert!(calls.services.server.entities.resolve(old).is_none());
            assert!(calls.services.server.entities.resolve(foreign).is_some());
            write(&mut vm, first, b"model", &[text as u32]);
            assert_eq!(find(calls, &mut vm, 0, field, text), Ok(first));
        });
    }
}

#[test]
fn find_validates_only_executed_native_fields_and_strings() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        calls.call(&mut vm, 14, 0).unwrap();
        let last = vm.image.globals[1];
        let field = vm.image.field(b"model").unwrap() as u32;
        let text = vm.insert_string(b"first").unwrap();
        write(&mut vm, first, b"model", &[text as u32]);
        write(&mut vm, last, b"model", &[u32::MAX]);
        assert_eq!(find(calls, &mut vm, 0, field, text), Ok(first));
        assert_eq!(find(calls, &mut vm, first, field, text), Err(Trap::Builtin));
        assert_eq!(find(calls, &mut vm, last, u32::MAX, text), Ok(0));
        for invalid in [
            1,
            last + vm.stride as u32,
            vm.entities.len() as u32,
            u32::MAX,
        ] {
            assert_eq!(
                find(calls, &mut vm, invalid, field, text),
                Err(Trap::Builtin)
            );
        }
        for invalid in [vm.image.entityfields as u32, u32::MAX] {
            assert_eq!(find(calls, &mut vm, 0, invalid, text), Err(Trap::Builtin));
        }
        assert_eq!(find(calls, &mut vm, last, field, -1), Err(Trap::String));
        assert_eq!(calls.call(&mut vm, 18, 2), Err(Trap::Builtin));
        let mut unseeded = vm_with_definitions(8, GLOBALS, FIELDS);
        assert_eq!(find(calls, &mut unseeded, 0, field, 0), Err(Trap::Builtin));
    });
}

#[test]
fn native_spawn_clear_queues_projection_and_preserves_header_on_reused_backing() {
    use qa_session::dispatch::{FunctionBinding, FunctionTable};
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let table = FunctionTable::<qa_session::clients::Server>::load([(
                calls.context.module,
                rules,
                vec![FunctionBinding {
                    entry: 1,
                    call: |_, _, _, _| true,
                }],
            )])
            .unwrap();
            let module = table.module_binding(calls.context.module);
            vm.bind_thinks(module, &[1]);
            let native = vm.spawn_entity(calls.services, calls.context).unwrap();
            let old = entity(calls, &vm, native);
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            vm.entities.write(u64::from(native), &[0x5a; 16]).unwrap();
            write(&mut vm, native, b"skin", &[23.0f32.to_bits()]);
            vm.remove_entity(calls.services, calls.context, native)
                .unwrap();
            assert!(calls.services.server.entities.resolve(old).is_none());
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            calls.context.clock = ThinkTime::Seconds(100.0);
            assert_eq!(vm.spawn_entity(calls.services, calls.context), Ok(native));
            let current = entity(calls, &vm, native);
            assert_ne!(current, old);
            assert_eq!(
                vm.entities.read(u64::from(native), 16).unwrap(),
                &[0x5a; 16]
            );
            assert_eq!(read(&vm, native, b"skin", 1), [0]);
            assert_eq!(
                calls
                    .services
                    .server
                    .entities
                    .columns
                    .think_binding(current.slot as usize)
                    .path,
                0
            );
            // Only the native clear made this row dirty. Later direct host
            // setup proves that mark survives until ordinary publication.
            write(&mut vm, native, b"model", &[u32::MAX]);
            write(&mut vm, native, b"solid", &[4.0f32.to_bits()]);
            assert_eq!(
                vm.project_dirty_entities(calls.services, calls.context),
                Err(CallError::Memory)
            );
            write(&mut vm, native, b"model", &[0]);
            write(&mut vm, native, b"solid", &[0]);
            write(&mut vm, native, b"skin", &[17.0f32.to_bits()]);
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            let columns = &calls.services.server.entities.columns;
            assert_eq!(columns.appearance[current.slot as usize].skin, 17);
            assert_eq!(
                columns.think_binding(current.slot as usize),
                ThinkBinding {
                    callback: Some(CallbackId(0)),
                    ..module.think(None)
                }
            );
            assert_eq!(
                columns.next_think[current.slot as usize],
                Some(ThinkTime::Seconds(0.0))
            );
            assert_eq!(calls.services.server.area.bounds(current), None);
        });
    }
}

#[test]
fn native_removal_keeps_deadline_last_when_it_aliases_the_cleared_origin() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let fields: Vec<_> = FIELDS
                .iter()
                .copied()
                .map(|(kind, offset, name)| {
                    (kind, if name == b"nextthink" { 0 } else { offset }, name)
                })
                .collect();
            let capacity = if rules == RuleSetId::QuakeWorld {
                40
            } else {
                8
            };
            let mut vm = seed_vm(calls, vm_with_definitions(capacity, GLOBALS, &fields));
            let native = vm.spawn_entity(calls.services, calls.context).unwrap();
            let old = entity(calls, &vm, native);
            write(
                &mut vm,
                native,
                b"origin",
                &[9.0f32, 8.0, 7.0].map(f32::to_bits),
            );
            vm.remove_entity(calls.services, calls.context, native)
                .unwrap();
            assert!(calls.services.server.entities.resolve(old).is_none());
            assert_eq!(read(&vm, native, b"origin", 3), [(-1.0f32).to_bits(), 0, 0]);
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            assert_eq!(read(&vm, native, b"origin", 3), [(-1.0f32).to_bits(), 0, 0]);
        });
    }
}

#[test]
fn spawn_remove_use_native_ordinals_and_independent_common_generations() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let first = vm.image.globals[1];
            let first_ordinal = if rules == RuleSetId::QuakeWorld {
                33
            } else {
                2
            };
            assert_eq!(vm.entity_number(first), Ok(first_ordinal));
            let old = entity(calls, &vm, first);
            assert_ne!(old.slot, first_ordinal);
            write(&mut vm, first, b"skin", &[19.0f32.to_bits()]);
            parameters(&mut vm, &[&[first]]);
            calls.call(&mut vm, 15, 1).unwrap();
            assert!(calls.services.server.entities.resolve(old).is_none());
            assert_eq!(read(&vm, first, b"nextthink", 1), vec![(-1.0f32).to_bits()]);
            assert_eq!(read(&vm, first, b"skin", 1), vec![0]);
            calls.context.clock = ThinkTime::Seconds(3.5);
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(first_ordinal + 1));
            calls.context.clock = ThinkTime::Seconds(3.500_001);
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.image.globals[1], first);
            let new = entity(calls, &vm, first);
            assert_ne!(new, old);
            assert_eq!(read(&vm, first, b"nextthink", 1), vec![0]);
            assert_eq!(read(&vm, first, b"skin", 1), vec![0]);
            assert_eq!(
                calls.services.server.entities.columns.owner[new.slot as usize],
                calls.context.module
            );
        });
    }
}

#[test]
fn unconnected_reserved_clients_get_ordinary_bodies_only_at_explicit_link() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let native = vm.client_count().unwrap() as u32 * vm.stride as u32;
            write(
                &mut vm,
                native,
                b"origin",
                &[20.0f32, 10.0, 5.0].map(f32::to_bits),
            );
            write(&mut vm, native, b"mins", &[-1.0f32; 3].map(f32::to_bits));
            write(&mut vm, native, b"maxs", &[1.0f32; 3].map(f32::to_bits));
            write(&mut vm, native, b"solid", &[2.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                vm.resolve_entity(
                    &calls.services.server.entities,
                    calls.context.module,
                    native
                ),
                Ok(None)
            );
            let count = calls.services.server.entities.active().count();
            vm.relink_entity(calls.services, calls.context, native)
                .unwrap();
            let body = entity(calls, &vm, native);
            assert_eq!(calls.services.server.entities.active().count(), count + 1);
            assert_eq!(
                calls.services.server.entities.columns.position[body.slot as usize],
                Vec3([20.0, 10.0, 5.0])
            );
            assert!(calls.services.server.area.bounds(body).is_some());
            assert_eq!(
                vm.ensure_client_body(calls.services, calls.context, native),
                Ok(body)
            );
            assert_eq!(calls.services.server.entities.active().count(), count + 1);
            let bytes = vm
                .entities
                .read(u64::from(native), vm.stride)
                .unwrap()
                .to_vec();
            assert_eq!(
                vm.remove_entity(calls.services, calls.context, native),
                Err(CallError::Entity)
            );
            assert_eq!(
                vm.make_static(calls.services, calls.context, native),
                Err(CallError::Entity)
            );
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                bytes
            );
            let ordinary = vm.spawn_entity(calls.services, calls.context).unwrap();
            vm.remove_entity(calls.services, calls.context, ordinary)
                .unwrap();
            vm.relink_entity(calls.services, calls.context, ordinary)
                .unwrap();
            assert_eq!(
                vm.resolve_entity(
                    &calls.services.server.entities,
                    calls.context.module,
                    ordinary
                ),
                Ok(None)
            );
            assert_eq!(
                vm.ensure_client_body(calls.services, calls.context, ordinary),
                Err(CallError::Entity)
            );
            vm.release_client_body(calls.services, calls.context, native)
                .unwrap();
            assert!(calls.services.server.entities.resolve(body).is_none());
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                bytes
            );
        });
    }
}

#[test]
fn ensuring_reserved_client_body_projects_without_linking_and_rejects_full_capacity() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let native = vm.stride as u32;
            write(&mut vm, native, b"origin", &[7.0f32.to_bits(), 0, 0]);
            let body = vm
                .ensure_client_body(calls.services, calls.context, native)
                .unwrap();
            assert_eq!(
                calls.services.server.entities.columns.position[body.slot as usize],
                Vec3([7.0, 0.0, 0.0])
            );
            assert_eq!(calls.services.server.area.bounds(body), None);
            assert_eq!(
                vm.ensure_client_body(calls.services, calls.context, 0),
                Err(CallError::Entity)
            );
            assert_eq!(
                vm.ensure_client_body(calls.services, calls.context, native + 1),
                Err(CallError::Entity)
            );
            let mut wrong_owner = calls.context;
            wrong_owner.module = ModuleId(99);
            assert_eq!(
                vm.ensure_client_body(calls.services, wrong_owner, native),
                Err(CallError::Entity)
            );
        });
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            while calls
                .services
                .server
                .entities
                .allocate(
                    calls.context.clock,
                    calls.context.module,
                    AllocationPolicy::EDICT,
                )
                .is_some()
            {}
            let before = calls.services.server.entities.active().collect::<Vec<_>>();
            let mut context = calls.context;
            context.allocation.full = qa_world::entities::FullTable::OverwriteLastOwned;
            assert_eq!(
                vm.ensure_client_body(calls.services, context, vm.stride as u32),
                Err(CallError::Capacity)
            );
            assert_eq!(
                calls.services.server.entities.active().collect::<Vec<_>>(),
                before
            );
            assert_eq!(
                vm.resolve_entity(
                    &calls.services.server.entities,
                    calls.context.module,
                    vm.stride as u32
                ),
                Ok(None)
            );
        });
    }
}

#[test]
fn quakeworld_reserves_all_32_native_clients_despite_maxclients_one() {
    with_services(RuleSetId::QuakeWorld, |calls| {
        let client = calls
            .services
            .server
            .entities
            .allocate(
                calls.context.clock,
                calls.context.module,
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        calls.services.server.entities.columns.native_entity[client.slot as usize] =
            Some(NativeEntity {
                module: calls.context.module,
                slot: 32,
            });
        assert!(calls.services.server.entities.set_never_free(client, true));
        let mut vm = seeded_vm(calls, 4);
        let client_value = 32 * vm.stride as u32;
        write(&mut vm, client_value, b"skin", &[19.0f32.to_bits()]);
        let before = vm
            .entities
            .read(u64::from(client_value), vm.stride)
            .unwrap()
            .to_vec();
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(33));
        assert!(calls.services.server.entities.resolve(client).is_some());
        assert_eq!(
            vm.entities
                .read(u64::from(client_value), vm.stride)
                .unwrap(),
            before
        );
        parameters(&mut vm, &[&[client_value]]);
        assert_eq!(calls.call(&mut vm, 15, 1), Err(Trap::Builtin));
        assert!(calls.services.server.entities.resolve(client).is_some());
        assert_eq!(
            vm.entities
                .read(u64::from(client_value), vm.stride)
                .unwrap(),
            before
        );
    });
}

#[test]
fn native_client_reservations_are_frozen_before_spawn_and_reuse() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let max_clients = calls
                .services
                .cvars
                .bind("sv_maxclients", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(max_clients, "3").unwrap();
            let reserved = if rules == RuleSetId::Quake { 3 } else { 32 };
            let client = calls
                .services
                .server
                .entities
                .allocate(
                    calls.context.clock,
                    calls.context.module,
                    AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            calls.services.server.entities.columns.native_entity[client.slot as usize] =
                Some(NativeEntity {
                    module: calls.context.module,
                    slot: reserved,
                });
            let mut vm = seeded_vm(calls, 8);
            let client_value = reserved as u32 * vm.stride as u32;
            write(&mut vm, client_value, b"skin", &[19.0f32.to_bits()]);
            let client_bytes = vm
                .entities
                .read(u64::from(client_value), vm.stride)
                .unwrap()
                .to_vec();
            for (value, ordinal) in [("0", reserved + 1), ("7", reserved + 2)] {
                calls
                    .services
                    .cvars
                    .force_write(max_clients, value)
                    .unwrap();
                calls.call(&mut vm, 14, 0).unwrap();
                assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(ordinal as u32));
            }
            let freed_value = vm.image.globals[1];
            let old = entity(calls, &vm, freed_value);
            parameters(&mut vm, &[&[freed_value]]);
            calls.call(&mut vm, 15, 1).unwrap();
            assert!(calls.services.server.entities.resolve(old).is_none());
            calls.services.cvars.force_write(max_clients, "0").unwrap();
            calls.context.clock = ThinkTime::Seconds(3.500_001);
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.image.globals[1], freed_value);
            let current = entity(calls, &vm, freed_value);
            assert_ne!(current, old);
            assert_eq!(
                calls.services.server.entities.columns.native_entity[current.slot as usize],
                Some(NativeEntity {
                    module: calls.context.module,
                    slot: reserved + 2,
                })
            );
            assert!(calls.services.server.entities.resolve(client).is_some());
            assert_eq!(
                vm.entities
                    .read(u64::from(client_value), vm.stride)
                    .unwrap(),
                client_bytes
            );
            parameters(&mut vm, &[&[client_value]]);
            assert_eq!(calls.call(&mut vm, 15, 1), Err(Trap::Builtin));
            assert!(calls.services.server.entities.resolve(client).is_some());
        });
    }
}

#[test]
fn changed_client_reservation_reseed_is_rejected_without_reconfiguration() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        assert_eq!(
            vm.seed_entities(
                &calls.services.server.entities,
                calls.context.module,
                RuleSetId::Quake,
                4,
            ),
            Err(CallError::Entity)
        );
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(2));
    });
}

#[test]
fn rejected_protected_remove_preserves_lifetime_area_and_native_bytes() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            let id = entity(calls, &vm, value);
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            let mins = [-2.0f32; 3].map(f32::to_bits);
            let maxs = [2.0f32; 3].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &mins, &maxs]);
            calls.call(&mut vm, 4, 3).unwrap();
            let origin = [10.0f32, 0.0, 0.0].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &origin]);
            calls.call(&mut vm, 2, 2).unwrap();
            assert!(calls.services.server.entities.set_never_free(id, true));
            let body = calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize);
            let bounds = calls.services.server.area.bounds(id);
            let bytes = vm
                .entities
                .read(u64::from(value), vm.stride)
                .unwrap()
                .to_vec();
            parameters(&mut vm, &[&[value]]);
            assert_eq!(calls.call(&mut vm, 15, 1), Err(Trap::Builtin));
            assert!(calls.services.server.entities.resolve(id).is_some());
            let after = calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize);
            assert_eq!(body_bits(after), body_bits(body));
            assert_eq!(calls.services.server.area.bounds(id), bounds);
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
        });
    }
}

#[test]
fn quakec_fly_flag_stays_visible_and_raw_at_shared_publication() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            write(&mut vm, value, b"flags", &[1.0f32.to_bits()]);
            write(&mut vm, value, b"modelindex", &[7.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            let columns = &calls.services.server.entities.columns;
            assert_eq!(columns.flags[id.slot as usize], 1);
            assert!(!columns.appearance[id.slot as usize].snapshot_excluded);
            assert_eq!(read(&vm, value, b"flags", 1), [1.0f32.to_bits()]);
        });
    }
}

#[test]
fn explicit_pose_size_and_item_link_use_shared_bounds_without_implicit_relinks() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        let id = entity(calls, &vm, value);
        write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
        write(&mut vm, value, b"flags", &[256.0f32.to_bits()]);
        let mins = [-2.0f32, -3.0, -4.0].map(f32::to_bits);
        let maxs = [2.0f32, 3.0, 4.0].map(f32::to_bits);
        parameters(&mut vm, &[&[value], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        let origin = [20.0f32, 10.0, -0.0].map(f32::to_bits);
        parameters(&mut vm, &[&[value], &origin]);
        calls.call(&mut vm, 2, 2).unwrap();
        let bounds = calls.services.server.area.bounds(id).unwrap();
        assert_eq!(
            bounds,
            Bounds {
                mins: Vec3([3.0, -8.0, -4.0]),
                maxs: Vec3([37.0, 28.0, 4.0])
            }
        );
        assert_eq!(
            read(&vm, value, b"absmin", 3),
            bounds.mins.0.map(f32::to_bits)
        );
        assert_eq!(
            read(&vm, value, b"size", 3),
            [4.0f32, 6.0, 8.0].map(f32::to_bits)
        );
        let relinks = calls.services.server.area.relinks;
        calls.call(&mut vm, 2, 2).unwrap();
        assert_eq!(calls.services.server.area.relinks, relinks + 1);
        write(&mut vm, value, b"origin", &[30.0f32.to_bits(), 0, 0]);
        write(&mut vm, value, b"modelindex", &[7.0f32.to_bits()]);
        write(&mut vm, value, b"effects", &[3.0f32.to_bits()]);
        vm.project_entities(calls.services, calls.context).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([30.0, 0.0, 0.0])
        );
        assert_eq!(calls.services.server.area.bounds(id), Some(bounds));
        let appearance = calls.services.server.entities.columns.appearance[id.slot as usize];
        assert_eq!(
            appearance.models[0],
            ModelRef::native(ResourceNamespace::Module(calls.context.module), 7)
        );
        assert_eq!(appearance.native.effects, 3);
        parameters(&mut vm, &[&[value], &maxs, &mins]);
        assert_eq!(calls.call(&mut vm, 4, 3), Err(Trap::Builtin));
        assert_eq!(read(&vm, value, b"mins", 3), mins);
        assert_eq!(calls.services.server.area.bounds(id), Some(bounds));
    });
}

#[test]
fn rejected_native_link_keeps_successful_origin_stores_dirty() {
    with_services(RuleSetId::Quake, |calls| {
        let fields: Vec<_> = FIELDS
            .iter()
            .copied()
            .map(|(kind, offset, name)| (if name == b"absmax" { 1 } else { kind }, offset, name))
            .collect();
        let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, native);
        vm.project_entities(calls.services, calls.context).unwrap();
        let origin = [42.0f32, 7.0, 3.0].map(f32::to_bits);
        parameters(&mut vm, &[&[native], &origin]);
        assert_eq!(calls.call(&mut vm, 2, 2), Err(Trap::Builtin));
        assert_eq!(read(&vm, native, b"origin", 3), origin);
        assert_eq!(calls.services.server.area.bounds(id), None);
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([42.0, 7.0, 3.0])
        );
    });
}

#[test]
fn linked_bounds_refresh_selected_aliases_and_retain_unrelated_pending_rows() {
    use qa_session::dispatch::{FunctionBinding, FunctionTable};
    use qa_session::physics::{MotionWrite, PushNumber};
    use qa_world::area::LinkFlags;
    for explicit_link in [false, true] {
        with_services(RuleSetId::Quake, |calls| {
            let fields: Vec<_> = FIELDS
                .iter()
                .copied()
                .map(|(kind, offset, name)| {
                    let offset = match name {
                        b"absmin" => 0,
                        b"absmax" => 34,
                        _ => offset,
                    };
                    (kind, offset, name)
                })
                .collect();
            let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
            let table = FunctionTable::<qa_session::clients::Server>::load([(
                calls.context.module,
                RuleSetId::Quake,
                vec![FunctionBinding {
                    entry: 1,
                    call: |_, _, _, _| true,
                }],
            )])
            .unwrap();
            let module = table.module_binding(calls.context.module);
            vm.bind_thinks(module, &[1]);
            let native = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, native);
            let pending = vm.spawn_entity(calls.services, calls.context).unwrap();
            let pending_id = entity(calls, &vm, pending);
            write(
                &mut vm,
                native,
                b"mins",
                &[-2.0f32, -3.0, -4.0].map(f32::to_bits),
            );
            write(
                &mut vm,
                native,
                b"maxs",
                &[2.0f32, 3.0, 4.0].map(f32::to_bits),
            );
            write(
                &mut vm,
                native,
                b"origin",
                &[10.0f32, 20.0, 30.0].map(f32::to_bits),
            );
            write(&mut vm, native, b"solid", &[2.0f32.to_bits()]);
            write(&mut vm, native, b"think", &[1]);
            vm.project_entities(calls.services, calls.context).unwrap();

            // A successful guest scalar store remains pending on another row.
            // Its invalid model must not be read by selected-row publication.
            vm.image.globals[28] = 77.0f32.to_bits();
            vm.image.globals[31] = vm
                .field_address(pending, vm.image.field(b"origin").unwrap() as u32, 1)
                .unwrap() as u32;
            vm.call(calls, 6, 100, false).unwrap();
            write(&mut vm, pending, b"model", &[u32::MAX]);
            assert_eq!(
                calls.services.server.entities.columns.position[pending_id.slot as usize],
                Vec3::default()
            );

            if explicit_link {
                vm.relink_entity(calls.services, calls.context, native)
                    .unwrap();
            } else {
                (ENGINE_CALLS.link)(calls.services, calls.context, id, LinkFlags::SOLID).unwrap();
                let relinks = calls.services.server.area.relinks;
                vm.publish_link_bounds(calls.services, calls.context, native)
                    .unwrap();
                assert_eq!(calls.services.server.area.relinks, relinks);
            }
            let bounds = calls.services.server.area.bounds(id).unwrap();
            assert_eq!(bounds.mins, Vec3([7.0, 16.0, 25.0]));
            assert_eq!(bounds.maxs, Vec3([13.0, 24.0, 35.0]));
            let columns = &calls.services.server.entities.columns;
            assert_eq!(columns.position[id.slot as usize], bounds.mins);
            assert_eq!(
                columns.next_think[id.slot as usize],
                Some(ThinkTime::Seconds(13.0))
            );
            assert_eq!(
                columns.think_binding(id.slot as usize),
                ThinkBinding {
                    callback: Some(CallbackId(24.0f32.to_bits())),
                    ..module.think(None)
                }
            );
            assert_eq!(columns.position[pending_id.slot as usize], Vec3::default());

            // Both rows retain their dirty membership after local projection.
            // Alter host bytes without making a second guest store to prove it.
            write(&mut vm, native, b"frame", &[11.0f32.to_bits()]);
            write(&mut vm, pending, b"model", &[0]);
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            assert_eq!(
                calls.services.server.entities.columns.appearance[id.slot as usize].frame,
                11
            );
            assert_eq!(
                calls.services.server.entities.columns.position[pending_id.slot as usize],
                Vec3([77.0, 0.0, 0.0])
            );

            // Retained freed backing still accepts native scalar rollback, but
            // has no area bounds to publish and must not acquire a new link.
            vm.remove_entity(calls.services, calls.context, native)
                .unwrap();
            vm.push_store(
                &mut calls.services.server.entities,
                calls.context,
                native,
                PushNumber::Motion(MotionWrite::Origin(0)),
                91.0,
            )
            .unwrap();
            let relinks = calls.services.server.area.relinks;
            vm.publish_link_bounds(calls.services, calls.context, native)
                .unwrap();
            assert_eq!(calls.services.server.area.relinks, relinks);
            assert_eq!(read(&vm, native, b"origin", 1), [91.0f32.to_bits()]);
        });
    }
}

#[test]
fn native_origin_stores_import_aliased_deadline_and_function_through_dirty_rows() {
    use qa_session::dispatch::{FunctionBinding, FunctionTable};
    use qa_session::physics::MotionWrite;
    with_services(RuleSetId::Quake, |calls| {
        let fields: Vec<_> = FIELDS
            .iter()
            .copied()
            .map(|(kind, offset, name)| (kind, if name == b"origin" { 34 } else { offset }, name))
            .collect();
        let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
        let table = FunctionTable::<qa_session::clients::Server>::load([(
            calls.context.module,
            RuleSetId::Quake,
            vec![
                FunctionBinding {
                    entry: 1,
                    call: |_, _, _, _| true,
                },
                FunctionBinding {
                    entry: 2,
                    call: |_, _, _, _| true,
                },
            ],
        )])
        .unwrap();
        let module = table.module_binding(calls.context.module);
        vm.bind_thinks(module, &[1, 2]);
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, native);
        write(&mut vm, native, b"think", &[1]);
        vm.project_entities(calls.services, calls.context).unwrap();
        calls.services.server.entities.columns.position[id.slot as usize].0[0] = 5.0;
        vm.write_motion(
            &mut calls.services.server.entities,
            calls.context,
            native,
            MotionWrite::Origin(0),
        )
        .unwrap();
        calls.services.server.entities.columns.position[id.slot as usize].0[1] = f32::from_bits(2);
        vm.write_motion(
            &mut calls.services.server.entities,
            calls.context,
            native,
            MotionWrite::Origin(1),
        )
        .unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        let columns = &calls.services.server.entities.columns;
        assert_eq!(
            columns.next_think[id.slot as usize],
            Some(ThinkTime::Seconds(5.0))
        );
        assert_eq!(
            columns.think_binding(id.slot as usize),
            module.think(Some(CallbackId(1)))
        );
    });
}

#[test]
fn trace_uses_shared_collision_and_native_pass_and_result_offsets() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            let mins = [-2.0f32; 3].map(f32::to_bits);
            let maxs = [2.0f32; 3].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &mins, &maxs]);
            calls.call(&mut vm, 4, 3).unwrap();
            let origin = [10.0f32, 0.0, 0.0].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &origin]);
            calls.call(&mut vm, 2, 2).unwrap();
            let start = [0u32; 3];
            let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
            parameters(&mut vm, &[&start, &end, &[0], &[0]]);
            calls.call(&mut vm, 16, 4).unwrap();
            assert!(f32::from_bits(vm.image.globals[62]) < 1.0);
            assert_eq!(vm.image.globals[72], value);
            assert_eq!(f32::from_bits(vm.image.globals[69]), -1.0);
            let before = f32::from_bits(vm.image.globals[62]);
            let id = entity(calls, &vm, value);
            let linked_bounds = calls.services.server.area.bounds(id);
            // A field store immediately preceding a trace must affect the
            // native narrow phase without inventing a relink.
            store_and_trace(calls, &mut vm, value, b"origin", &[12.0f32.to_bits(), 0, 0]).unwrap();
            assert!(f32::from_bits(vm.image.globals[62]) > before);
            assert_eq!(calls.services.server.area.bounds(id), linked_bounds);
            let box_fraction = vm.image.globals[62];
            write(&mut vm, value, b"angles", &[f32::NAN.to_bits(); 3]);
            calls.call(&mut vm, 16, 4).unwrap();
            assert_eq!(vm.image.globals[62], box_fraction);
            assert_eq!(calls.services.server.area.bounds(id), linked_bounds);
            // Full publication still validates the presentation angles.
            assert_eq!(
                vm.project_entities(calls.services, calls.context),
                Err(CallError::Entity)
            );
            write(&mut vm, value, b"angles", &[0; 3]);
            store_and_trace(calls, &mut vm, value, b"solid", &[0]).unwrap();
            assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
            assert_eq!(calls.services.server.area.bounds(id), linked_bounds);
            parameters(&mut vm, &[&start, &end, &[0], &[value]]);
            store_and_trace(calls, &mut vm, value, b"solid", &[2.0f32.to_bits()]).unwrap();
            assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
            assert_eq!(vm.image.globals[72], 0);
            parameters(&mut vm, &[&start, &end, &[1.0f32.to_bits()], &[0]]);
            calls.call(&mut vm, 16, 4).unwrap();
            assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
            parameters(&mut vm, &[&start, &end, &[0], &[1]]);
            assert_eq!(calls.call(&mut vm, 16, 4), Err(Trap::Builtin));
        });
    }
}

fn spawn_box(
    calls: &mut QuakeCCalls<'_, '_>,
    vm: &mut Vm,
    origin: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
) -> u32 {
    calls.call(vm, 14, 0).unwrap();
    let value = vm.image.globals[1];
    write(vm, value, b"solid", &[2.0f32.to_bits()]);
    parameters(
        vm,
        &[&[value], &mins.map(f32::to_bits), &maxs.map(f32::to_bits)],
    );
    calls.call(vm, 4, 3).unwrap();
    parameters(vm, &[&[value], &origin.map(f32::to_bits)]);
    calls.call(vm, 2, 2).unwrap();
    value
}

fn solid_order(calls: &QuakeCCalls<'_, '_>) -> Vec<EntityId> {
    calls
        .services
        .server
        .area
        .query(
            &calls.services.server.entities,
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
            qa_world::area::LinkFlags::SOLID,
        )
        .map(|linked| linked.id)
        .collect()
}

#[test]
fn droptofloor_uses_live_box_and_caller_rules_over_a_shared_brush_world() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -512.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_collision_services(native, None, &[floor], |calls| {
            let mut vm = seeded_vm(calls, 8);
            let value = spawn_box(calls, &mut vm, [0.0, 0.0, 40.0], [-1.0; 3], [1.0; 3]);
            let id = entity(calls, &vm, value);
            for (trace_rules, epsilon) in
                [(RuleSetId::Quake, 0.03125f32), (RuleSetId::Quake3, 0.125)]
            {
                // The input is live, while the cached body still sits at40.
                write(
                    &mut vm,
                    value,
                    b"origin",
                    &[0.0f32, 0.0, 32.0].map(f32::to_bits),
                );
                write(&mut vm, value, b"flags", &[3.75f32.to_bits()]);
                write(&mut vm, value, b"groundentity", &[value]);
                vm.image.globals[34] = value;
                calls.context.trace_rules = trace_rules;
                calls.context.console.source = RuleSetId::Quake2;
                let trace_globals = vm.image.globals[60..73].to_vec();
                calls.call(&mut vm, 34, 0).unwrap();
                assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
                assert_eq!(
                    read(&vm, value, b"origin", 3),
                    [0.0f32, 0.0, 1.0 + epsilon].map(f32::to_bits)
                );
                assert_eq!(read(&vm, value, b"flags", 1), [515.0f32.to_bits()]);
                assert_eq!(read(&vm, value, b"groundentity", 1), [0]);
                assert_eq!(&vm.image.globals[60..73], trace_globals);
                assert_eq!(
                    calls.services.server.entities.columns.position[id.slot as usize],
                    Vec3([0.0, 0.0, 1.0 + epsilon])
                );
                // Relink precedes the flags write. The actual post-Think
                // dirty importer must then see the final native flags too.
                assert_eq!(
                    calls.services.server.entities.columns.flags[id.slot as usize],
                    3
                );
                let linked = calls.services.server.area.bounds(id);
                vm.project_dirty_entities(calls.services, calls.context)
                    .unwrap();
                assert_eq!(
                    calls.services.server.entities.columns.flags[id.slot as usize],
                    515
                );
                assert_eq!(calls.services.server.area.bounds(id), linked);
            }
        });
    }
}

#[test]
fn droptofloor_sets_linked_native_ground_and_reinserts_an_unchanged_pose() {
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(native, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let floor = spawn_box(
                calls,
                &mut vm,
                [0.0; 3],
                [-16.0, -16.0, -8.0],
                [16.0, 16.0, 0.0],
            );
            let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
            vm.image.globals[34] = value;
            calls.call(&mut vm, 34, 0).unwrap();
            assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
            assert_eq!(read(&vm, value, b"groundentity", 1), [floor]);
            assert_eq!(
                read(&vm, value, b"origin", 3),
                [0.0f32, 0.0, 1.03125].map(f32::to_bits)
            );
            let peer = spawn_box(calls, &mut vm, [8.0, 0.0, 1.03125], [-1.0; 3], [1.0; 3]);
            let value_id = entity(calls, &vm, value);
            let peer_id = entity(calls, &vm, peer);
            let order = solid_order(calls);
            assert!(
                order.iter().position(|id| *id == value_id).unwrap()
                    < order.iter().position(|id| *id == peer_id).unwrap()
            );
            let pose = read(&vm, value, b"origin", 3);
            let bounds = calls.services.server.area.bounds(value_id);
            let peer_bytes = vm
                .entities
                .read(u64::from(peer), vm.stride)
                .unwrap()
                .to_vec();
            vm.image.globals[34] = value;
            calls.call(&mut vm, 34, 0).unwrap();
            assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
            assert_eq!(read(&vm, value, b"origin", 3), pose);
            assert_eq!(calls.services.server.area.bounds(value_id), bounds);
            let order = solid_order(calls);
            assert!(
                order.iter().position(|id| *id == value_id).unwrap()
                    > order.iter().position(|id| *id == peer_id).unwrap()
            );
            assert_eq!(
                vm.entities.read(u64::from(peer), vm.stride).unwrap(),
                peer_bytes
            );
        });
    }
}

#[test]
fn droptofloor_miss_and_allsolid_preserve_native_row_common_body_and_links() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -512.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for boxes in [&[][..], &[floor][..]] {
            with_collision_services(native, None, boxes, |calls| {
                let mut vm = seeded_vm(calls, 8);
                let value = spawn_box(calls, &mut vm, [0.0, 0.0, -20.0], [-1.0; 3], [1.0; 3]);
                write(&mut vm, value, b"flags", &[81.5f32.to_bits()]);
                write(&mut vm, value, b"groundentity", &[value]);
                vm.image.globals[34] = value;
                let id = entity(calls, &vm, value);
                let bytes = vm
                    .entities
                    .read(u64::from(value), vm.stride)
                    .unwrap()
                    .to_vec();
                let body = calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize);
                let bounds = calls.services.server.area.bounds(id);
                let order = solid_order(calls);
                calls.call(&mut vm, 34, 0).unwrap();
                assert_eq!(vm.image.globals[1], 0.0f32.to_bits());
                assert_eq!(
                    vm.entities.read(u64::from(value), vm.stride).unwrap(),
                    bytes
                );
                assert_eq!(
                    body_bits(
                        calls
                            .services
                            .server
                            .entities
                            .columns
                            .body(id.slot as usize)
                    ),
                    body_bits(body)
                );
                assert_eq!(calls.services.server.area.bounds(id), bounds);
                assert_eq!(solid_order(calls), order);
            });
        }
    }
}

#[test]
fn droptofloor_accepts_startsolid_when_trace_reaches_an_actual_floor() {
    let boxes = [
        Bounds {
            mins: Vec3([-100.0, -100.0, -512.0]),
            maxs: Vec3([100.0, 100.0, 0.0]),
        },
        Bounds {
            mins: Vec3([-100.0, -100.0, 31.0]),
            maxs: Vec3([100.0, 100.0, 33.0]),
        },
    ];
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_collision_services(native, None, &boxes, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
            vm.image.globals[34] = value;
            let (rules, entity_rules) =
                qa_world::collision::trace_policy(calls.context.trace_rules);
            let mut query = qa_world::collision::TraceQuery::point(
                Vec3([0.0, 0.0, 32.0]),
                Vec3([0.0, 0.0, -224.0]),
                rules,
                entity_rules.with_quake_kind(Some(qa_world::collision::QuakeTraceKind::Normal)),
            );
            query.mins = Vec3([-1.0; 3]);
            query.maxs = Vec3([1.0; 3]);
            query.pass = CollisionOwner::Lifetime(entity(calls, &vm, value));
            let trace = (ENGINE_CALLS.trace)(calls.services, query, None).unwrap();
            assert!(trace.start_solid && !trace.all_solid && trace.fraction < 1.0);
            calls.call(&mut vm, 34, 0).unwrap();
            assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
            assert_eq!(
                read(&vm, value, b"origin", 3),
                trace.end.0.map(f32::to_bits)
            );
            assert_eq!(read(&vm, value, b"groundentity", 1), [0]);
        });
    }
}

#[test]
fn droptofloor_rejects_only_executed_arity_or_required_field_paths_without_partial_relink() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -512.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for missing in [
        b"groundentity".as_slice(),
        b"origin",
        b"mins",
        b"maxs",
        b"flags",
        b"absmin",
        b"absmax",
        b"angles",
        b"solid",
        b"owner",
    ] {
        with_collision_services(RuleSetId::Quake, None, &[floor], |calls| {
            let fields = FIELDS
                .iter()
                .copied()
                .map(|mut definition| {
                    if definition.2 == missing {
                        definition.0 = 1;
                    }
                    definition
                })
                .collect::<Vec<_>>();
            let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
            // Loading unrelated bytecode and the ordinary spawn path still
            // work. Only the builtin's executed field path rejects.
            assert!(vm.call(calls, 1, 100, false).is_ok());
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            write(
                &mut vm,
                value,
                b"origin",
                &[0.0f32, 0.0, 32.0].map(f32::to_bits),
            );
            write(&mut vm, value, b"mins", &[-1.0f32; 3].map(f32::to_bits));
            write(&mut vm, value, b"maxs", &[1.0f32; 3].map(f32::to_bits));
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            vm.image.globals[34] = value;
            let id = entity(calls, &vm, value);
            let bytes = vm
                .entities
                .read(u64::from(value), vm.stride)
                .unwrap()
                .to_vec();
            let body = calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize);
            assert_eq!(calls.call(&mut vm, 34, 0), Err(Trap::Builtin));
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
            assert_eq!(
                body_bits(
                    calls
                        .services
                        .server
                        .entities
                        .columns
                        .body(id.slot as usize)
                ),
                body_bits(body)
            );
            assert_eq!(calls.services.server.area.bounds(id), None);
        });
    }
    with_services(RuleSetId::Quake, |calls| {
        let fields = FIELDS
            .iter()
            .copied()
            .filter(|field| field.2 != b"groundentity")
            .collect::<Vec<_>>();
        let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
        let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
        vm.image.globals[34] = value;
        calls.call(&mut vm, 34, 0).unwrap(); // Miss never reads groundentity.
        assert_eq!(vm.image.globals[1], 0);
        let bytes = vm
            .entities
            .read(u64::from(value), vm.stride)
            .unwrap()
            .to_vec();
        assert_eq!(calls.call(&mut vm, 34, 1), Err(Trap::Builtin));
        assert_eq!(
            vm.entities.read(u64::from(value), vm.stride).unwrap(),
            bytes
        );
    });
    with_services(RuleSetId::Quake, |calls| {
        let globals = GLOBALS
            .iter()
            .copied()
            .map(|mut definition| {
                if definition.2 == b"self" {
                    definition.0 = 2;
                }
                definition
            })
            .collect::<Vec<_>>();
        let mut vm = seed_vm(calls, vm_with_definitions(8, &globals, FIELDS));
        assert!(vm.call(calls, 1, 100, false).is_ok());
        assert_eq!(calls.call(&mut vm, 34, 0), Err(Trap::Builtin));
    });
}

#[test]
fn droptofloor_finishes_native_candidate_validation_before_mutating_self() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let floor = spawn_box(
            calls,
            &mut vm,
            [0.0; 3],
            [-16.0, -16.0, -8.0],
            [16.0, 16.0, 0.0],
        );
        let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
        // This selected floor's live owner is malformed after its old area
        // membership was published. The view captures the boundary error.
        write(&mut vm, floor, b"owner", &[1]);
        vm.image.globals[34] = value;
        let bytes = vm
            .entities
            .read(u64::from(value), vm.stride)
            .unwrap()
            .to_vec();
        let id = entity(calls, &vm, value);
        let body = calls
            .services
            .server
            .entities
            .columns
            .body(id.slot as usize);
        let bounds = calls.services.server.area.bounds(id);
        let order = solid_order(calls);
        assert_eq!(calls.call(&mut vm, 34, 0), Err(Trap::Builtin));
        assert_eq!(
            vm.entities.read(u64::from(value), vm.stride).unwrap(),
            bytes
        );
        assert_eq!(
            body_bits(
                calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize)
            ),
            body_bits(body)
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        assert_eq!(solid_order(calls), order);
    });
}

#[test]
fn quakec_trace_role_controls_epsilon_and_linked_merge_independently_of_console_source() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let front = vm.image.globals[1];
        write(&mut vm, front, b"solid", &[2.0f32.to_bits()]);
        let mins = [-1.0f32; 3].map(f32::to_bits);
        let maxs = [1.0f32; 3].map(f32::to_bits);
        parameters(&mut vm, &[&[front], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        let origin = [12.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&[front], &origin]);
        calls.call(&mut vm, 2, 2).unwrap();
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        let mut fractions = [0; 2];
        for (index, trace_rules) in [RuleSetId::Quake, RuleSetId::Quake3]
            .into_iter()
            .enumerate()
        {
            calls.context.trace_rules = trace_rules;
            for console_source in [RuleSetId::Quake, RuleSetId::Quake3] {
                calls.context.console.source = console_source;
                parameters(&mut vm, &[&start, &end, &[0], &[0]]);
                calls.call(&mut vm, 16, 4).unwrap();
                let epsilon = if index == 0 { 0.03125 } else { 0.125 };
                let expected = ((11.0 - epsilon) / 20.0) as f32;
                // Box-hull recursion composes near-side fractions. Its native
                // f32 rounding can differ from this single-plane estimate.
                assert!((f32::from_bits(vm.image.globals[62]) - expected).abs() <= f32::EPSILON);
                assert_eq!(vm.image.globals[72], front);
                if console_source == RuleSetId::Quake {
                    fractions[index] = vm.image.globals[62];
                } else {
                    assert_eq!(vm.image.globals[62], fractions[index]);
                }
            }
        }
        assert!(f32::from_bits(fractions[1]) < f32::from_bits(fractions[0]));

        calls.call(&mut vm, 14, 0).unwrap();
        let containing = vm.image.globals[1];
        write(&mut vm, containing, b"solid", &[2.0f32.to_bits()]);
        parameters(&mut vm, &[&[containing], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        parameters(&mut vm, &[&[containing], &start]);
        calls.call(&mut vm, 2, 2).unwrap();
        for (index, trace_rules) in [RuleSetId::Quake, RuleSetId::Quake3]
            .into_iter()
            .enumerate()
        {
            calls.context.trace_rules = trace_rules;
            for console_source in [RuleSetId::QuakeWorld, RuleSetId::Quake2Rerelease] {
                calls.context.console.source = console_source;
                parameters(&mut vm, &[&start, &end, &[0], &[0]]);
                calls.call(&mut vm, 16, 4).unwrap();
                assert_eq!(vm.image.globals[61], 1.0f32.to_bits());
                assert_eq!(vm.image.globals[60], 0);
                assert_eq!(
                    vm.image.globals[62],
                    if index == 0 {
                        1.0f32.to_bits()
                    } else {
                        fractions[1]
                    }
                );
                assert_eq!(
                    vm.image.globals[72],
                    if index == 0 { containing } else { front }
                );
            }
        }
        let slot = entity(calls, &vm, containing).slot as usize;
        assert_eq!(
            calls.services.server.entities.columns.appearance[slot].source,
            RuleSetId::Quake
        );
    });
}

#[test]
fn quakec_native_print_signature_does_not_follow_the_console_view() {
    use qa_core::events::{FrameEvent, OutputSubmission};
    for native_rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(native_rules, |calls| {
            let mut vm = seeded_vm(calls, 4);
            let text = vm.insert_string(b"native ABI unchanged\xff").unwrap() as u32;
            for console_source in [RuleSetId::Quake3, RuleSetId::Quake2Rerelease] {
                calls.context.console.source = console_source;
                if native_rules == RuleSetId::QuakeWorld {
                    parameters(&mut vm, &[&[3.0f32.to_bits()], &[text]]);
                    calls.call(&mut vm, 23, 2).unwrap();
                } else {
                    parameters(&mut vm, &[&[text]]);
                    calls.call(&mut vm, 23, 1).unwrap();
                }
                let ring = &mut calls.services.server.events;
                let consumer = calls.services.server.presentation;
                let mut batch = ring.batch(consumer).unwrap();
                let record = ring.next(&mut batch).unwrap();
                let FrameEvent::Print(print) = record.event else {
                    panic!("expected print")
                };
                assert_eq!(
                    ring.texts.get(print.text),
                    Some(&b"native ABI unchanged\xff"[..])
                );
                assert_eq!(
                    print.kind,
                    if native_rules == RuleSetId::QuakeWorld {
                        PrintKind::Chat
                    } else {
                        PrintKind::Console
                    }
                );
                assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
            }
        });
    }
}

#[test]
fn spatial_queries_read_storep_without_publishing_unrelated_dirty_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        let id = entity(calls, &vm, value);
        calls.call(&mut vm, 14, 0).unwrap();
        let untouched = vm.image.globals[1];
        // Direct host edits are synchronized by the export scan. They must not
        // make every spatial query inspect a different, untouched ABI row.
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        // Executed StorePV makes this unlinked row malformed. Trace must not
        // validate its bounds merely because it has a dirty publication mark.
        store_and_trace(calls, &mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]).unwrap();
        assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
        store_and_trace(calls, &mut vm, value, b"origin", &[12.0f32.to_bits(), 0, 0]).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3::default()
        );
        calls.call(&mut vm, 16, 4).unwrap();
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Entity)
        );
        write(&mut vm, untouched, b"mins", &[0; 3]);
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([12.0, 0.0, 0.0])
        );
        vm.project_entities(calls.services, calls.context).unwrap();
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        vm.image.globals[34] = value;
        vm.image.globals[35] = 3.0f32.to_bits();
        vm.image.globals[28] = 37.0f32.to_bits();
        vm.image.globals[29] = 0x0100_0001;
        vm.image.globals[32] = 5;
        vm.call(calls, 4, 100, false).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.appearance[id.slot as usize].frame,
            37
        );
        assert_eq!(read(&vm, value, b"think", 1), [0x0100_0001]);
        assert_eq!(
            read(&vm, value, b"nextthink", 1),
            [((3.0f64 + 0.1) as f32).to_bits()]
        );
        vm.image.globals[28] = 41.0f32.to_bits();
        assert_eq!(vm.call(calls, 8, 2, false), Err(Trap::Budget));
        calls.call(&mut vm, 16, 4).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.appearance[id.slot as usize].frame,
            41
        );
    });
}

#[test]
fn pointer_stores_mark_crossed_rows_and_flush_the_current_reused_lifetime() {
    with_services(RuleSetId::Quake, |calls| {
        calls.context.clock = ThinkTime::Seconds(1.0);
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        let old = entity(calls, &vm, first);
        calls.call(&mut vm, 14, 0).unwrap();
        let second = vm.image.globals[1];
        let second_id = entity(calls, &vm, second);
        assert_eq!(second as usize, first as usize + vm.stride);
        write(&mut vm, second, b"origin", &[15.0f32.to_bits(), 0, 0]);
        vm.image.globals[28..31].fill(0);
        vm.image.globals[31] = first + vm.stride as u32 - 4;
        vm.image.globals[32] = 5;
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        vm.call(calls, 2, 100, false).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[second_id.slot as usize],
            Vec3([15.0, 0.0, 0.0])
        );
        write(&mut vm, second, b"origin", &[17.0f32.to_bits(), 0, 0]);
        // Native pointers can be unaligned. This single word touches both rows.
        vm.image.globals[28] = 0;
        vm.image.globals[31] = first + vm.stride as u32 - 2;
        vm.call(calls, 3, 100, false).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[second_id.slot as usize],
            Vec3([17.0, 0.0, 0.0])
        );
        vm.image.globals[28] = 11.0f32.to_bits();
        vm.image.globals[31] = vm.field_address(first, 0, 1).unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        parameters(&mut vm, &[&[first]]);
        calls.call(&mut vm, 15, 1).unwrap();
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.image.globals[1], first);
        let new = entity(calls, &vm, first);
        assert_ne!(new, old);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        calls.call(&mut vm, 16, 4).unwrap();
        assert!(calls.services.server.entities.resolve(old).is_none());
        assert_eq!(
            calls.services.server.entities.columns.position[new.slot as usize],
            Vec3([0.0; 3])
        );
    });
}

#[test]
fn rejected_dirty_import_keeps_bounded_membership_until_it_can_retry() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 4);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        let first_id = entity(calls, &vm, first);
        calls.call(&mut vm, 14, 0).unwrap();
        let second = vm.image.globals[1];
        let second_id = entity(calls, &vm, second);
        write(&mut vm, second, b"mins", &[4.0f32.to_bits(); 3]);
        vm.image.globals[28] = 0;
        vm.image.globals[31] = vm.field_address(first, 0, 1).unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        vm.image.globals[28] = 9.0f32.to_bits();
        vm.image.globals[31] = vm
            .field_address(second, vm.image.field(b"frame").unwrap() as u32, 1)
            .unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        for x in 1..=16 {
            store_and_trace(
                calls,
                &mut vm,
                first,
                b"origin",
                &[(x as f32).to_bits(), 0, 0],
            )
            .unwrap();
            assert_eq!(
                vm.project_dirty_entities(calls.services, calls.context),
                Err(CallError::Entity)
            );
        }
        write(&mut vm, second, b"mins", &[0; 3]);
        calls.call(&mut vm, 16, 4).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[first_id.slot as usize],
            Vec3([16.0, 0.0, 0.0])
        );
        assert_eq!(
            calls.services.server.entities.columns.appearance[second_id.slot as usize].frame,
            9
        );
    });
}

#[test]
fn full_qw_reuses_exact_last_ordinal_and_stale_free_writes_do_not_relink() {
    with_services(RuleSetId::QuakeWorld, |calls| {
        let mut vm = seeded_vm(calls, 4);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        let first_id = entity(calls, &vm, first);
        calls.call(&mut vm, 14, 0).unwrap();
        let last = vm.image.globals[1];
        let last_id = entity(calls, &vm, last);
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.image.globals[1], last);
        assert!(calls.services.server.entities.resolve(last_id).is_none());
        assert!(calls.services.server.entities.resolve(first_id).is_some());
        let replacement = entity(calls, &vm, last);
        assert_eq!(replacement.slot, last_id.slot);
        assert_eq!(replacement.generation, last_id.generation + 1);
        parameters(&mut vm, &[&[last]]);
        calls.call(&mut vm, 15, 1).unwrap();
        let relinks = calls.services.server.area.relinks;
        let origin = [9.0f32, 8.0, 7.0].map(f32::to_bits);
        parameters(&mut vm, &[&[last], &origin]);
        calls.call(&mut vm, 2, 2).unwrap();
        assert_eq!(read(&vm, last, b"origin", 3), origin);
        assert_eq!(calls.services.server.area.relinks, relinks);
        assert!(
            calls
                .services
                .server
                .entities
                .resolve(replacement)
                .is_none()
        );
    });
}

#[test]
fn grace_reuse_and_invalid_operand_traps_do_not_create_lifetimes() {
    with_services(RuleSetId::Quake, |calls| {
        calls.context.clock = ThinkTime::Seconds(1.0);
        let mut vm = seeded_vm(calls, 4);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        parameters(&mut vm, &[&[value]]);
        calls.call(&mut vm, 15, 1).unwrap();
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.image.globals[1], value);
        let live = calls.services.server.entities.active().count();
        let origin = [0u32; 3];
        for invalid in [1u32, vm.entities.len() as u32, u32::MAX] {
            parameters(&mut vm, &[&[invalid], &origin]);
            assert_eq!(calls.call(&mut vm, 2, 2), Err(Trap::Builtin));
        }
        assert_eq!(calls.call(&mut vm, 2, 1), Err(Trap::Builtin));
        assert_eq!(calls.services.server.entities.active().count(), live);
    });
}

#[test]
fn vm_rejects_zero_stride_before_entity_number_or_builtin_resolution() {
    let bytes = quakec_program::program(&[(Opcode::Return, 1, 0, 0)], &[0, 1]);
    let mut image = Image::parse(&bytes, Some(5927)).unwrap();
    image.entityfields = 0;
    assert!(matches!(
        Vm::load(
            image,
            Layout {
                entities: 1,
                header_bytes: 0,
                extra_string_bytes: 0,
                state_step: 0.1,
                random_seed: 1,
            }
        ),
        Err(Trap::Memory)
    ));
}

#[test]
fn native_staging_dense_sounds_reject_the_whole_group_without_resync() {
    use qa_core::events::{FrameEvent, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let sound = bind_native_message_sound(calls);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let mut wire = vec![6; NATIVE_WIRE_BYTES];
            for cue in wire[..(NATIVE_WIRE_BYTES / 3) * 3].chunks_exact_mut(3) {
                cue.copy_from_slice(&[9, 0, 1]);
            }
            stage_native_message(calls, &wire);
            assert_eq!(
                (ENGINE_CALLS.message_bits)(calls.services, ModuleId(1), 6, 8),
                Err(CallError::Capacity)
            );
            let before = calls.services.server.events.sequence_range();
            let counters = calls.services.server.events.counters(consumer).unwrap();
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(rules),
                    calls.context.sound_policy
                ),
                Err(CallError::Capacity)
            );
            let ring = &calls.services.server.events;
            assert_eq!(ring.sequence_range(), before);
            assert_eq!(ring.counters(consumer), Some(counters));
            assert!(!ring.needs_resync(consumer));
            assert_eq!(ring.rejected_publications(), 1);
            // The failed emission consumed staging and cannot retry a prefix.
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(rules),
                    calls.context.sound_policy
                )
                .unwrap(),
                None
            );
            stage_native_message(calls, &[9, 0, 1]);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            let ring = &calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            assert!(
                matches!(ring.next(&mut batch).unwrap().event, FrameEvent::Sound(e) if e.sound == sound && e.native_fields.unwrap().flags == 0)
            );
            assert!(ring.next(&mut batch).is_none());
        });
    }
}
fn native_message_delivery(rules: RuleSetId) -> NativeDelivery {
    NativeDelivery {
        module: ModuleId(1),
        source: rules,
        target: NativeTarget::All,
        origin: Vec3::default(),
        reliable: true,
        dupe_key: 0,
        component: 0,
        emission: None,
        check_portals: false,
    }
}
fn stage_native_message(calls: &mut QuakeCCalls<'_, '_>, bytes: &[u8]) {
    for &byte in bytes {
        (ENGINE_CALLS.message_bits)(calls.services, ModuleId(1), u32::from(byte), 8).unwrap();
    }
}

#[test]
fn staged_native_particles_share_the_atomic_print_group_and_catalog_binding() {
    use qa_core::events::{FrameEvent, OutputSubmission};
    with_services(RuleSetId::Quake, |calls| {
        let client = calls
            .services
            .server
            .connect(qa_session::clients::Connection::Remote, ModuleId(2), None)
            .unwrap();
        let consumer = calls.services.server.clients[client.0 as usize]
            .output
            .unwrap();
        // svc_print, svc_particle with signed coordinate/direction bytes,
        // then another print. Each service retains source order and offset.
        let wire = [
            8, b'p', 0, 18, 0xfe, 0xff, 0x11, 0, 0, 0, 0x80, 0x7f, 0xff, 255, 7, 8, b'q', 0,
        ];
        stage_native_message(calls, &wire);
        let mut delivery = native_message_delivery(RuleSetId::Quake);
        delivery.target = NativeTarget::Client(client);
        delivery.reliable = false;
        (ENGINE_CALLS.native_message)(calls.services, delivery, calls.context.sound_policy)
            .unwrap();
        let binding = calls
            .services
            .registry
            .effects
            .native_particles(RuleSetId::Quake)
            .unwrap();
        let ring = &mut calls.services.server.events;
        let mut batch = ring.batch(consumer).unwrap();
        for (component, text) in [(0, Some(b'p')), (3, None), (15, Some(b'q'))] {
            let record = ring.next(&mut batch).unwrap();
            assert_eq!(record.delivery.unwrap().component, component);
            assert_eq!(
                record.delivery.unwrap().target,
                NativeTarget::Client(client)
            );
            match record.event {
                FrameEvent::Effect(event) => {
                    let expected = binding.event(
                        Vec3([-0.25, 2.125, 0.0]),
                        Vec3([-8.0, 7.9375, -0.0625]),
                        7,
                        255,
                    );
                    assert_eq!(event.effect, expected.effect);
                    assert_eq!(event.count, 1024);
                    assert_eq!(event.position, expected.position);
                    assert_eq!(event.direction, expected.direction);
                    assert_eq!(event.color, 7);
                    assert_eq!((event.client, event.client_lifetime), (None, None));
                    assert_eq!(event.time, EventTime(0));
                }
                FrameEvent::Print(print) => {
                    let PrintText::Native {
                        wire, text: span, ..
                    } = print.text
                    else {
                        panic!("native print")
                    };
                    let mut scratch = [0; 8];
                    let bytes = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                    assert_eq!(&bytes[span.range()], &[text.unwrap()]);
                }
                _ => panic!("recognized particle must have one common owner"),
            }
            assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
        }
        assert!(ring.next(&mut batch).is_none());
        assert!(ring.is_empty());
        assert_eq!(
            (ENGINE_CALLS.native_message)(calls.services, delivery, calls.context.sound_policy)
                .unwrap(),
            None
        );
    });
}

#[test]
fn malformed_or_overfull_particle_staging_publishes_no_prefix_and_consumes_once() {
    use qa_core::events::OutputTarget;
    with_services(RuleSetId::Quake, |calls| {
        let consumer = calls
            .services
            .server
            .events
            .bind(OutputTarget::Client(ClientId(0)))
            .unwrap();
        let particle = [18, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 7];
        for length in 1..particle.len() {
            stage_native_message(calls, &[8, b'p', 0]);
            stage_native_message(calls, &particle[..length]);
            let before = calls.services.server.events.sequence_range();
            let counters = calls.services.server.events.counters(consumer);
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(RuleSetId::Quake),
                    calls.context.sound_policy,
                ),
                Err(CallError::Memory)
            );
            assert_eq!(calls.services.server.events.sequence_range(), before);
            assert_eq!(calls.services.server.events.counters(consumer), counters);
            assert!(!calls.services.server.events.needs_resync(consumer));
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(RuleSetId::Quake),
                    calls.context.sound_policy,
                )
                .unwrap(),
                None
            );
        }
        stage_native_message(calls, &particle);
        for _ in 0..calls.services.server.events.capacity() {
            stage_native_message(calls, &[1]);
        }
        let before = calls.services.server.events.sequence_range();
        assert_eq!(
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(RuleSetId::Quake),
                calls.context.sound_policy,
            ),
            Err(CallError::Capacity)
        );
        assert_eq!(calls.services.server.events.sequence_range(), before);
        assert!(!calls.services.server.events.needs_resync(consumer));
        assert_eq!(
            calls.services.server.events.rejected_publications(),
            particle.len() as u64
        );
        assert_eq!(
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(RuleSetId::Quake),
                calls.context.sound_policy,
            )
            .unwrap(),
            None
        );
    });
}

fn bind_native_message_sound(calls: &mut QuakeCCalls<'_, '_>) -> SoundId {
    let rate = std::num::NonZeroU32::new(44100).unwrap();
    let bank = std::sync::Arc::new(
        qa_audio::Bank::load(
            rate,
            vec![(
                "sound/test.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![100; 4],
                    loop_start: None,
                    loop_end: None,
                    loop_invalid: false,
                },
            )],
        )
        .unwrap(),
    );
    let sound = bank.find("sound/test.wav").unwrap();
    let sounds = calls.services.storage.resources(ModuleId(1)).unwrap()[1];
    (ENGINE_CALLS.configstring)(
        calls.services,
        ModuleId(1),
        sounds.first as usize + 1,
        b"test.wav",
    )
    .unwrap();
    calls
        .services
        .storage
        .bind_sound_bank(
            ModuleId(1),
            sounds,
            bank.as_ref(),
            Some(qa_audio::SoundSource::DEFAULT),
        )
        .unwrap();
    sound
}
#[test]
fn native_mixed_print_and_sound_components_use_one_ring_in_source_order() {
    use qa_core::events::{FrameEvent, OutputSubmission, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let sound = bind_native_message_sound(calls);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let bytes = [10, 2, b'p', 0, 9, 3, 1, 255, 64, 9, 0, 1, 10, 2, b'q', 0];
            for _ in 0..2 {
                stage_native_message(calls, &bytes);
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(rules),
                    calls.context.sound_policy,
                )
                .unwrap();
            }
            // Later config mutation cannot replace the captured PCM handle.
            let sounds = calls.services.storage.resources(ModuleId(1)).unwrap()[1];
            (ENGINE_CALLS.configstring)(
                calls.services,
                ModuleId(1),
                sounds.first as usize + 1,
                b"missing.wav",
            )
            .unwrap();
            let ring = &mut calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            for _ in 0..2 {
                for (component, expected_flags, expected_text) in [
                    (0, None, Some(&bytes[..4])),
                    (4, Some(3), None),
                    (9, Some(0), None),
                    (12, None, Some(&bytes[12..])),
                ] {
                    let record = ring.next(&mut batch).unwrap();
                    assert_eq!(record.delivery.unwrap().component, component);
                    match record.event {
                        FrameEvent::Sound(event) => {
                            assert_eq!(event.sound, sound);
                            assert_eq!(event.native_fields.unwrap().flags, expected_flags.unwrap());
                            assert_eq!(event.spatial, calls.context.sound_policy);
                        }
                        FrameEvent::Print(print) => {
                            let qa_core::primitives::PrintText::Native { wire, text, .. } =
                                print.text
                            else {
                                panic!("native print must retain the original wire pages")
                            };
                            let mut scratch = [0; 64];
                            let bytes = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                            assert_eq!(bytes, expected_text.unwrap());
                            assert_eq!(&bytes[text.range()], &expected_text.unwrap()[2..3]);
                            assert_eq!((print.kind, print.level), (PrintKind::Console, 2));
                        }
                        _ => panic!("unexpected component"),
                    }
                    assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
                }
            }
            assert!(ring.next(&mut batch).is_none());
            assert!(ring.is_empty());
            assert_eq!(ring.rejected_publications(), 0);
        });
    }
}

#[test]
fn native_print_retains_long_rr_text_and_opaque_classic_continuation_once() {
    use qa_core::events::{FrameEvent, OutputTarget};
    use qa_core::primitives::PrintText;
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let mut bytes = vec![10, 0];
            bytes.extend(std::iter::repeat_n(
                b'x',
                if rules == RuleSetId::Quake2 {
                    2050
                } else {
                    9000
                },
            ));
            bytes.push(0);
            bytes.extend_from_slice(b"\x0fnext\0");
            stage_native_message(calls, &bytes);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            let ring = &calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            let record = ring.next(&mut batch).unwrap();
            let FrameEvent::Print(print) = record.event else {
                panic!("typed native print")
            };
            let PrintText::Native {
                wire,
                text,
                consumed,
            } = print.text
            else {
                panic!("one raw owner")
            };
            let mut scratch = vec![0; bytes.len()];
            let raw = ring.texts.copy_payload(wire, &mut scratch).unwrap();
            if rules == RuleSetId::Quake2 {
                assert_eq!(raw, bytes);
                assert_eq!(consumed, 2049);
                assert_eq!(text.count, 2047);
                assert!(
                    ring.next(&mut batch).is_none(),
                    "unknown tail must not be independently sent after an oversized prefix"
                );
            } else {
                assert_eq!(raw, &bytes[..9003]);
                assert_eq!(consumed, 9003);
                assert_eq!(text.count, 9000);
                let FrameEvent::Print(after) = ring.next(&mut batch).unwrap().event else {
                    panic!("later RR print")
                };
                let PrintText::Native { wire, text, .. } = after.text else {
                    panic!("typed next")
                };
                let mut scratch = [0; 16];
                let raw = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                assert_eq!(&raw[text.range()], b"next");
                assert!(ring.next(&mut batch).is_none());
            }
            assert_eq!((print.kind, print.level), (PrintKind::Notify, 0));
            assert_eq!(ring.texts.truncated(), 0);
        });
    }
    with_services(RuleSetId::Quake2, |calls| {
        let consumer = calls
            .services
            .server
            .events
            .bind(OutputTarget::Client(ClientId(0)))
            .unwrap();
        let bytes = b"\x0a\x01a\x80\xff\0opaque";
        stage_native_message(calls, bytes);
        (ENGINE_CALLS.native_message)(
            calls.services,
            native_message_delivery(RuleSetId::Quake2),
            calls.context.sound_policy,
        )
        .unwrap();
        let ring = &calls.services.server.events;
        let mut batch = ring.batch(consumer).unwrap();
        let FrameEvent::Print(print) = ring.next(&mut batch).unwrap().event else {
            panic!("print")
        };
        let PrintText::Native {
            wire,
            text,
            consumed,
        } = print.text
        else {
            panic!("raw native print")
        };
        let mut scratch = [0; 64];
        let raw = ring.texts.copy_payload(wire, &mut scratch).unwrap();
        assert_eq!(raw, bytes);
        assert_eq!(&raw[text.range()], b"a\x80");
        assert_eq!(consumed, 5);
        assert!(ring.next(&mut batch).is_none());
    });
}

#[test]
fn native_sound_captures_the_emitter_lifetime_before_slot_reuse() {
    use qa_core::events::{FrameEvent, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let sound = bind_native_message_sound(calls);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let old = calls
                .services
                .server
                .entities
                .allocate(
                    ThinkTime::Seconds(0.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            let native = NativeEntity {
                module: ModuleId(1),
                slot: 7,
            };
            calls.services.server.entities.columns.native_entity[old.slot as usize] = Some(native);
            stage_native_message(calls, &[9, 8, 1, 58, 0]);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            assert!(
                calls
                    .services
                    .server
                    .entities
                    .release(old, ThinkTime::Seconds(1.0))
            );
            let replacement = calls
                .services
                .server
                .entities
                .allocate(
                    ThinkTime::Seconds(1000.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            assert_eq!(replacement.slot, old.slot);
            assert_ne!(replacement.generation, old.generation);
            calls.services.server.entities.columns.native_entity[replacement.slot as usize] =
                Some(native);
            let ring = &calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            let FrameEvent::Sound(event) = ring.next(&mut batch).unwrap().event else {
                panic!("missing sound")
            };
            assert_eq!(event.sound, sound);
            assert_eq!(event.entity, Some(SoundEmitter::Entity(old)));
            assert_eq!(event.native_fields.unwrap().entity, 7);
            assert_eq!(event.source_ordinal, Some(1));
            assert!(ring.next(&mut batch).is_none());
        });
    }
}

#[test]
fn unresolved_native_sound_preserves_sibling_print_runs() {
    use qa_core::events::{FrameEvent, OutputSubmission, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let bytes = [10, 2, b'p', 0, 9, 0, 1, 10, 2, b'q', 0];
            stage_native_message(calls, &bytes);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            let ring = &mut calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            for (component, expected) in [(0, &bytes[..4]), (7, &bytes[7..])] {
                let record = ring.next(&mut batch).unwrap();
                assert_eq!(record.delivery.unwrap().component, component);
                let FrameEvent::Print(print) = record.event else {
                    panic!("unresolved sound must not publish")
                };
                let qa_core::primitives::PrintText::Native { wire, text, .. } = print.text else {
                    panic!("native print storage")
                };
                let mut scratch = [0; 32];
                let bytes = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                assert_eq!(bytes, expected);
                assert_eq!(&bytes[text.range()], &expected[2..3]);
                assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
            }
            assert!(ring.next(&mut batch).is_none());
            assert!(ring.is_empty());
            assert!(!ring.needs_resync(consumer));
            assert_eq!(ring.rejected_publications(), 1);
        });
    }
}

#[test]
fn point_contents_reads_live_candidates_without_flushing_unrelated_dirty_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        let id = entity(calls, &vm, value);
        write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
        let mins = [-2.0f32; 3].map(f32::to_bits);
        let maxs = [2.0f32; 3].map(f32::to_bits);
        parameters(&mut vm, &[&[value], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        let bounds = calls.services.server.area.bounds(id);
        // A different unlinked, untouched row is malformed but not scanned.
        calls.call(&mut vm, 14, 0).unwrap();
        let untouched = vm.image.globals[1];
        // Spawn's entvars clear is a tracked write. Publish that valid clear
        // before introducing the unrelated, untracked malformed host edit.
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        parameters(&mut vm, &[&[0; 3]]);
        vm.image.globals[28..31].copy_from_slice(&[12.0f32.to_bits(), 0, 0]);
        vm.image.globals[31] = vm
            .field_address(value, vm.image.field(b"origin").unwrap() as u32, 3)
            .unwrap() as u32;
        vm.image.globals[32] = 10;
        vm.call(calls, 9, 100, false).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3::default()
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // Original Q1 pointcontents remains world-only despite the live row.
        assert_eq!(vm.image.globals[1], (-1.0f32).to_bits());
        // Publication retains its existing explicit boundary; the query did
        // not consume the dirty mark or change the area membership.
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([12.0, 0.0, 0.0])
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // A real bytecode store makes an unrelated unlinked row malformed.
        // Stock PointContents does not inspect or publish any entity row.
        vm.image.globals[28..31].copy_from_slice(&[4.0f32.to_bits(); 3]);
        vm.image.globals[31] = vm
            .field_address(untouched, vm.image.field(b"mins").unwrap() as u32, 3)
            .unwrap() as u32;
        vm.call(calls, 9, 100, false).unwrap();
        assert_eq!(vm.image.globals[1], (-1.0f32).to_bits());
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // A combined caller selects linked contents independently of the QC
        // source. Raw host/native bytes have no dirty mark: the candidate-local
        // view must decode this linked row without importing unrelated rows.
        calls.context.trace_rules = RuleSetId::Quake2;
        write(&mut vm, value, b"origin", &[0; 3]);
        write(&mut vm, value, b"mins", &[4.0f32.to_bits(); 3]);
        parameters(&mut vm, &[&[0; 3]]);
        assert_eq!(calls.call(&mut vm, 41, 1), Err(Trap::Builtin));
        assert_eq!(
            calls.services.server.entities.columns.mins[id.slot as usize],
            Vec3([-2.0; 3])
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // The caller's linked-content policy uses the old solid chain even
        // after the QC row becomes nonsolid. Bounds remain executed fields.
        write(&mut vm, value, b"solid", &[0]);
        write(&mut vm, value, b"owner", &[1]);
        assert_eq!(calls.call(&mut vm, 41, 1), Err(Trap::Builtin));
        write(&mut vm, value, b"mins", &[(-2.0f32).to_bits(); 3]);
        // Native temporary boxes do not use angles or owner for contents.
        write(&mut vm, value, b"angles", &[f32::NAN.to_bits(); 3]);
        calls.call(&mut vm, 41, 1).unwrap();
        assert_eq!(vm.image.globals[1], (-2.0f32).to_bits());
        assert_eq!(calls.services.server.area.bounds(id), bounds);
    });
}

#[test]
fn makestatic_copies_native_signon_values_then_frees_the_original_lifetime() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let anchor = entity(calls, &vm, 0);
            calls
                .services
                .storage
                .bind_lifetime(calls.context.module, anchor)
                .unwrap();
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let models = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[0];
            let model = (ENGINE_CALLS.resource_index)(
                calls.services,
                calls.context.module,
                models,
                b"progs/flame.mdl",
            )
            .unwrap();
            let name = vm.insert_string(b"progs/flame.mdl").unwrap() as u32;
            write(&mut vm, value, b"model", &[name]);
            write(&mut vm, value, b"modelindex", &[99.0f32.to_bits()]);
            write(
                &mut vm,
                value,
                b"origin",
                &[-0.2f32, -0.999, 4096.2].map(f32::to_bits),
            );
            write(
                &mut vm,
                value,
                b"angles",
                &[1.9f32, 180.0, -1.9].map(f32::to_bits),
            );
            write(&mut vm, value, b"frame", &[258.9f32.to_bits()]);
            write(&mut vm, value, b"skin", &[(-1.9f32).to_bits()]);
            write(&mut vm, value, b"colormap", &[513.9f32.to_bits()]);
            write(&mut vm, value, b"effects", &[999.0f32.to_bits()]);
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Remote,
                    calls.context.module,
                    None,
                )
                .unwrap();
            let consumer = calls.services.server.clients[client.0 as usize]
                .output
                .unwrap();
            parameters(&mut vm, &[&[value]]);
            calls.call(&mut vm, 69, 1).unwrap();
            assert!(calls.services.server.entities.resolve(id).is_none());
            assert!(calls.services.server.area.bounds(id).is_none());
            assert_eq!(read(&vm, value, b"model", 1), [0]);
            assert_eq!(read(&vm, value, b"nextthink", 1), [(-1.0f32).to_bits()]);
            assert_eq!(read(&vm, value, b"origin", 3), [0; 3]);
            let rate = std::num::NonZeroU32::new(44100).unwrap();
            let bank = std::sync::Arc::new(qa_audio::Bank::load(rate, Vec::new()).unwrap());
            let sounds = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[1];
            calls
                .services
                .storage
                .bind_sound_bank(
                    calls.context.module,
                    sounds,
                    bank.as_ref(),
                    Some(qa_audio::SoundSource::DEFAULT),
                )
                .unwrap();
            assert_eq!(
                calls.services.storage.publish_signon(
                    calls.services.server,
                    calls.context,
                    client,
                    &mut SignonCursor::default(),
                    Some(bank.as_ref())
                ),
                Ok(1)
            );
            let mut batch = calls.services.server.events.batch(consumer).unwrap();
            let record = calls.services.server.events.next(&mut batch).unwrap();
            let qa_core::events::FrameEvent::StaticEntity(record) = record.event else {
                panic!("static entity");
            };
            assert_eq!(record.position, Vec3([-0.125, -0.875, -4095.875]));
            assert_eq!(
                record.angles,
                if rules == RuleSetId::Quake {
                    Vec3([0.0, -180.0, 0.0])
                } else {
                    Vec3([1.40625, -180.0, -1.40625])
                }
            );
            assert_eq!(
                record.appearance.models[0],
                ModelRef::native(ResourceNamespace::Module(calls.context.module), model)
            );
            assert_eq!(
                (
                    record.appearance.frame,
                    record.appearance.skin,
                    record.appearance.native.colormap,
                    record.appearance.native.effects
                ),
                (2, 255, 1, 0)
            );
            calls.context.clock = ThinkTime::Seconds(4.0);
            let reused = vm.spawn_entity(calls.services, calls.context).unwrap();
            assert_eq!(reused, value);
            assert_ne!(entity(calls, &vm, reused), id);
            assert_eq!(record.position, Vec3([-0.125, -0.875, -4095.875]));
        });
    }
}

#[test]
fn makestatic_rejects_protected_and_malformed_removal_without_retained_ghosts() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let anchor = entity(calls, &vm, 0);
        calls
            .services
            .storage
            .bind_lifetime(calls.context.module, anchor)
            .unwrap();
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        calls.services.server.entities.set_never_free(id, true);
        let bytes = vm
            .entities
            .read(u64::from(value), vm.stride)
            .unwrap()
            .to_vec();
        assert_eq!(
            vm.make_static(calls.services, calls.context, value),
            Err(CallError::Entity)
        );
        assert_eq!(
            vm.make_static(calls.services, calls.context, 0),
            Err(CallError::Entity)
        );
        assert_eq!(
            vm.entities.read(u64::from(value), vm.stride).unwrap(),
            bytes
        );
        assert!(calls.services.server.entities.resolve(id).is_some());
        let rate = std::num::NonZeroU32::new(44100).unwrap();
        let bank = std::sync::Arc::new(qa_audio::Bank::load(rate, Vec::new()).unwrap());
        let sounds = calls
            .services
            .storage
            .resources(calls.context.module)
            .unwrap()[1];
        calls
            .services
            .storage
            .bind_sound_bank(
                calls.context.module,
                sounds,
                bank.as_ref(),
                Some(qa_audio::SoundSource::DEFAULT),
            )
            .unwrap();
        let client = calls
            .services
            .server
            .connect(
                qa_session::clients::Connection::Remote,
                ModuleId::default(),
                None,
            )
            .unwrap();
        assert_eq!(
            calls.services.storage.publish_signon(
                calls.services.server,
                calls.context,
                client,
                &mut SignonCursor::default(),
                Some(bank.as_ref())
            ),
            Ok(0)
        );
    });
    for field in [
        b"model".as_slice(),
        b"frame",
        b"skin",
        b"colormap",
        b"nextthink",
        b"origin",
        b"angles",
        b"solid",
        b"takedamage",
        b"modelindex",
    ] {
        with_services(RuleSetId::Quake, |calls| {
            let definitions: Vec<_> = FIELDS
                .iter()
                .copied()
                .filter(|(_, _, name)| *name != field)
                .collect();
            let image = vm_with_definitions(8, GLOBALS, &definitions);
            let mut vm = seed_vm(calls, image);
            let anchor = entity(calls, &vm, 0);
            calls
                .services
                .storage
                .bind_lifetime(calls.context.module, anchor)
                .unwrap();
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let bytes = vm
                .entities
                .read(u64::from(value), vm.stride)
                .unwrap()
                .to_vec();
            assert_eq!(
                vm.make_static(calls.services, calls.context, value),
                Err(CallError::Memory),
                "{field:?}"
            );
            assert!(calls.services.server.entities.resolve(id).is_some());
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
        });
    }
}

#[test]
fn makestatic_capacity_drop_still_releases_the_source_edict() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let anchor = entity(calls, &vm, 0);
        calls
            .services
            .storage
            .bind_lifetime(calls.context.module, anchor)
            .unwrap();
        for _ in 0..qa_compat::services::SIGNON_BYTES / 14 {
            (ENGINE_CALLS.static_entity)(calls.services, calls.context, EntityRecord::default())
                .unwrap();
        }
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        assert_eq!(vm.make_static(calls.services, calls.context, value), Ok(()));
        assert!(calls.services.server.entities.resolve(id).is_none());
        assert_eq!(calls.services.server.events.rejected_publications(), 1);
    });
}

#[test]
fn native_weapon_deadlines_publish_for_the_owning_client_with_independent_roles() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let max_clients = calls
                .services
                .cvars
                .bind("sv_maxclients", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(max_clients, "2").unwrap();
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Local,
                    calls.context.module,
                    Some(NativeEntity {
                        module: calls.context.module,
                        slot: 2,
                    }),
                )
                .unwrap();
            let mut fields = FIELDS.to_vec();
            fields.push((2, 37, b"attack_finished"));
            let image = vm_with_definitions(40, GLOBALS, &fields);
            let mut vm = seed_vm(calls, image);
            let value = (2 * vm.stride) as u32;
            let canonical = calls.services.server.clients[client.0 as usize].entity;
            assert_eq!(entity(calls, &vm, value), canonical);
            assert_ne!(canonical.slot, 2);
            calls.context.console.source = RuleSetId::Quake2;
            calls.context.trace_rules = RuleSetId::Quake3;
            let player = &mut calls.services.server.clients[client.0 as usize].player;
            player.movement_rules = RuleSetId::Quake3;
            player.trace_rules = RuleSetId::Quake2Rerelease;
            assert_eq!(player.weapon_state, None);

            for bits in [
                0,
                (-3.25f32).to_bits(),
                0x8000_0000,
                0x3f80_0001,
                0xffc0_1234,
            ] {
                write(&mut vm, value, b"attack_finished", &[bits]);
                vm.project_entities(calls.services, calls.context).unwrap();
                let player = &calls.services.server.clients[client.0 as usize].player;
                let state = player.weapon_state.unwrap();
                assert_eq!(state.source, rules);
                assert_eq!(
                    state.namespace,
                    ResourceNamespace::Module(calls.context.module)
                );
                let WeaponReadiness::DeadlineSeconds(deadline) = state.readiness else {
                    panic!("native QC seconds");
                };
                assert_eq!(deadline.to_bits(), bits);
                assert_eq!(
                    (player.movement_rules, player.trace_rules),
                    (RuleSetId::Quake3, RuleSetId::Quake2Rerelease)
                );
            }

            // Exercise real StorePF dirty publication rather than direct importer setup.
            vm.image.globals[28] = 7.125f32.to_bits();
            vm.image.globals[31] = vm.field_address(value, 37, 1).unwrap() as u32;
            vm.call(calls, 6, 100, false).unwrap();
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            assert_eq!(
                calls.services.server.clients[client.0 as usize]
                    .player
                    .weapon_state,
                Some(WeaponState {
                    source: rules,
                    namespace: ResourceNamespace::Module(calls.context.module),
                    readiness: WeaponReadiness::DeadlineSeconds(7.125),
                })
            );

            let other = vm.spawn_entity(calls.services, calls.context).unwrap();
            write(&mut vm, other, b"attack_finished", &[99.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                calls.services.server.clients[client.0 as usize]
                    .player
                    .weapon_state
                    .unwrap()
                    .readiness,
                WeaponReadiness::DeadlineSeconds(7.125)
            );
            calls.services.server.clients[client.0 as usize].module = ModuleId(17);
            write(&mut vm, value, b"attack_finished", &[123.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                calls.services.server.clients[client.0 as usize]
                    .player
                    .weapon_state
                    .unwrap()
                    .readiness,
                WeaponReadiness::DeadlineSeconds(7.125)
            );
        });
    }
}

#[test]
fn a_missing_native_weapon_deadline_does_not_fabricate_zero() {
    with_services(RuleSetId::Quake, |calls| {
        let client = calls
            .services
            .server
            .connect(
                qa_session::clients::Connection::Local,
                calls.context.module,
                Some(NativeEntity {
                    module: calls.context.module,
                    slot: 1,
                }),
            )
            .unwrap();
        let mut vm = seeded_vm(calls, 8);
        assert_eq!(
            calls.services.server.clients[client.0 as usize]
                .player
                .weapon_state,
            None
        );
        vm.project_entities(calls.services, calls.context).unwrap();
        assert_eq!(
            calls.services.server.clients[client.0 as usize]
                .player
                .weapon_state,
            None
        );
    });
}

fn alias_vm(capacity: usize, body: &[(Opcode, i16, i16, i16)]) -> Vm {
    let mut fields = FIELDS.to_vec();
    fields.extend_from_slice(&[
        (1, 37, b"classname"),
        (4, 38, b"chain"),
        (2, 39, b"invincible_sound"),
        (2, 40, b"health"),
        (2, 41, b"movetype"),
        (3, 42, b"avelocity"),
    ]);
    let bytes = quakec_program::program_with_definitions(body, &[0, 1], GLOBALS, &fields, 45, 96);
    Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: capacity,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap()
}
fn alias_actor(calls: &mut QuakeCCalls<'_, '_>, module: ModuleId, ordinal: i32) -> EntityId {
    let entity = calls
        .services
        .server
        .entities
        .allocate(calls.context.clock, module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let columns = &mut calls.services.server.entities.columns;
    columns.native_entity[entity.slot as usize] = Some(NativeEntity {
        module,
        slot: ordinal,
    });
    columns.set_body(
        entity.slot as usize,
        Body {
            position: Vec3([10.0, 20.0, 30.0]),
            velocity: Vec3([1.0, -2.0, 3.0]),
            mins: Vec3([-1.0, -2.0, -3.0]),
            maxs: Vec3([4.0, 5.0, 6.0]),
        },
    );
    columns.set_angles(entity.slot as usize, Vec3([7.0, 8.0, 9.0]));
    columns.collision_shape[entity.slot as usize] = CollisionShape::Box;
    columns.collision_contents[entity.slot as usize] = qa_world::collision::Contents::SOLID.0;
    entity
}
fn alias_load(
    vm: &mut Vm,
    calls: &mut QuakeCCalls<'_, '_>,
    value: u32,
    field: &[u8],
    vector: bool,
) -> Result<[u32; 3], Trap> {
    vm.image.globals[28] = value;
    vm.image.globals[29] = vm.image.field(field).unwrap() as u32;
    vm.image.statements[1].opcode = if vector { Opcode::LoadV } else { Opcode::LoadF };
    vm.call(calls, 1, 100, false)
}

#[test]
fn foreign_alias_reads_current_published_pose_and_link_bounds_without_a_second_body() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            4,
            &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let first = alias_actor(calls, ModuleId(2), 7);
        let second = alias_actor(calls, ModuleId(3), 7);
        let mut link_context = calls.context;
        link_context.module = ModuleId(2);
        (ENGINE_CALLS.link)(calls.services, link_context, first, LinkFlags::SOLID).unwrap();
        let count = calls.services.server.entities.active().count();
        let relinks = calls.services.server.area.relinks;
        let absolute = calls.services.server.area.bounds(first).unwrap();
        let a = vm
            .admit_entity(&calls.services.server.entities, calls.context, first)
            .unwrap();
        let b = vm
            .admit_entity(&calls.services.server.entities, calls.context, second)
            .unwrap();
        assert_ne!(a.offset(), b.offset());
        assert_eq!(
            vm.resolve_entity(
                &calls.services.server.entities,
                calls.context.module,
                a.offset()
            ),
            Ok(Some(first))
        );
        assert_eq!(
            vm.bound_entity(
                &calls.services.server.entities,
                NativeEntity {
                    module: calls.context.module,
                    slot: vm.entity_number(a.offset()).unwrap() as i32
                }
            ),
            Ok(None)
        );
        for (field, expected) in [
            (b"origin".as_slice(), Vec3([10.0, 20.0, 30.0])),
            (b"velocity", Vec3([1.0, -2.0, 3.0])),
            (b"angles", Vec3([7.0, 8.0, 9.0])),
            (b"mins", Vec3([-1.0, -2.0, -3.0])),
            (b"maxs", Vec3([4.0, 5.0, 6.0])),
            (b"size", Vec3([5.0, 7.0, 9.0])),
            (b"absmin", absolute.mins),
            (b"absmax", absolute.maxs),
        ] {
            assert_eq!(
                alias_load(&mut vm, calls, a.offset(), field, true),
                Ok(expected.0.map(f32::to_bits))
            );
        }
        calls
            .services
            .server
            .entities
            .columns
            .set_position(first.slot as usize, Vec3([40.0, 50.0, 60.0]));
        assert_eq!(
            alias_load(&mut vm, calls, a.offset(), b"origin", true),
            Ok([40.0f32, 50.0, 60.0].map(f32::to_bits))
        );
        assert_eq!(
            alias_load(&mut vm, calls, a.offset(), b"absmin", true),
            Ok(absolute.mins.0.map(f32::to_bits))
        );
        assert_eq!(
            alias_load(&mut vm, calls, b.offset(), b"absmin", true),
            Ok([9.0f32, 18.0, 27.0].map(f32::to_bits))
        );
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(calls.services.server.entities.active().count(), count);
        assert_eq!(calls.services.server.area.relinks, relinks);
        assert_eq!(calls.services.server.area.bounds(first), Some(absolute));
        vm.release_entity(a).unwrap();
        vm.release_entity(b).unwrap();
    });
}

#[test]
fn foreign_box_sound_uses_nq_center_and_qw_origin_with_only_required_authority() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = alias_vm(34, &[(Opcode::Return, 1, 0, 0)]);
            vm.seed_entities(
                &calls.services.server.entities,
                calls.context.module,
                calls.context.native_rules,
                1,
            )
            .unwrap();
            let actor = alias_actor(calls, ModuleId(2), 7);
            calls.services.server.entities.columns.set_body(
                actor.slot as usize,
                Body {
                    position: Vec3::default(),
                    mins: Vec3([-16.0, -16.0, -24.0]),
                    maxs: Vec3([16.0, 16.0, 32.0]),
                    ..Body::default()
                },
            );
            let admitted = vm
                .admit_entity(&calls.services.server.entities, calls.context, actor)
                .unwrap();
            let sound = bind_native_message_sound(calls);
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Remote,
                    ModuleId::default(),
                    None,
                )
                .unwrap();
            let consumer = calls.services.server.clients[client.0 as usize]
                .output
                .unwrap();
            let count = calls.services.server.entities.active().count();
            let relinks = calls.services.server.area.relinks;
            let channel = if rules == RuleSetId::QuakeWorld { 9 } else { 1 };
            vm.start_sound(
                calls.services,
                calls.context,
                calls.platform_time,
                admitted.offset(),
                b"test.wav",
                channel,
                255,
                1.0,
            )
            .unwrap();
            let mut batch = calls.services.server.events.batch(consumer).unwrap();
            let record = calls.services.server.events.next(&mut batch).unwrap();
            let qa_core::events::FrameEvent::Sound(event) = record.event else {
                panic!("sound record");
            };
            let centered = rules == RuleSetId::Quake;
            let expected = if centered {
                Vec3([0.0, 0.0, 4.0])
            } else {
                Vec3::default()
            };
            assert_eq!(event.sound, sound);
            assert_eq!(event.entity, Some(SoundEmitter::Entity(actor)));
            assert_eq!(event.position, expected);
            assert_eq!(event.positioned, centered);
            assert_eq!(event.channel, 1);
            assert_eq!(event.spatial, calls.context.sound_policy);
            let delivery = record.delivery.unwrap();
            assert_eq!(delivery.module, calls.context.module);
            assert_eq!(delivery.source, rules);
            assert_eq!(delivery.origin, expected);
            assert_eq!(delivery.target, NativeTarget::All);
            assert_eq!(delivery.reliable, rules == RuleSetId::QuakeWorld);

            calls.services.server.entities.columns.published_fields[actor.slot as usize] =
                EntityFields::POSITION;
            let before = calls.services.server.events.sequence_range();
            let result = vm.start_sound(
                calls.services,
                calls.context,
                calls.platform_time,
                admitted.offset(),
                b"test.wav",
                channel,
                255,
                1.0,
            );
            if centered {
                assert_eq!(result, Err(CallError::Unsupported));
                assert_eq!(calls.services.server.events.sequence_range(), before);
            } else {
                result.unwrap();
                let mut batch = calls.services.server.events.batch(consumer).unwrap();
                calls.services.server.events.next(&mut batch).unwrap();
                let second = calls.services.server.events.next(&mut batch).unwrap();
                let qa_core::events::FrameEvent::Sound(event) = second.event else {
                    panic!("second sound record");
                };
                assert_eq!(event.position, Vec3::default());
                assert!(!event.positioned);
                assert_eq!(event.entity, Some(SoundEmitter::Entity(actor)));
            }
            assert_eq!(calls.services.server.entities.active().count(), count);
            assert_eq!(calls.services.server.area.relinks, relinks);
            assert_eq!(
                calls.services.server.entities.columns.native_entity[actor.slot as usize],
                Some(NativeEntity {
                    module: ModuleId(2),
                    slot: 7
                })
            );
            vm.release_entity(admitted).unwrap();
        });
    }
}

#[test]
fn foreign_reads_reject_unknown_authority_and_private_layouts_before_destination_writes() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            3,
            &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let actor = alias_actor(calls, ModuleId(2), 7);
        calls.services.server.entities.columns.published_fields[actor.slot as usize] =
            EntityFields::POSITION;
        let admitted = vm
            .admit_entity(&calls.services.server.entities, calls.context, actor)
            .unwrap();
        for field in [b"velocity".as_slice(), b"solid", b"health", b"classname"] {
            vm.image.globals[50..53].copy_from_slice(&[11, 22, 33]);
            assert_eq!(
                alias_load(
                    &mut vm,
                    calls,
                    admitted.offset(),
                    field,
                    field == b"velocity"
                ),
                Err(Trap::EntityAccess)
            );
            assert_eq!(vm.image.globals[50..53], [11, 22, 33]);
        }
        vm.image.globals[28] = admitted.offset();
        vm.image.globals[29] = 0;
        vm.image.statements[1].opcode = Opcode::LoadV;
        let mut run = vm.begin_root(1, 100, false).unwrap();
        assert_eq!(
            vm.advance(&mut run, &qa_compat::quakec::EntityAccess::native_only()),
            Err(Trap::EntityAccess)
        );
        vm.release_entity(admitted).unwrap();
    });
}

#[test]
fn foreign_classname_cache_reuses_its_bounded_string_reservation() {
    with_services(RuleSetId::Quake, |calls| {
        *calls.services.names =
            NameTable::load([b"abcdefgh".as_slice(), b"xy", b"longer than eight"]).unwrap();
        let mut vm = alias_vm(
            3,
            &[(Opcode::LoadS, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let actor = alias_actor(calls, ModuleId(2), 7);
        calls.services.server.entities.columns.classname[actor.slot as usize] =
            calls.services.names.find(b"abcdefgh");
        let admitted = vm
            .admit_entity(&calls.services.server.entities, calls.context, actor)
            .unwrap();
        let offset = alias_load(&mut vm, calls, admitted.offset(), b"classname", false).unwrap()[0];
        assert_eq!(vm.string(offset as i32).unwrap(), b"abcdefgh");
        while vm.insert_string(b"x").is_ok() {}
        assert_eq!(
            alias_load(&mut vm, calls, admitted.offset(), b"classname", false).unwrap()[0],
            offset
        );
        calls.services.server.entities.columns.classname[actor.slot as usize] =
            calls.services.names.find(b"xy");
        assert_eq!(
            alias_load(&mut vm, calls, admitted.offset(), b"classname", false).unwrap()[0],
            offset
        );
        assert_eq!(vm.string(offset as i32).unwrap(), b"xy");
        calls.services.server.entities.columns.classname[actor.slot as usize] =
            calls.services.names.find(b"longer than eight");
        vm.image.globals[50..53].copy_from_slice(&[11, 22, 33]);
        assert_eq!(
            alias_load(&mut vm, calls, admitted.offset(), b"classname", false),
            Err(Trap::String)
        );
        assert_eq!(vm.image.globals[50..53], [11, 22, 33]);
        vm.release_entity(admitted).unwrap();
    });
}

#[test]
fn live_aliases_keep_their_ordinal_and_retired_pins_hold_capacity_until_release() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            3,
            &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let first = alias_actor(calls, ModuleId(2), 7);
        let second = alias_actor(calls, ModuleId(3), 7);
        let one = vm
            .admit_entity(&calls.services.server.entities, calls.context, first)
            .unwrap();
        let offset = one.offset();
        vm.release_entity(one).unwrap();
        assert!(matches!(
            vm.admit_entity(&calls.services.server.entities, calls.context, second),
            Err(CallError::Capacity)
        ));
        let one = vm
            .admit_entity(&calls.services.server.entities, calls.context, first)
            .unwrap();
        let two = vm
            .admit_entity(&calls.services.server.entities, calls.context, first)
            .unwrap();
        assert_eq!(one.offset(), offset);
        assert_eq!(two.offset(), offset);
        assert!(
            calls
                .services
                .server
                .free_entity(first, calls.context.clock)
        );
        assert_eq!(
            alias_load(&mut vm, calls, offset, b"origin", true),
            Err(Trap::EntityAccess)
        );
        vm.release_entity(one).unwrap();
        assert!(matches!(
            vm.admit_entity(&calls.services.server.entities, calls.context, second),
            Err(CallError::Capacity)
        ));
        vm.release_entity(two).unwrap();
        let replacement = vm
            .admit_entity(&calls.services.server.entities, calls.context, second)
            .unwrap();
        assert_eq!(replacement.offset(), offset);
        vm.release_entity(replacement).unwrap();
    });
}

#[test]
fn captured_retired_reference_stays_nonzero_and_pinned_without_becoming_readable() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            3,
            &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let retired = alias_actor(calls, ModuleId(2), 7);
        let captured = calls
            .services
            .server
            .entities
            .capture_reference(retired)
            .unwrap();
        let other = alias_actor(calls, ModuleId(3), 7);
        assert!(
            calls
                .services
                .server
                .free_entity(retired, calls.context.clock)
        );
        assert!(matches!(
            vm.admit_reference(
                &calls.services.server.entities,
                calls.context,
                captured.reference().unwrap()
            ),
            Err(CallError::Entity)
        ));
        let one = vm
            .admit_captured_reference(&calls.services.server.entities, calls.context, captured)
            .unwrap();
        let two = vm
            .admit_captured_reference(&calls.services.server.entities, calls.context, captured)
            .unwrap();
        let offset = one.offset();
        assert_ne!(offset, 0);
        assert_eq!(two.offset(), offset);
        vm.image.globals[50..53].copy_from_slice(&[11, 22, 33]);
        assert_eq!(
            alias_load(&mut vm, calls, offset, b"origin", true),
            Err(Trap::EntityAccess)
        );
        assert_eq!(vm.image.globals[50..53], [11, 22, 33]);
        vm.release_entity(one).unwrap();
        assert!(matches!(
            vm.admit_entity(&calls.services.server.entities, calls.context, other),
            Err(CallError::Capacity)
        ));
        vm.release_entity(two).unwrap();
        let reused = vm
            .admit_entity(&calls.services.server.entities, calls.context, other)
            .unwrap();
        assert_eq!(reused.offset(), offset);
        assert_eq!(
            alias_load(&mut vm, calls, reused.offset(), b"origin", true).unwrap(),
            [10.0_f32.to_bits(), 20.0_f32.to_bits(), 30.0_f32.to_bits()]
        );
        vm.release_entity(reused).unwrap();
    });
}

#[test]
fn anonymous_capture_admission_retains_exact_generation_and_retired_pin() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            3,
            &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let actor = alias_actor(calls, ModuleId(2), 7);
        calls.services.server.entities.columns.native_entity[actor.slot as usize] = None;
        let captured = calls
            .services
            .server
            .entities
            .capture_reference(actor)
            .unwrap();
        assert_eq!(captured.reference(), None);
        let live = vm
            .admit_captured_reference(&calls.services.server.entities, calls.context, captured)
            .unwrap();
        assert_ne!(live.offset(), 0);
        assert_eq!(
            alias_load(&mut vm, calls, live.offset(), b"origin", true).unwrap(),
            [10.0_f32.to_bits(), 20.0_f32.to_bits(), 30.0_f32.to_bits()]
        );
        assert!(
            calls
                .services
                .server
                .free_entity(actor, calls.context.clock)
        );
        let retired = vm
            .admit_captured_reference(&calls.services.server.entities, calls.context, captured)
            .unwrap();
        assert_eq!(retired.offset(), live.offset());
        assert_eq!(
            alias_load(&mut vm, calls, retired.offset(), b"origin", true),
            Err(Trap::EntityAccess)
        );
        let replacement = alias_actor(calls, ModuleId(2), 7);
        calls.services.server.entities.columns.native_entity[replacement.slot as usize] = None;
        assert_eq!(
            calls.services.server.entities.resolve_reference(captured),
            None
        );
        vm.release_entity(live).unwrap();
        assert!(matches!(
            vm.admit_entity(&calls.services.server.entities, calls.context, replacement),
            Err(CallError::Capacity)
        ));
        vm.release_entity(retired).unwrap();
        let current = vm
            .admit_entity(&calls.services.server.entities, calls.context, replacement)
            .unwrap();
        assert_eq!(
            vm.resolve_entity(
                &calls.services.server.entities,
                calls.context.module,
                current.offset()
            )
            .unwrap(),
            Some(replacement)
        );
        vm.release_entity(current).unwrap();
    });
}

#[test]
fn captured_reference_prefers_current_native_row_over_retired_canonical_generation() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            4,
            &[(Opcode::LoadV, 28, 29, 50), (Opcode::Return, 50, 0, 0)],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let retired = alias_actor(calls, ModuleId(2), 7);
        let captured = calls
            .services
            .server
            .entities
            .capture_reference(retired)
            .unwrap();
        assert!(
            calls
                .services
                .server
                .free_entity(retired, calls.context.clock)
        );
        let old_pin = vm
            .admit_captured_reference(&calls.services.server.entities, calls.context, captured)
            .unwrap();
        let current = alias_actor(calls, ModuleId(2), 7);
        assert_ne!(current, retired);
        calls
            .services
            .server
            .entities
            .columns
            .set_position(current.slot as usize, Vec3([70.0, 80.0, 90.0]));
        let replacement = vm
            .admit_captured_reference(&calls.services.server.entities, calls.context, captured)
            .unwrap();
        assert_ne!(old_pin.offset(), replacement.offset());
        assert_eq!(
            alias_load(&mut vm, calls, replacement.offset(), b"origin", true).unwrap(),
            [70.0_f32.to_bits(), 80.0_f32.to_bits(), 90.0_f32.to_bits()]
        );
        assert_eq!(
            alias_load(&mut vm, calls, old_pin.offset(), b"origin", true),
            Err(Trap::EntityAccess)
        );
        assert_eq!(
            vm.resolve_entity(
                &calls.services.server.entities,
                calls.context.module,
                replacement.offset()
            )
            .unwrap(),
            Some(current)
        );
        vm.release_entity(old_pin).unwrap();
        vm.release_entity(replacement).unwrap();
    });
}

#[test]
fn foreign_private_scalar_words_persist_but_vector_cross_row_and_state_stores_are_atomic_rejections()
 {
    with_services(RuleSetId::Quake, |calls| {
        for hooks in [false, true] {
            let mut vm = alias_vm(
                4,
                &[(Opcode::StorePF, 28, 31, 0), (Opcode::Return, 1, 0, 0)],
            );
            vm.seed_entities(
                &calls.services.server.entities,
                calls.context.module,
                calls.context.native_rules,
                1,
            )
            .unwrap();
            let actor = alias_actor(calls, ModuleId(2), 7);
            let admitted = vm
                .admit_entity(&calls.services.server.entities, calls.context, actor)
                .unwrap();
            let count = calls.services.server.entities.active().count();
            vm.image.globals[28] = 0x7fc1_2345;
            for field in [b"chain".as_slice(), b"invincible_sound"] {
                vm.image.globals[31] = vm
                    .field_address(admitted.offset(), vm.image.field(field).unwrap() as u32, 1)
                    .unwrap() as u32;
                assert!(vm.call(calls, 1, 100, hooks).is_ok());
                assert_eq!(read(&vm, admitted.offset(), field, 1), [0x7fc1_2345]);
            }
            vm.image.statements[1].opcode = Opcode::StorePV;
            let before = vm.entities.read(0, vm.entities.len()).unwrap().to_vec();
            assert_eq!(vm.call(calls, 1, 100, hooks), Err(Trap::EntityAccess));
            assert_eq!(vm.entities.read(0, vm.entities.len()).unwrap(), before);
            vm.image.statements[1].opcode = Opcode::StorePF;
            vm.image.globals[31] = admitted.offset() + vm.header_bytes as u32 - 2;
            assert_eq!(vm.call(calls, 1, 100, hooks), Err(Trap::EntityAccess));
            assert_eq!(vm.entities.read(0, vm.entities.len()).unwrap(), before);
            vm.image.globals[31] = admitted.offset() - 2;
            assert_eq!(vm.call(calls, 1, 100, hooks), Err(Trap::EntityAccess));
            assert_eq!(vm.entities.read(0, vm.entities.len()).unwrap(), before);
            vm.image.globals[34] = admitted.offset();
            vm.image.statements[1].opcode = Opcode::State;
            assert_eq!(vm.call(calls, 1, 100, hooks), Err(Trap::EntityAccess));
            assert_eq!(vm.entities.read(0, vm.entities.len()).unwrap(), before);
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            assert_eq!(calls.services.server.entities.active().count(), count);
            assert_eq!(
                calls.services.server.entities.columns.position[actor.slot as usize],
                Vec3([10.0, 20.0, 30.0])
            );
            assert_eq!(read(&vm, admitted.offset(), b"chain", 1), [0x7fc1_2345]);
            vm.release_entity(admitted).unwrap();
        }
    });
}

#[test]
fn native_store_prefix_survives_a_later_rejected_foreign_vector_store() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = alias_vm(
            4,
            &[
                (Opcode::StorePF, 28, 31, 0),
                (Opcode::StorePV, 50, 32, 0),
                (Opcode::Return, 1, 0, 0),
            ],
        );
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let native = vm.spawn_entity(calls.services, calls.context).unwrap();
        let own = entity(calls, &vm, native);
        let actor = alias_actor(calls, ModuleId(2), 7);
        let admitted = vm
            .admit_entity(&calls.services.server.entities, calls.context, actor)
            .unwrap();
        let alias_bytes = vm
            .entities
            .read(u64::from(admitted.offset()), vm.stride)
            .unwrap()
            .to_vec();
        vm.image.globals[28] = 17.0f32.to_bits();
        vm.image.globals[31] = vm.field_address(native, 29, 1).unwrap() as u32;
        vm.image.globals[32] = vm.field_address(admitted.offset(), 0, 3).unwrap() as u32;
        vm.image.globals[50..53].copy_from_slice(&[9.0f32, 8.0, 7.0].map(f32::to_bits));
        assert_eq!(vm.call(calls, 1, 100, false), Err(Trap::EntityAccess));
        assert_eq!(
            vm.entities
                .read(u64::from(admitted.offset()), vm.stride)
                .unwrap(),
            alias_bytes
        );
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.appearance[own.slot as usize].frame,
            17
        );
        assert_eq!(
            calls.services.server.entities.columns.position[actor.slot as usize],
            Vec3([10.0, 20.0, 30.0])
        );
        vm.release_entity(admitted).unwrap();
    });
}

#[test]
fn full_qw_backing_never_overwrites_a_live_foreign_alias() {
    with_services(RuleSetId::QuakeWorld, |calls| {
        let mut vm = alias_vm(34, &[(Opcode::Return, 1, 0, 0)]);
        vm.seed_entities(
            &calls.services.server.entities,
            calls.context.module,
            calls.context.native_rules,
            1,
        )
        .unwrap();
        let actor = alias_actor(calls, ModuleId(2), 7);
        let admitted = vm
            .admit_entity(&calls.services.server.entities, calls.context, actor)
            .unwrap();
        let offset = admitted.offset();
        vm.release_entity(admitted).unwrap();
        let count = calls.services.server.entities.active().count();
        assert_eq!(
            vm.spawn_entity(calls.services, calls.context),
            Err(CallError::Capacity)
        );
        assert_eq!(calls.services.server.entities.active().count(), count);
        assert_eq!(
            vm.resolve_entity(
                &calls.services.server.entities,
                calls.context.module,
                offset
            ),
            Ok(Some(actor))
        );
    });
}

#[test]
fn quakec_trace_globals_adopt_foreign_brush_media_from_the_caller_trace_role() {
    use qa_world::collision::Contents;
    let water = Bounds {
        mins: Vec3([2.0, -2.0, -2.0]),
        maxs: Vec3([4.0, 2.0, 2.0]),
    };
    let wall = Bounds {
        mins: Vec3([6.0, -2.0, -2.0]),
        maxs: Vec3([7.0, 2.0, 2.0]),
    };
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_brush_services(
            native,
            None,
            &[(water, Contents::WATER), (wall, Contents::SOLID)],
            |calls| {
                let bytes = quakec_program::program_with_definitions(
                    &[(Opcode::Call4, 32, 0, 0), (Opcode::Return, 1, 0, 0)],
                    &[0, 1, -16],
                    GLOBALS,
                    FIELDS,
                    37,
                    96,
                );
                let layout = Layout {
                    entities: 64,
                    header_bytes: 16,
                    extra_string_bytes: 128,
                    state_step: 0.1,
                    random_seed: 1,
                };
                let mut vm = seed_vm(
                    calls,
                    Vm::load(Image::parse(&bytes, Some(5927)).unwrap(), layout).unwrap(),
                );
                vm.image.globals[32] = 2;
                let start = [0.0f32; 3].map(f32::to_bits);
                let end = [10.0f32, 0.0, 0.0].map(f32::to_bits);
                calls.context.console.source = RuleSetId::Quake3;
                for role in [
                    RuleSetId::Quake,
                    RuleSetId::QuakeWorld,
                    RuleSetId::Quake2,
                    RuleSetId::Quake2Rerelease,
                    RuleSetId::Quake3,
                ] {
                    calls.context.trace_rules = role;
                    parameters(&mut vm, &[&start, &end, &[0], &[0]]);
                    vm.call(calls, 1, 64, false).unwrap();
                    let native_media = matches!(role, RuleSetId::Quake | RuleSetId::QuakeWorld);
                    let expected = if native_media { 1.0f32 } else { 0.0f32 }.to_bits();
                    assert_eq!(
                        &vm.image.globals[63..65],
                        &[expected; 2],
                        "native={native:?} trace={role:?}"
                    );
                    assert!(f32::from_bits(vm.image.globals[62]) > 0.5);
                    assert!(f32::from_bits(vm.image.globals[62]) < 0.6);
                    assert_eq!(&vm.image.globals[69..72], &[(-1.0f32).to_bits(), 0, 0]);
                }
            },
        );
    }
}

#[test]
fn quakec_precache_and_model_bounds_pass_the_calling_modules_file_selection() {
    let mut models = Models::default();
    let mut retained = None;
    with_model_services(RuleSetId::Quake, Some(&mut models), |calls| {
        let search = calls.services.vfs.search_view([]).unwrap();
        calls
            .services
            .storage
            .set_file_search(
                calls.context.module,
                Some(search.clone()),
                calls.services.vfs,
            )
            .unwrap();
        let mut vm = vm(4);
        let name = vm.insert_string(b"progs/selected.mdl").unwrap() as u32;
        parameters(&mut vm, &[&[name]]);
        calls.call(&mut vm, 20, 1).unwrap();
        retained = Some(search);
    });
    let selected = retained.unwrap();
    assert!(
        models.loaded_searches[0]
            .as_ref()
            .unwrap()
            .same_selection(&selected)
    );
    models.expected_search = Some(selected.clone());
    with_model_services(RuleSetId::Quake, Some(&mut models), |calls| {
        // A new service owner must not obtain metadata from another owner's
        // cached file just because the path and rules happen to match.
        assert_eq!(
            (ENGINE_CALLS.model_bounds)(
                calls.services,
                calls.context.module,
                b"progs/selected.mdl",
                RuleSetId::Quake
            ),
            Err(CallError::Geometry)
        );
    });
}
