use super::*;
use crate::{
    Runtime, WorldCollision,
    modules::{
        ModuleRequest,
        contacts::tests::{FIELDS, request},
    },
};
use qa_compat::quakec::Layout;
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{Bounds, IndexSpan, Plane, SurfaceFlags},
    sys_events::EventTime,
};
use qa_formats::program::quakec::{Image, Opcode::*};
use qa_session::{clients::Connection, timing::TickRate};
use qa_world::{
    collision::{
        Contents,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
    },
    entities::AllocationPolicy,
};

#[path = "../../../compat/tests/support/quakec_program.rs"]
mod quakec_program;

#[path = "../support/service_program.rs"]
mod service_program;
#[path = "../support/visibility_world.rs"]
mod visibility_world;

fn qvm_passenger_fixture(a: RuleSetId, connection: Connection) -> Fixture {
    use qa_formats::program::qvm::Opcode as Q;
    let mut runtime = Runtime::load(1, []).unwrap();
    let owner = geometry(&mut runtime, 4.0);
    let query = geometry(&mut runtime, 12.0);
    runtime.collision = Some(visibility_world::collision(
        &runtime.geometry,
        [None, None],
        qa_world::visibility::PvsRows::load(vec![None, None], vec![], None).unwrap(),
    ));
    let (first, _) = request(
        &mut runtime,
        A,
        Program::quakec(program(64), None),
        (0..10).collect(),
        a,
    );
    let mut vm = service_program::program(
        &[
            (Q::Enter, 64),
            (Q::Const, 4096),
            (Q::Arg, 8),
            (Q::Const, 1),
            (Q::Arg, 12),
            (Q::Const, 608),
            (Q::Arg, 16),
            (Q::Const, 16384),
            (Q::Arg, 20),
            (Q::Const, 640),
            (Q::Arg, 24),
            (Q::Const, -16),
            (Q::Call, 0),
            (Q::Pop, 0),
            (Q::Const, 0),
            (Q::Leave, 64),
        ],
        &[],
    );
    vm.configure_q3_game(qa_core::primitives::Q3StatLayout::BASE);
    for (offset, value) in [(0, 0), (460, 1), (512, 1023)] {
        vm.memory.write_word(4096 + offset, value).unwrap();
    }
    for offset in [24, 92, 488] {
        vm.memory
            .write_vec3(4096 + offset, Vec3([0.0, 0.0, 6.0]))
            .unwrap();
    }
    vm.memory.write_vec3(4096 + 436, Vec3([-1.0; 3])).unwrap();
    vm.memory.write_vec3(4096 + 448, Vec3([1.0; 3])).unwrap();
    vm.memory
        .write_vec3(16384 + 20, Vec3([0.0, 0.0, 6.0]))
        .unwrap();
    vm.memory
        .write_vec3(16384 + 32, Vec3([2.0, 3.0, 4.0]))
        .unwrap();
    vm.memory.write_word(16384 + 148, 1023).unwrap();
    let (mut second, extra) = request(
        &mut runtime,
        B,
        Program::Qvm {
            vm: Box::new(vm),
            imports: &qa_compat::abi::Q3_SERVER,
        },
        vec![0],
        RuleSetId::Quake3,
    );
    runtime.server.entities.columns.native_entity[second.anchor.slot as usize] = None;
    runtime
        .server
        .entities
        .release(extra.unwrap(), ThinkTime::Milliseconds(0));
    second.initialize = vec![super::super::Export::words(
        qa_core::primitives::CallbackId(0),
        &[],
    )];
    let pusher = allocate(&mut runtime, A, 40);
    let id = runtime
        .server
        .connect(connection, B, Some(NativeEntity { module: B, slot: 0 }))
        .unwrap();
    let rider = runtime.server.clients[id.0 as usize].entity;
    runtime.server.clients[id.0 as usize].module_movement = true;
    runtime.server.clients[id.0 as usize].player.movement_rules = RuleSetId::Quake2;
    runtime.server.clients[id.0 as usize].player.trace_rules = RuleSetId::Quake3;
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![first, second],
    )
    .unwrap();
    host.initialize_modules(super::super::Phase::Server);
    assert_eq!(host.module_state(B), Some(crate::modules::State::Running));
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
    edit_quakec(
        &mut host,
        A,
        crate::modules::execution::TraceAccess::empty(),
        |vm, services, context| {
            let offset = 40 * vm.stride as u32;
            for (field, value) in [
                (0, Vec3::default()),
                (9, Vec3([-4.0; 3])),
                (12, Vec3([4.0; 3])),
            ] {
                vector(vm, offset, field, value);
            }
            for (field, value) in [(24, 4.0f32), (33, 7.0), (25, 512.0), (41, 1.0), (31, 100.0)] {
                word(vm, offset, field, value.to_bits());
            }
            let model = vm.insert_string(b"*1").unwrap() as u32;
            word(vm, offset, 27, model);
            vector(vm, offset, 3, Vec3([16.0, 0.0, 0.0]));
            vm.relink_entity(services, context, offset)
        },
    )
    .unwrap();
    projected::transport(
        &mut host,
        NativeEntity { module: B, slot: 0 },
        Vec3([0.0, 0.0, 6.0]),
        true,
    )
    .unwrap();
    host.runtime.server.clients[id.0 as usize]
        .player
        .movement
        .grounded = true;
    host.runtime.server.clients[id.0 as usize]
        .player
        .movement
        .ground = Some(pusher);
    let visibility = host.runtime.collision.as_ref().unwrap().visibility.clone();
    host.runtime.collision = Some(
        WorldCollision::new(&host.runtime.geometry, query, 0).with_visibility(visibility.unwrap()),
    );
    Fixture {
        host,
        pusher,
        rider,
        owner,
        query,
    }
}

#[test]
fn qvm_client_zero_rides_foreign_push_once_without_another_game_export() {
    for (rules, connection) in [RuleSetId::Quake, RuleSetId::QuakeWorld]
        .into_iter()
        .flat_map(|rules| {
            [Connection::Bot, Connection::Local, Connection::Remote]
                .map(|connection| (rules, connection))
        })
    {
        let mut f = qvm_passenger_fixture(rules, connection);
        let before = f.host.module_counts(B).unwrap().calls;
        f.run().unwrap();
        let position = Vec3([1.0, 0.0, 6.0]);
        assert_eq!(
            f.host.runtime.server.entities.columns.position[f.rider.slot as usize],
            position
        );
        let player = &f.host.runtime.server.clients[0].player;
        assert_eq!(player.body.position, position);
        assert_eq!(player.body.velocity, Vec3([2.0, 3.0, 4.0]));
        assert_eq!(player.movement_rules, RuleSetId::Quake2);
        assert_eq!(player.trace_rules, RuleSetId::Quake3);
        assert_eq!(player.movement.ground, Some(f.pusher));
        assert_eq!(f.host.module_counts(B).unwrap().calls, before);
        let Program::Qvm { vm, .. } = &f.host.module_row(B).unwrap().request.program else {
            unreachable!()
        };
        for at in [4096 + 24, 4096 + 92, 4096 + 488, 16384 + 20] {
            assert_eq!(vm.memory.read_vec3(at).unwrap(), position);
        }
        assert!(f.host.runtime.server.area.bounds(f.rider).is_some());
    }
}

#[test]
fn qvm_passenger_rollback_restores_public_player_pose_after_foreign_blocked() {
    let mut f = qvm_passenger_fixture(RuleSetId::Quake, Connection::Bot);
    let native = NativeEntity { module: B, slot: 0 };
    let position = Vec3([5.5, 0.0, 0.0]);
    projected::transport(&mut f.host, native, position, true).unwrap();
    Fixture::body(&mut f.host, A, Vec3([7.5625, 0.0, 0.0]), 2.0, 0);
    f.blocked(4);
    let before = f.host.module_counts(B).unwrap().calls;
    f.run().unwrap();
    assert_eq!(
        f.host.runtime.server.entities.columns.position[f.pusher.slot as usize],
        Vec3::default()
    );
    assert_eq!(
        f.host.runtime.server.clients[0].player.body.position,
        position
    );
    assert_eq!(f.host.module_counts(B).unwrap().calls, before);
    let Program::Qvm { vm, .. } = &f.host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    for at in [4096 + 24, 4096 + 92, 4096 + 488, 16384 + 20] {
        assert_eq!(vm.memory.read_vec3(at).unwrap(), position);
    }
    let Program::QuakeC(qc) = &f.host.module_row(A).unwrap().request.program else {
        unreachable!()
    };
    assert_ne!(qc.vm.image.globals[67], 0);
}

#[test]
fn qvm_passenger_relink_calls_foreign_quakec_trigger_with_the_transported_pose() {
    let mut f = qvm_passenger_fixture(RuleSetId::Quake, Connection::Bot);
    Fixture::body(&mut f.host, A, Vec3([1.0, 0.0, 6.0]), 1.0, 2);
    let before = f.host.module_counts(B).unwrap().calls;
    f.run().unwrap();
    assert_eq!(f.host.module_counts(B).unwrap().calls, before);
    let Program::QuakeC(qc) = &f.host.module_row(A).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(
        qc.vm.image.globals[60..63],
        [1.0f32, 0.0, 6.0].map(f32::to_bits)
    );
    assert_eq!(qc.vm.image.globals[59], 1.0f32.to_bits());
}

#[test]
fn qvm_passenger_impact_keeps_its_capture_after_relink_retires_the_client() {
    use qa_session::physics::DisplacementHost;
    let mut f = qvm_passenger_fixture(RuleSetId::Quake, Connection::Local);
    let obstacle = contacts::capture(&f.host, f.pusher).unwrap();
    let mover = contacts::capture(&f.host, f.rider).unwrap();
    assert!(f.host.runtime.disconnect_client(ClientId(0)));
    assert!(f.host.runtime.server.entities.resolve(f.rider).is_none());
    let before = f.host.module_counts(B).unwrap().calls;
    let trace = MotionTrace {
        trace: qa_world::collision::Trace::clear(Vec3::default()),
        ground: Some(obstacle),
    };
    let mut driver = crate::modules::Driver::cache_only();
    let mut passenger = projected::Passenger {
        host: &mut f.host,
        driver: &mut driver,
        native: NativeEntity { module: B, slot: 0 },
        mover,
        rules: qa_gameplay::rules::motion::push_rules(RuleSetId::Quake).unwrap(),
    };
    assert!(matches!(passenger.impact(None, &trace), Ok(None)));
    assert_eq!(f.host.module_counts(B).unwrap().calls, before);
}

const A: ModuleId = ModuleId(1);
const B: ModuleId = ModuleId(2);

fn program(capacity: usize) -> Vm {
    let mut operations = vec![];
    let mut functions = vec![0];
    let mut function = |body: &[_]| {
        functions.push(operations.len() as i32 + 1);
        operations.extend_from_slice(body);
    };
    function(&[(Return, 0, 0, 0)]); // 1 idle
    function(&[
        // 2 trigger: capture actual recipient clock and contact pose
        (LoadV, 36, 50, 60),
        (StoreF, 35, 63, 0),
        (StoreEnt, 36, 64, 0),
        (StoreEnt, 34, 65, 0),
        (AddF, 59, 48, 59),
        (Return, 60, 0, 0),
    ]);
    function(&[
        // 3 blocked: dereference the admitted foreign Other
        (StoreF, 35, 66, 0),
        (StoreEnt, 36, 67, 0),
        (LoadV, 36, 50, 68),
        (Return, 68, 0, 0),
    ]);
    function(&[
        // 4 blocked: retain weak Other without dereferencing it
        (StoreEnt, 36, 67, 0),
        (StoreF, 35, 66, 0),
        (Return, 67, 0, 0),
    ]);
    function(&[
        // 5 remove only at the restore relink, after the forward touch.
        (AddF, 59, 48, 59),
        (EqF, 59, 108, 127),
        (IfNot, 127, 4, 0),
        (LoadV, 36, 50, 110),
        (StoreEnt, 36, 4, 0),
        (Call1, 73, 0, 0),
        (Return, 0, 0, 0),
    ]);
    function(&[(Call0, 75, 0, 0), (Return, 0, 0, 0)]); // 6 trap
    function(&[
        // 7 replace only at the restore relink, after the forward touch.
        (AddF, 59, 48, 59),
        (EqF, 59, 108, 127),
        (IfNot, 127, 16, 0),
        (LoadV, 36, 50, 110),
        (StoreEnt, 36, 4, 0),
        (Call1, 73, 0, 0),
        (Call0, 74, 0, 0),
        (StoreEnt, 1, 76, 0),
        (Address, 1, 50, 124),
        (StorePV, 100, 124, 0),
        (Address, 1, 51, 124),
        (StorePV, 103, 124, 0),
        (Address, 1, 52, 124),
        (StorePF, 106, 124, 0),
        (Address, 1, 54, 124),
        (StorePF, 77, 124, 0),
        (Address, 1, 55, 124),
        (StorePF, 78, 124, 0),
        (Return, 76, 0, 0),
    ]);
    functions.extend([-15, -14]); // 8 remove, 9 spawn
    let mut fields = FIELDS.to_vec();
    fields.extend_from_slice(&[
        (4, 40, b"groundentity"),
        (2, 41, b"ltime"),
        (6, 42, b"blocked"),
        (2, 43, b"gravity"),
        (2, 44, b"waterlevel"),
        (2, 45, b"watertype"),
        (3, 46, b"avelocity"),
    ]);
    let bytes = quakec_program::program_with_definitions(
        &operations,
        &functions,
        &[(4, 34, b"self"), (2, 35, b"time"), (4, 36, b"other")],
        &fields,
        49,
        128,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: capacity,
            header_bytes: 16,
            extra_string_bytes: 512,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    for (index, word) in [
        (48, 1.0f32.to_bits()),
        (50, 0),
        (51, 3),
        (52, 25),
        (54, 24),
        (55, 33),
        (73, 8),
        (74, 9),
        (75, u32::MAX),
        (77, 2.0f32.to_bits()),
        (78, 3.0f32.to_bits()),
        (108, 2.0f32.to_bits()),
        (106, 32768.0f32.to_bits()),
    ] {
        vm.image.globals[index] = word;
    }
    vm.image.globals[100..103].copy_from_slice(&[123.0f32, 0.0, 0.0].map(f32::to_bits));
    vm.image.globals[103..106].copy_from_slice(&[71.0f32, 0.0, 0.0].map(f32::to_bits));
    vm
}

fn geometry(runtime: &mut Runtime, size: f32) -> qa_core::primitives::GeometryId {
    let planes = [
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        Vec3([0.0, 1.0, 0.0]),
        Vec3([0.0, -1.0, 0.0]),
        Vec3([0.0, 0.0, 1.0]),
        Vec3([0.0, 0.0, -1.0]),
    ]
    .map(|normal| Plane::oriented(normal, size))
    .to_vec();
    runtime
        .geometry
        .load_brushes(
            planes,
            vec![Brush {
                sides: IndexSpan { first: 0, count: 6 },
                contents: Contents::SOLID,
            }],
            Vec::new(),
            SurfaceTable::flags(vec![SurfaceFlags::default(); 6]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![
                    CollisionLeaf {
                        stored_contents: None,
                        brushes: IndexSpan::default(),
                        patches: Default::default(),
                    },
                    CollisionLeaf {
                        stored_contents: None,
                        brushes: IndexSpan { first: 0, count: 1 },
                        patches: Default::default(),
                    },
                ],
                leaf_brushes: vec![0],
                models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                leaf_patches: Vec::new(),
            },
            vec![
                Bounds {
                    mins: Vec3([-512.0; 3]),
                    maxs: Vec3([512.0; 3]),
                },
                Bounds {
                    mins: Vec3([-size; 3]),
                    maxs: Vec3([size; 3]),
                },
            ],
        )
        .unwrap()
}

fn allocate(runtime: &mut Runtime, module: ModuleId, native: i32) -> EntityId {
    let entity = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
        module,
        slot: native,
    });
    entity
}

fn vector(vm: &mut Vm, native: u32, field: u32, value: Vec3) {
    let at = vm.field_address(native, field, 3).unwrap();
    vm.entities.write_vec3(at, value).unwrap();
}
fn word(vm: &mut Vm, native: u32, field: u32, value: u32) {
    let at = vm.field_address(native, field, 1).unwrap();
    vm.entities.write_word(at, value as i32).unwrap();
}

struct Fixture {
    host: FrameHost,
    pusher: EntityId,
    rider: EntityId,
    owner: qa_core::primitives::GeometryId,
    query: qa_core::primitives::GeometryId,
}
impl Fixture {
    fn new(a: RuleSetId, b: RuleSetId, blocked: bool, client: bool) -> Self {
        Self::with_capacity(a, b, blocked, client, 64)
    }
    fn with_capacity(
        a: RuleSetId,
        b: RuleSetId,
        blocked: bool,
        client: bool,
        capacity: usize,
    ) -> Self {
        let mut runtime = Runtime::load(1, []).unwrap();
        let owner = geometry(&mut runtime, 4.0);
        let query = geometry(&mut runtime, 12.0);
        runtime.collision = Some(WorldCollision::new(&runtime.geometry, owner, 0));
        let requests: Vec<ModuleRequest> = [(A, a), (B, b)]
            .into_iter()
            .map(|(module, rules)| {
                request(
                    &mut runtime,
                    module,
                    Program::quakec(program(if module == B { capacity } else { 64 }), None),
                    (0..10).collect(),
                    rules,
                )
                .0
            })
            .collect();
        let pusher = allocate(&mut runtime, A, 40);
        let native = if client { 1 } else { 40 };
        let borrowed = if client {
            let id = runtime
                .server
                .connect(
                    Connection::Bot,
                    B,
                    Some(NativeEntity {
                        module: B,
                        slot: native,
                    }),
                )
                .unwrap();
            runtime.server.clients[id.0 as usize].module_movement = true;
            runtime.server.clients[id.0 as usize].player.trace_rules = RuleSetId::Quake3;
            runtime.server.clients[id.0 as usize].player.movement_rules = RuleSetId::Quake3;
            qa_movement::set_bounds(&mut runtime.server.clients[id.0 as usize].player);
            Some(runtime.server.clients[id.0 as usize].entity)
        } else {
            None
        };
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            requests,
        )
        .unwrap();
        host.time = EventTime::from_milliseconds(100);
        host.module_row_mut(B).unwrap().request.context.clock = ThinkTime::Seconds(42.0);
        // Ordinary native bodies must be owned by the VM allocator. Imported
        // startup rows are borrowed and intentionally cannot be removed by QC.
        let rider = borrowed.unwrap_or_else(|| {
            edit_quakec(
                &mut host,
                B,
                crate::modules::execution::TraceAccess::empty(),
                |vm, services, context| {
                    loop {
                        let offset = vm.spawn_entity(services, context)?;
                        let slot = offset / vm.stride as u32;
                        assert!(slot <= 40, "fixture skipped native passenger40");
                        if slot == 40 {
                            return vm
                                .resolve_entity(&services.server.entities, B, offset)?
                                .ok_or(CallError::Entity);
                        }
                    }
                },
            )
            .unwrap()
        });
        for (module, id, position, solid, motion, mins, maxs) in [
            (
                A,
                pusher,
                Vec3::default(),
                4.0f32,
                7.0f32,
                Vec3([-4.0; 3]),
                Vec3([4.0; 3]),
            ),
            (
                B,
                rider,
                if blocked {
                    Vec3([5.5, 0.0, 0.0])
                } else {
                    Vec3([0.0, 0.0, 6.0])
                },
                2.0,
                3.0,
                Vec3([-1.0; 3]),
                Vec3([1.0; 3]),
            ),
        ] {
            edit_quakec(
                &mut host,
                module,
                crate::modules::execution::TraceAccess::empty(),
                |vm, services, context| {
                    let slot = services.server.entities.columns.native_entity[id.slot as usize]
                        .unwrap()
                        .slot;
                    let offset = slot as u32 * vm.stride as u32;
                    for (field, value) in [(0, position), (9, mins), (12, maxs)] {
                        vector(vm, offset, field, value);
                    }
                    for (field, value) in [
                        (24, solid.to_bits()),
                        (33, motion.to_bits()),
                        (25, 512.0f32.to_bits()),
                    ] {
                        word(vm, offset, field, value);
                    }
                    if module == A {
                        let model = vm.insert_string(b"*1").unwrap() as u32;
                        word(vm, offset, 27, model);
                        word(vm, offset, 41, 1.0f32.to_bits());
                        word(vm, offset, 31, 100.0f32.to_bits());
                        vector(vm, offset, 3, Vec3([16.0, 0.0, 0.0]));
                    }
                    vm.relink_entity(services, context, offset)
                },
            )
            .unwrap();
        }
        let captured = contacts::capture(&host, pusher).unwrap();
        edit_quakec(
            &mut host,
            B,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                vm.write_motion_field(
                    &mut services.server.entities,
                    context,
                    native as u32 * vm.stride as u32,
                    MotionField::Ground(captured),
                )
            },
        )
        .unwrap();
        if blocked {
            Self::body(&mut host, A, Vec3([7.5625, 0.0, 0.0]), 2.0, 0);
        }
        host.runtime.collision = Some(WorldCollision::new(&host.runtime.geometry, query, 0));
        Self {
            host,
            pusher,
            rider,
            owner,
            query,
        }
    }
    fn body(
        host: &mut FrameHost,
        module: ModuleId,
        position: Vec3,
        solid: f32,
        touch: u32,
    ) -> EntityId {
        edit_quakec(
            host,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                let offset = vm.spawn_entity(services, context)?;
                let entity = vm
                    .resolve_entity(&services.server.entities, module, offset)?
                    .ok_or(CallError::Entity)?;
                for (field, value) in [(0, position), (9, Vec3([-1.0; 3])), (12, Vec3([1.0; 3]))] {
                    vector(vm, offset, field, value);
                }
                word(vm, offset, 24, solid.to_bits());
                word(vm, offset, 30, touch);
                vm.relink_entity(services, context, offset)?;
                Ok(entity)
            },
        )
        .unwrap()
    }
    fn blocked(&mut self, callback: u32) {
        edit_quakec(
            &mut self.host,
            A,
            crate::modules::execution::TraceAccess::empty(),
            |vm, _, _| {
                word(vm, 40 * vm.stride as u32, 42, callback);
                Ok(())
            },
        )
        .unwrap();
    }
    fn run(&mut self) -> Result<(), ClientCommandError> {
        push(
            &mut self.host,
            A,
            self.pusher,
            ThinkTime::Seconds(77.0),
            0.0625,
            &mut crate::modules::Driver::cache_only(),
        )
    }
    fn raw_position(&mut self, module: ModuleId, native: i32) -> Vec3 {
        edit_quakec(
            &mut self.host,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, _, _| {
                let at = vm
                    .field_address(native as u32 * vm.stride as u32, 0, 3)
                    .map_err(|_| CallError::Memory)?;
                Ok(vm.entities.read_vec3(at)?)
            },
        )
        .unwrap()
    }
}

#[test]
fn foreign_primary_passenger_routes_once_through_its_owner_and_retained_geometry() {
    for (a, b) in [
        (RuleSetId::Quake, RuleSetId::QuakeWorld),
        (RuleSetId::QuakeWorld, RuleSetId::Quake),
    ] {
        let mut f = Fixture::new(a, b, false, false);
        let captured = contacts::capture(&f.host, f.rider).unwrap();
        let admitted = contacts::admit_captured(
            &mut f.host,
            A,
            crate::modules::execution::TraceAccess::empty(),
            captured,
        )
        .unwrap();
        let alias = admitted.reference().ordinal as i32;
        contacts::release(&mut f.host, admitted).unwrap();
        let Program::QuakeC(qc) = &f.host.module_row(B).unwrap().request.program else {
            unreachable!()
        };
        let first_foreign = qc.vm.client_count().unwrap() as i32 + 1;
        let mut driver = crate::modules::Driver::cache_only();
        let mut adapter = Host::new(
            &mut f.host,
            A,
            f.pusher,
            ThinkTime::Seconds(77.0),
            &mut driver,
        )
        .unwrap();
        let mut cursor = adapter.start_candidates();
        let mut candidates = vec![];
        while let Some(candidate) = adapter.next_candidate(&mut cursor).unwrap() {
            candidates.push(candidate);
        }
        assert!(!candidates.contains(&CollisionOwner::Native(NativeEntity {
            module: A,
            slot: alias
        })));
        assert_eq!(
            candidates.first(),
            Some(&CollisionOwner::Native(NativeEntity { module: A, slot: 1 }))
        );
        let foreign_start = candidates
            .iter()
            .position(|owner| matches!(owner,CollisionOwner::Native(native) if native.module==B))
            .unwrap();
        assert!(
            candidates[..foreign_start]
                .iter()
                .all(|owner| matches!(owner,CollisionOwner::Native(native) if native.module==A))
        );
        let foreign: Vec<_> = candidates[foreign_start..]
            .iter()
            .map(|owner| {
                let CollisionOwner::Native(native) = owner else {
                    unreachable!()
                };
                assert_eq!(native.module, B);
                native.slot
            })
            .collect();
        assert_eq!(foreign, (first_foreign..=40).collect::<Vec<_>>());
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| **candidate
                    == CollisionOwner::Native(NativeEntity {
                        module: B,
                        slot: 40
                    }))
                .count(),
            1
        );
        assert_eq!(
            candidates.last(),
            Some(&CollisionOwner::Native(NativeEntity {
                module: B,
                slot: 40
            }))
        );
        drop(adapter);
        f.run().unwrap();
        assert_eq!(f.raw_position(A, 40), Vec3([1.0, 0.0, 0.0]));
        assert_eq!(f.raw_position(B, 40), Vec3([1.0, 0.0, 6.0]));
        assert_eq!(
            f.host.runtime.server.entities.columns.position[f.rider.slot as usize],
            Vec3([1.0, 0.0, 6.0])
        );
        assert_eq!(
            f.host.runtime.server.entities.columns.collision_shape[f.pusher.slot as usize],
            qa_core::primitives::CollisionShape::Model {
                geometry: f.owner,
                index: 1
            }
        );
        assert_ne!(f.owner, f.query);
        let bounds = f.host.runtime.server.area.bounds(f.rider).unwrap();
        assert_eq!(bounds.mins, Vec3([-1.0, -2.0, 4.0]));
        for module in [A, B] {
            assert_eq!(
                (
                    f.host.module_counts(module).unwrap().rejected,
                    f.host.module_counts(module).unwrap().traps
                ),
                (0, 0)
            );
        }
    }
}

#[test]
fn foreign_blocked_other_reads_restored_pose_and_trap_releases_its_lease() {
    for a in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut f = Fixture::new(a, RuleSetId::QuakeWorld, true, false);
        f.blocked(3);
        f.run().unwrap();
        assert_eq!(f.raw_position(A, 40), Vec3::default());
        assert_eq!(f.raw_position(B, 40), Vec3([5.5, 0.0, 0.0]));
        let Program::QuakeC(qc) = &f.host.module_row(A).unwrap().request.program else {
            unreachable!()
        };
        assert_eq!(qc.vm.image.globals[68], 5.5f32.to_bits());
        assert_ne!(qc.vm.image.globals[67], 0);
        f.blocked(6);
        assert!(f.run().is_err());
        assert_eq!(f.host.module_counts(A).unwrap().traps, 1);
        f.blocked(3);
        f.run().unwrap();
        assert_eq!(f.host.module_counts(A).unwrap().traps, 1);
    }
}

#[test]
fn foreign_client_passenger_keeps_selected_trace_and_publishes_hot_pose_without_a_packet() {
    for b in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut f = Fixture::new(RuleSetId::Quake, b, false, true);
        f.host
            .module_row_mut(B)
            .unwrap()
            .request
            .context
            .trace_rules = RuleSetId::Quake;
        let client = f
            .host
            .runtime
            .server
            .entities
            .client_index(f.rider)
            .unwrap();
        let selected = f.host.runtime.server.clients[client].player.body;
        f.run().unwrap();
        let raw = f.raw_position(B, 1);
        let player = &f.host.runtime.server.clients[client].player;
        assert_eq!(raw, Vec3([1.0, 0.0, 6.0]));
        assert_eq!(
            f.host.runtime.server.entities.columns.position[f.rider.slot as usize],
            raw
        );
        assert_eq!(player.trace_rules, RuleSetId::Quake3);
        if b == RuleSetId::QuakeWorld {
            assert_eq!(player.body.position, raw + Vec3([-1.0; 3]) - selected.mins);
            assert_eq!(
                (player.body.mins, player.body.maxs),
                (selected.mins, selected.maxs)
            );
        } else {
            assert_eq!(player.body.position, raw);
        }
    }
}

#[test]
fn foreign_passenger_trigger_uses_recipient_clock() {
    for b in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut f = Fixture::new(RuleSetId::Quake, b, false, false);
        Fixture::body(&mut f.host, B, Vec3([1.0, 0.0, 6.0]), 1.0, 2);
        let clock = f.host.module_row(B).unwrap().time(f.host.time).seconds() as f32;
        assert_ne!(clock, 77.0);
        f.run().unwrap();
        let Program::QuakeC(qc) = &f.host.module_row(B).unwrap().request.program else {
            unreachable!()
        };
        assert_eq!(qc.vm.image.globals[59], 1.0f32.to_bits());
        assert_eq!(qc.vm.image.globals[63], clock.to_bits());
        assert_eq!(
            &qc.vm.image.globals[60..63],
            &[1.0f32.to_bits(), 0, 6.0f32.to_bits()]
        );
    }
}

#[test]
fn foreign_blocker_retired_by_restore_trigger_keeps_nonzero_weak_other() {
    for callback in [3, 4] {
        let mut f = Fixture::new(RuleSetId::Quake, RuleSetId::QuakeWorld, true, false);
        Fixture::body(&mut f.host, B, Vec3([5.5, 0.0, 0.0]), 1.0, 5);
        f.blocked(callback);
        assert_eq!(f.run().is_err(), callback == 3);
        assert!(f.host.runtime.server.entities.resolve(f.rider).is_none());
        let Program::QuakeC(qc) = &f.host.module_row(B).unwrap().request.program else {
            unreachable!()
        };
        assert_eq!(qc.vm.image.globals[59], 2.0f32.to_bits());
        assert_eq!(&qc.vm.image.globals[110..113], &[5.5f32.to_bits(), 0, 0]);
        assert_eq!(f.raw_position(A, 40), Vec3::default());
        let Program::QuakeC(qc) = &f.host.module_row(A).unwrap().request.program else {
            unreachable!()
        };
        let other = qc.vm.image.globals[67];
        assert_ne!(other, 0);
        assert_eq!(
            qc.vm
                .resolve_entity(&f.host.runtime.server.entities, A, other)
                .unwrap(),
            None
        );
        assert_eq!(
            f.host.module_counts(A).unwrap().traps,
            u64::from(callback == 3)
        );
        f.blocked(4);
        f.run().unwrap();
        assert_eq!(f.raw_position(A, 40), Vec3([1.0, 0.0, 0.0]));
    }
}

#[test]
fn foreign_blocked_other_rebinds_reused_native_row_and_undo_restores_only_origin() {
    let mut f = Fixture::new(RuleSetId::Quake, RuleSetId::QuakeWorld, true, false);
    // The early edict grace interval permits immediate reuse within this callback.
    f.host.module_row_mut(B).unwrap().request.context.clock = ThinkTime::Seconds(0.1);
    // Setup allocated all QW primaries33..40 through Spawn. The lower live
    // rows force the trigger's Spawn to reclaim the passenger's native40.
    let Program::QuakeC(qc) = &f.host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    for slot in 33..40 {
        assert!(
            qc.vm
                .bound_entity(
                    &f.host.runtime.server.entities,
                    NativeEntity { module: B, slot }
                )
                .unwrap()
                .is_some()
        );
    }
    Fixture::body(&mut f.host, B, Vec3([5.5, 0.0, 0.0]), 1.0, 7);
    f.blocked(3);
    f.run().unwrap();
    assert!(f.host.runtime.server.entities.resolve(f.rider).is_none());
    let Program::QuakeC(qc) = &f.host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(qc.vm.image.globals[59], 2.0f32.to_bits());
    assert_eq!(&qc.vm.image.globals[110..113], &[5.5f32.to_bits(), 0, 0]);
    let native = NativeEntity {
        module: B,
        slot: 40,
    };
    let replacement = f
        .host
        .runtime
        .server
        .entities
        .find_native(native, None)
        .unwrap();
    assert_ne!(replacement, f.rider);
    let slot = replacement.slot as usize;
    assert_eq!(
        f.host.runtime.server.entities.columns.position[slot],
        Vec3([5.5, 0.0, 0.0])
    );
    assert_eq!(
        f.host.runtime.server.entities.columns.velocity[slot],
        Vec3([71.0, 0.0, 0.0])
    );
    assert_eq!(f.host.runtime.server.entities.columns.flags[slot], 32768);
    let Program::QuakeC(qc) = &f.host.module_row(A).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(&qc.vm.image.globals[68..71], &[123.0f32.to_bits(), 0, 0]);
    assert_eq!(
        qc.vm
            .resolve_entity(&f.host.runtime.server.entities, A, qc.vm.image.globals[67])
            .unwrap(),
        Some(replacement)
    );
}

fn fill_aliases(host: &mut FrameHost, recipient: ModuleId) -> Vec<EntityId> {
    let Program::QuakeC(qc) = &host.module_row(recipient).unwrap().request.program else {
        unreachable!()
    };
    let capacity = qc.vm.entities.len() / qc.vm.stride;
    let mut aliases = vec![];
    for _ in 0..=capacity {
        let entity = host
            .runtime
            .server
            .entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(90),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let capture = contacts::capture(host, entity).unwrap();
        match contacts::admit_captured(
            host,
            recipient,
            crate::modules::execution::TraceAccess::empty(),
            capture,
        ) {
            Ok(admitted) => {
                contacts::release(host, admitted).unwrap();
                aliases.push(entity);
            }
            Err(ClientCommandError::Usercmd(CallError::Capacity)) => return aliases,
            Err(error) => panic!("unexpected alias admission: {error:?}"),
        }
    }
    panic!("recipient exceeded its backing capacity")
}

#[test]
fn foreign_blocked_without_callback_does_not_admit_other_into_a_full_recipient() {
    let mut f = Fixture::new(RuleSetId::Quake, RuleSetId::QuakeWorld, true, false);
    assert!(!fill_aliases(&mut f.host, A).is_empty());
    f.run().unwrap();
    assert_eq!(f.raw_position(A, 40), Vec3::default());
    assert_eq!(f.raw_position(B, 40), Vec3([5.5, 0.0, 0.0]));
    let capture = contacts::capture(&f.host, f.rider).unwrap();
    assert!(matches!(
        contacts::admit_captured(
            &mut f.host,
            A,
            crate::modules::execution::TraceAccess::empty(),
            capture
        ),
        Err(ClientCommandError::Usercmd(CallError::Capacity))
    ));
}

#[test]
fn foreign_passenger_count_can_exceed_pushers_native_backing_capacity() {
    let mut f = Fixture::with_capacity(RuleSetId::Quake, RuleSetId::QuakeWorld, false, false, 128);
    let pusher = contacts::capture(&f.host, f.pusher).unwrap();
    let mut passengers = vec![f.rider];
    for index in 1..65 {
        let entity = Fixture::body(&mut f.host, B, Vec3([0.0, index as f32 * 4.0, 6.0]), 2.0, 0);
        edit_quakec(
            &mut f.host,
            B,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                let native =
                    services.server.entities.columns.native_entity[entity.slot as usize].unwrap();
                let offset = native.slot as u32 * vm.stride as u32;
                word(vm, offset, 25, 512.0f32.to_bits());
                word(vm, offset, 33, 3.0f32.to_bits());
                vm.write_motion_field(
                    &mut services.server.entities,
                    context,
                    offset,
                    MotionField::Ground(pusher),
                )
            },
        )
        .unwrap();
        passengers.push(entity);
    }
    assert_eq!(passengers.len(), 65);
    f.run().unwrap();
    for (index, entity) in passengers.into_iter().enumerate() {
        assert_eq!(
            f.host.runtime.server.entities.columns.position[entity.slot as usize],
            Vec3([1.0, index as f32 * 4.0, 6.0])
        );
    }
}

#[test]
fn saved_foreign_native_origin_store_rejects_alias_takeover_before_any_write() {
    let mut f = Fixture::new(RuleSetId::Quake, RuleSetId::QuakeWorld, true, false);
    f.host.time = EventTime::from_milliseconds(100_000);
    f.host.module_row_mut(B).unwrap().request.context.clock = ThinkTime::Seconds(100.0);
    edit_quakec(
        &mut f.host,
        B,
        crate::modules::execution::TraceAccess::empty(),
        |vm, services, context| vm.remove_entity(services, context, 40 * vm.stride as u32),
    )
    .unwrap();
    // Reclaim through the existing edict delay, without changing private bindings.
    f.host.time = EventTime::from_milliseconds(100_600);
    f.host.module_row_mut(B).unwrap().request.context.clock = ThinkTime::Seconds(100.6);
    let aliases = fill_aliases(&mut f.host, B);
    let Program::QuakeC(qc) = &f.host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    let target = qc
        .vm
        .resolve_entity(&f.host.runtime.server.entities, B, 40 * qc.vm.stride as u32)
        .unwrap()
        .unwrap();
    assert!(aliases.contains(&target));
    assert_eq!(
        qc.vm
            .bound_entity(
                &f.host.runtime.server.entities,
                NativeEntity {
                    module: B,
                    slot: 40
                }
            )
            .unwrap(),
        None
    );
    let body = f
        .host
        .runtime
        .server
        .entities
        .columns
        .body(target.slot as usize);
    let mut driver = crate::modules::Driver::cache_only();
    let mut adapter = Host::new(
        &mut f.host,
        A,
        f.pusher,
        ThinkTime::Seconds(77.0),
        &mut driver,
    )
    .unwrap();
    assert!(matches!(
        PushHost::store(
            &mut adapter,
            CollisionOwner::Native(NativeEntity {
                module: B,
                slot: 40
            }),
            PushNumber::Motion(MotionWrite::Origin(0)),
            999.0
        ),
        Err(ClientCommandError::Usercmd(CallError::Unsupported))
    ));
    drop(adapter);
    let after = f
        .host
        .runtime
        .server
        .entities
        .columns
        .body(target.slot as usize);
    assert_eq!(
        (after.position, after.velocity, after.mins, after.maxs),
        (body.position, body.velocity, body.mins, body.maxs)
    );
}
