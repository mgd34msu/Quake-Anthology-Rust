//! Complete provider PUSH transactions against an owned native Q3 client.
use super::*;
use qa_app::modules::State as ModuleState;
use qa_compat::quakec::{Layout, Vm as QuakeC};
use qa_core::primitives::{ClientId, EntityId, NativeEntity, Vec3};
use qa_formats::program::quakec::{Image as QcImage, Opcode::*};
use qa_session::clients::Connection;

#[path = "../support/quakec_fields.rs"]
mod quakec_fields;
#[path = "../../../compat/tests/support/quakec_program.rs"]
#[allow(dead_code)]
mod quakec_program;

const PUSHER: ModuleId = ModuleId(1);
const CLIENT: ModuleId = ModuleId(2);
const ENTITIES: u64 = 0x2000;
const PLAYERS: u64 = 0x3000;

#[derive(Clone, Copy)]
enum Case {
    Ride,
    Trigger,
    Blocked,
}

fn own(runtime: &mut Runtime, slot: i32) -> EntityId {
    let entity = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), PUSHER, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
        module: PUSHER,
        slot,
    });
    entity
}

fn pusher(runtime: &mut Runtime, case: Case) -> (ModuleRequest, EntityId) {
    let mut operations = Vec::new();
    let mut functions = vec![0];
    let mut function = |body: &[_]| {
        functions.push(operations.len() as i32 + 1);
        operations.extend_from_slice(body);
    };
    function(&[(Return, 0, 0, 0)]); // StartFrame
    let mut initialize = vec![(StoreEnt, 80, 4, 0), (StoreV, 81, 7, 0), (Call2, 52, 0, 0)];
    if !matches!(case, Case::Ride) {
        initialize.extend([(StoreEnt, 84, 4, 0), (StoreV, 85, 7, 0), (Call2, 52, 0, 0)]);
    }
    initialize.push((Return, 0, 0, 0));
    function(&initialize);
    function(&[(LoadV, 36, 53, 60), (Return, 60, 0, 0)]); // Blocked
    function(&[(LoadV, 36, 53, 60), (AddF, 64, 65, 64), (Return, 60, 0, 0)]); // Trigger
    function(&[(Return, 60, 0, 0)]); // Inspect the callback's Other pose
    functions.push(-2); // setorigin, through the shared CallTable and link service
    let mut fields = quakec_fields::FIELDS.to_vec();
    fields.extend([
        (4, 40, b"groundentity".as_slice()),
        (2, 41, b"ltime"),
        (6, 42, b"blocked"),
    ]);
    let bytes = quakec_program::program_with_definitions(
        &operations,
        &functions,
        &[(4, 34, b"self"), (2, 35, b"time"), (4, 36, b"other")],
        &fields,
        49,
        128,
    );
    let mut vm = QuakeC::load(
        QcImage::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    vm.image.globals[52] = 6;
    vm.image.globals[53] = 0; // origin field
    vm.image.globals[65] = 1.0f32.to_bits();
    let entity = own(runtime, 40);
    let extra = match case {
        Case::Ride => None,
        Case::Trigger => Some((Vec3([1.0, 0.0, 6.0]), 1.0f32, 4u32)),
        Case::Blocked => Some((Vec3([7.5625, 0.0, 0.0]), 2.0f32, 0u32)),
    };
    for (slot, position, solid, touch, bounds) in
        std::iter::once((40, Vec3::default(), 4.0f32, 0u32, 4.0f32)).chain(extra.map(
            |(position, solid, touch)| {
                own(runtime, 41);
                (41, position, solid, touch, 1.0)
            },
        ))
    {
        let native = slot * vm.stride as u32;
        for (field, value) in [
            (0, position),
            (9, Vec3([-bounds; 3])),
            (12, Vec3([bounds; 3])),
        ] {
            let at = vm.field_address(native, field, 3).unwrap();
            vm.entities.write_vec3(at, value).unwrap();
        }
        for (field, value) in [(24, solid.to_bits()), (30, touch)] {
            let at = vm.field_address(native, field, 1).unwrap();
            vm.entities.write_word(at, value as i32).unwrap();
        }
        let constant = if slot == 40 { 80 } else { 84 };
        vm.image.globals[constant] = native;
        vm.image.globals[constant + 1..constant + 4].copy_from_slice(&position.0.map(f32::to_bits));
    }
    let native = 40 * vm.stride as u32;
    let model = vm.insert_string(b"*1").unwrap() as u32;
    for (field, value) in [
        (27, model),
        (33, 7.0f32.to_bits()),
        (41, 1.0f32.to_bits()),
        (31, 100.0f32.to_bits()),
        (42, if matches!(case, Case::Blocked) { 3 } else { 0 }),
    ] {
        let at = vm.field_address(native, field, 1).unwrap();
        vm.entities.write_word(at, value as i32).unwrap();
    }
    let at = vm.field_address(native, 3, 3).unwrap();
    vm.entities.write_vec3(at, Vec3([16.0, 0.0, 0.0])).unwrap();
    let mut request = request_program(
        runtime,
        PUSHER,
        RuleSetId::Quake,
        TickRate::FrameDriven,
        Program::quakec(vm, None),
    );
    runtime.server.entities.columns.native_entity[request.anchor.slot as usize] =
        Some(NativeEntity {
            module: PUSHER,
            slot: 0,
        });
    request.context.clock = ThinkTime::Seconds(0.0);
    request.entries = (0..6).collect();
    request.frame = Export::words(CallbackId(1), &[]);
    request.initialize = vec![Export::words(CallbackId(2), &[])];
    (request, entity)
}

fn native(runtime: &mut Runtime, position: Vec3) -> ModuleRequest {
    let mut file = pe_export_fixture(64);
    put(&mut file, 168, 0, 4); // No DLL entry point in this authored module.
    put(&mut file, 208, 20480, 4);
    put(&mut file, 392 + 8, 16384, 4);
    put(&mut file, 392 + 16, 16384, 4);
    file.resize(16896, 0);
    pe_rva(&mut file, 0x1140, 0x1200, 4);
    pe_text(&mut file, 0x1200, &[0x31, 0xc0, 0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let named: Vec<_> = (0..=9)
        .into_iter()
        .map(|command| NamedExport {
            name: b"GetGameAPI",
            command: Some(command),
            parameters: &[NativeScalar::I32, NativeScalar::Word],
            result: NativeScalar::Word,
        })
        .collect();
    let mut vm = Vm::map_image(image, &named, &[], Duration::from_secs(3)).unwrap();
    vm.configure_q3_game(qa_core::primitives::Q3StatLayout::BASE);
    // Command 0 registers and links the public entity/player arrays through
    // real child imports. Command 1 reads a word from the same child memory.
    let mut code = vec![0x48, 0xb8];
    code.extend((base + 0x4000).to_le_bytes());
    code.extend([0xff, 0x04, 0x88]); // inc callback_counts[command]
    let branch_start = code.len();
    code.extend([
        0x83, 0xf9, 1, 0x0f, 0x84, 0, 0, 0, 0, 0x85, 0xc9, 0x74, 3, 0x31, 0xc0, 0xc3,
    ]);
    code.extend([0x48, 0x83, 0xec, 56, 0xb9, 15, 0, 0, 0, 0x48, 0xba]);
    code.extend((base + ENTITIES).to_le_bytes());
    code.extend([0x41, 0xb8, 1, 0, 0, 0, 0x41, 0xb9]);
    code.extend(608u32.to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend((base + PLAYERS).to_le_bytes());
    code.extend([0x48, 0x89, 0x44, 0x24, 32, 0x48, 0xc7, 0x44, 0x24, 40]);
    code.extend(640u32.to_le_bytes());
    for number in [None, Some(30u32)] {
        if let Some(number) = number {
            code.push(0xb9);
            code.extend(number.to_le_bytes());
            code.extend([0x48, 0xba]);
            code.extend((base + ENTITIES).to_le_bytes());
        }
        code.extend([0x48, 0xb8]);
        code.extend(vm.import_callback().to_le_bytes());
        code.extend([0xff, 0xd0]);
    }
    code.extend([0x48, 0x83, 0xc4, 56, 0x31, 0xc0, 0xc3]);
    let branch = code.len() as i32 - (branch_start as i32 + 9);
    code[branch_start + 5..branch_start + 9].copy_from_slice(&branch.to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend(base.to_le_bytes());
    code.extend([0x8b, 0x04, 0x10, 0xc3]);
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        memory.write(base + 0x1200, &code).unwrap();
        for (offset, word) in [(0, 0u32), (460, 1), (512, 1023)] {
            memory
                .write_word(base + ENTITIES + offset, word as i32)
                .unwrap();
        }
        for offset in [24, 92, 488] {
            memory
                .write_vec3(base + ENTITIES + offset, position)
                .unwrap();
        }
        memory
            .write_vec3(base + ENTITIES + 436, Vec3([-1.0; 3]))
            .unwrap();
        memory
            .write_vec3(base + ENTITIES + 448, Vec3([1.0; 3]))
            .unwrap();
        memory.write_vec3(base + PLAYERS + 20, position).unwrap();
        memory
            .write_vec3(base + PLAYERS + 32, Vec3([2.0, 3.0, 4.0]))
            .unwrap();
        memory.write_word(base + PLAYERS + 148, 1023).unwrap();
    }
    let mut request = request(
        runtime,
        CLIENT,
        RuleSetId::Quake3,
        TickRate::fixed(1000).unwrap(),
        vm,
    );
    request.entries = (0..=9).collect();
    request.initialize = vec![Export::words(CallbackId(0), &[])];
    request.frame = Export::words(CallbackId(1), &[0]);
    request
}

pub(super) fn run() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for case in [Case::Ride, Case::Trigger, Case::Blocked] {
            let mut runtime = Runtime::load(1, []).unwrap();
            visibility_world::inline_box(&mut runtime, 4.0);
            let query = visibility_world::inline_box(&mut runtime, 12.0);
            runtime.collision = Some(visibility_world::collision(
                &runtime.geometry,
                [None, None],
                qa_world::visibility::PvsRows::load(vec![None, None], vec![], None).unwrap(),
            ));
            let (mut pushing, pusher) = pusher(&mut runtime, case);
            pushing.context.native_rules = rules;
            pushing.context.clock = ThinkTime::module_clock(rules);
            pushing.context.link_order = qa_gameplay::rules::link_order(rules);
            pushing.timing_rules = rules;
            let position = if matches!(case, Case::Blocked) {
                Vec3([5.5, 0.0, 0.0])
            } else {
                Vec3([0.0, 0.0, 6.0])
            };
            let passenger = native(&mut runtime, position);
            let client = runtime
                .server
                .connect(
                    Connection::Bot,
                    CLIENT,
                    Some(NativeEntity {
                        module: CLIENT,
                        slot: 0,
                    }),
                )
                .unwrap();
            assert_eq!(client, ClientId(0));
            let entity = runtime.server.clients[0].entity;
            runtime.server.clients[0].module_movement = true;
            runtime.server.clients[0].player.movement_rules = RuleSetId::Quake2;
            runtime.server.clients[0].player.trace_rules = RuleSetId::Quake3;
            // Advance only the independent pusher provider. No new bot
            // command or native game tick replaces the accepted ground pose.
            let mut host = FrameHost::load_modules(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::fixed(1000).unwrap(),
                vec![pushing, passenger],
            )
            .unwrap();
            host.initialize_modules(Phase::Server);
            host.connect_module_client(CLIENT, client, b"\\name\\passenger")
                .unwrap();
            for module in [PUSHER, CLIENT] {
                assert_eq!(host.module_state(module), Some(ModuleState::Running));
                assert_eq!(host.module_counts(module).unwrap().traps, 0);
            }
            let visibility = host.runtime.collision.as_ref().unwrap().visibility.clone();
            host.runtime.collision = Some(
                qa_app::WorldCollision::new(&host.runtime.geometry, query, 0)
                    .with_visibility(visibility.unwrap()),
            );
            host.runtime.server.clients[0].player.movement.grounded = true;
            host.runtime.server.clients[0].player.movement.ground = Some(pusher);
            let mut source = Source {
                time: EventTime(0),
                polls: 0,
            };
            host.frame(&mut source, true);
            let calls = host.module_counts(CLIENT).unwrap().calls;
            source.time = EventTime(62_500_000);
            host.frame(&mut source, true);
            assert_eq!(
                host.module_counts(CLIENT).unwrap().calls,
                calls,
                "PUSH must not invoke the passenger module"
            );
            assert_eq!(host.module_counts(PUSHER).unwrap().rejected, 0);
            let expected = if matches!(case, Case::Blocked) {
                position
            } else {
                position + Vec3([1.0, 0.0, 0.0])
            };
            assert_eq!(
                host.runtime.server.entities.columns.position[entity.slot as usize],
                expected
            );
            let player = &host.runtime.server.clients[0].player;
            assert_eq!(player.body.position, expected);
            assert_eq!(player.body.velocity, Vec3([2.0, 3.0, 4.0]));
            assert_eq!(player.movement_rules, RuleSetId::Quake2);
            assert_eq!(player.trace_rules, RuleSetId::Quake3);
            assert_eq!(
                host.runtime.server.entities.columns.position[pusher.slot as usize],
                if matches!(case, Case::Blocked) {
                    Vec3::default()
                } else {
                    Vec3([1.0, 0.0, 0.0])
                }
            );
            if !matches!(case, Case::Ride) {
                let touched = qa_gameplay::rules::motion::push_rules(rules)
                    .unwrap()
                    .traced_passengers;
                let callback_pose = if matches!(case, Case::Trigger) && !touched {
                    Vec3::default()
                } else {
                    expected
                };
                host.call_module_export(
                    PUSHER,
                    Export::words(CallbackId(5), &[]),
                    ThinkTime::Seconds(0.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(PUSHER).unwrap().last_result,
                    Some(ModuleResult::QuakeC(callback_pose.0.map(f32::to_bits)))
                );
            }
            // Read back via a real native export, after the full transaction.
            for at in [ENTITIES + 24, ENTITIES + 92, ENTITIES + 488, PLAYERS + 20] {
                for (axis, value) in expected.0.into_iter().enumerate() {
                    host.call_module_export(
                        CLIENT,
                        Export::words(CallbackId(1), &[at + axis as u64 * 4]),
                        ThinkTime::Milliseconds(0),
                    )
                    .unwrap();
                    assert_eq!(
                        host.module_counts(CLIENT).unwrap().last_result,
                        Some(ModuleResult::Native(u64::from(value.to_bits())))
                    );
                }
            }
            host.call_module_export(
                CLIENT,
                Export::words(CallbackId(1), &[0x4000 + 7 * 4]),
                ThinkTime::Milliseconds(0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(CLIENT).unwrap().last_result,
                Some(ModuleResult::Native(0)),
                "no usercmd is due before the independent world/client tick"
            );
            for module in [PUSHER, CLIENT] {
                assert_eq!(host.module_counts(module).unwrap().traps, 0);
            }
            host.shutdown_modules();
        }
    }
}
