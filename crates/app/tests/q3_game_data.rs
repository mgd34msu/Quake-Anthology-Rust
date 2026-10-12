#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;
#[path = "support/visibility_world.rs"]
mod visibility_world;

use qa_app::Runtime;
use qa_compat::{
    abi::{Q3_SERVER, QvmCalls, UnknownCalls},
    qvm::{SystemCalls, Trap, Vm},
    services::{CallContext, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{CollisionOwner, EntityId, ModuleId, NativeEntity, RuleSetId, ThinkTime, Vec3},
    sys_events::EventTime,
};
use qa_world::{
    area::{LinkFlags, LinkOrder},
    entities::AllocationPolicy,
    visibility::PvsRows,
};

const ENTITIES: u64 = 4096;
const STRIDE: u64 = 608;
const PLAYERS: u64 = 16384;

struct Fixture {
    runtime: Runtime,
    console: Console<Runtime>,
    storage: ServiceStorage,
    vm: Vm,
    unknown: UnknownCalls,
    trace_rules: RuleSetId,
}
fn context() -> CallContext {
    CallContext {
        module: ModuleId(1),
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
        server_frame: 0,
        clock: ThinkTime::Milliseconds(0),
        console: Context::default(),
        allocation: AllocationPolicy::q3(0),
        link_order: LinkOrder::Head,
    }
}
impl Fixture {
    fn new() -> Self {
        Self::with_clients(1)
    }
    fn with_clients(clients: usize) -> Self {
        let mut runtime = Runtime::load(clients, []).unwrap();
        runtime.collision = Some(visibility_world::collision(
            &runtime.geometry,
            [Some(0), Some(1)],
            PvsRows::load(vec![Some(0), Some(1)], vec![3, 3]).unwrap(),
        ));
        let console = Console::new(Context::default()).unwrap();
        let storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut vm =
            service_program::program(&[(qa_formats::program::qvm::Opcode::Ignore, 0)], &[]);
        vm.configure_q3_game();
        let values = runtime.catalog.hud.values.native(RuleSetId::Quake3);
        vm.bind_player_values(&values.stats, &values.persistent)
            .unwrap();
        Self {
            runtime,
            console,
            storage,
            vm,
            unknown: UnknownCalls::load(1).unwrap(),
            trace_rules: RuleSetId::Quake,
        }
    }
    fn call(&mut self, number: u32, arguments: &[i32]) -> Result<i32, Trap> {
        let mut scratch = self.runtime.geometry.scratch();
        let mut services =
            self.runtime
                .engine_services(&mut self.console, &mut self.storage, &mut scratch);
        QvmCalls {
            services: &mut services,
            table: &Q3_SERVER,
            context: CallContext {
                trace_rules: self.trace_rules,
                ..context()
            },
            platform_time: EventTime(0),
            command: &[],
            unknown: &mut self.unknown,
        }
        .call(&mut self.vm, number, arguments)
    }
    fn publish(&mut self) {
        let mut scratch = self.runtime.geometry.scratch();
        let mut services =
            self.runtime
                .engine_services(&mut self.console, &mut self.storage, &mut scratch);
        self.vm.publish_entities(&mut services, context()).unwrap();
    }
    fn register(&mut self, address: u64, count: i32) -> Result<i32, Trap> {
        self.call(
            15,
            &[0, address as i32, count, STRIDE as i32, PLAYERS as i32, 512],
        )
    }
    fn row(&mut self, address: u64, number: i32, contents: i32) {
        self.vm
            .memory
            .read_mut(address, STRIDE as usize)
            .unwrap()
            .fill(0);
        for (offset, value) in [
            (0, number),
            (160, 9),
            (172, 11),
            (416, 257),
            (460, contents),
            (512, 1022),
        ] {
            self.vm.memory.write_word(address + offset, value).unwrap();
        }
        for (offset, value) in [
            (92, Vec3([number as f32, 0.0, 0.0])),
            (116, Vec3([0.0, 45.0, 0.0])),
            (436, Vec3([-1.0; 3])),
            (448, Vec3([1.0; 3])),
            (488, Vec3([number as f32 * 2.0 - 2.0, 0.0, 0.0])),
        ] {
            self.vm.memory.write_vec3(address + offset, value).unwrap();
        }
    }
    fn binding(&self, number: i32) -> EntityId {
        self.runtime
            .server
            .entities
            .active()
            .find(|id| {
                self.runtime.server.entities.columns.native_entity[id.slot as usize]
                    == Some(NativeEntity {
                        module: ModuleId(1),
                        slot: number,
                    })
            })
            .unwrap()
    }
    fn candidates(&self, role: LinkFlags) -> Vec<EntityId> {
        self.runtime
            .server
            .area
            .query(
                &self.runtime.server.entities,
                qa_core::primitives::Bounds {
                    mins: Vec3([-10.0; 3]),
                    maxs: Vec3([10.0; 3]),
                },
                role,
            )
            .map(|body| body.id)
            .collect()
    }
    fn empty_collision(&mut self) {
        let geometry = self
            .runtime
            .geometry
            .load_brushes(
                vec![],
                vec![],
                qa_world::collision::surfaces::SurfaceTable::flags(vec![]),
                qa_world::collision::brushes::BrushTree::direct(0).unwrap(),
                vec![qa_core::primitives::Bounds {
                    mins: Vec3([-100.0; 3]),
                    maxs: Vec3([100.0; 3]),
                }],
            )
            .unwrap();
        let collision = self.runtime.collision.as_mut().unwrap();
        collision.geometry = geometry;
        collision.scratch = self.runtime.geometry.scratch();
    }
    fn collision_model(&mut self) {
        use qa_core::primitives::{Bounds, IndexSpan, Plane, SurfaceFlags};
        use qa_world::collision::{
            Contents,
            brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
            surfaces::SurfaceTable,
        };
        let planes = (0..6)
            .map(|side| {
                Plane::oriented(
                    Vec3::axis(side / 2, if side % 2 == 0 { 1.0 } else { -1.0 }),
                    1.0,
                )
            })
            .collect();
        let geometry = self
            .runtime
            .geometry
            .load_brushes(
                planes,
                vec![Brush {
                    sides: IndexSpan { first: 0, count: 6 },
                    contents: Contents::SOLID,
                }],
                SurfaceTable::flags(vec![SurfaceFlags::from_q3(0x2002); 6]),
                BrushTree {
                    planes: vec![],
                    nodes: vec![],
                    leaves: vec![
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan::default(),
                        },
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan { first: 0, count: 1 },
                        },
                    ],
                    leaf_brushes: vec![0],
                    models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                },
                vec![
                    Bounds {
                        mins: Vec3([-10.0; 3]),
                        maxs: Vec3([10.0; 3]),
                    },
                    Bounds {
                        mins: Vec3([-2.0; 3]),
                        maxs: Vec3([2.0; 3]),
                    },
                ],
            )
            .unwrap();
        let collision = self.runtime.collision.as_mut().unwrap();
        collision.geometry = geometry;
        collision.scratch = self.runtime.geometry.scratch();
        self.trace_rules = RuleSetId::Quake3;
    }
    fn vector(&mut self, address: u64, value: Vec3) -> i32 {
        self.vm.memory.write_vec3(address, value).unwrap();
        address as i32
    }
    fn trace(&mut self, start: Vec3, end: Vec3, pass: i32) -> i32 {
        let start = self.vector(600, start);
        let end = self.vector(616, end);
        self.vm.memory.read_mut(512, 64).unwrap().fill(0xa5);
        assert_eq!(self.call(24, &[0, 512, start, 0, 0, end, pass, -1]), Ok(0));
        assert_eq!(self.vm.memory.read(568, 8).unwrap(), &[0xa5; 8]);
        self.vm.memory.read_word(564).unwrap()
    }
}

#[test]
fn qvm_trace_and_area_query_before_locate_need_no_module_rows() {
    let mut f = Fixture::new();
    f.collision_model();
    assert_eq!(
        f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 1023),
        1023
    );
    let mins = f.vector(600, Vec3([-10.0; 3]));
    let maxs = f.vector(616, Vec3([10.0; 3]));
    f.vm.memory.read_mut(700, 16).unwrap().fill(0xa5);
    assert_eq!(f.call(32, &[0, mins, maxs, 700, 4]), Ok(0));
    assert_eq!(f.vm.memory.read(700, 16).unwrap(), &[0xa5; 16]);
    assert!(f.call(32, &[0, mins, maxs, 0, 4]).is_err());
    f.runtime.collision.as_mut().unwrap().index = 1;
    assert_eq!(f.trace(Vec3::default(), Vec3::default(), 1023), 1022);
    assert_eq!(
        f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32),
        0.0
    );
    assert_eq!(f.trace(Vec3::default(), Vec3::default(), 1022), 1022);
}

#[test]
fn qvm_world_pass_is_a_weak_identity_without_an_ordinary_world_row() {
    let mut f = Fixture::new();
    f.collision_model();
    f.register(ENTITIES, 3).unwrap();
    f.runtime.collision.as_mut().unwrap().index = 1;
    assert_eq!(f.trace(Vec3::default(), Vec3::default(), 1022), 1022);
    f.runtime.collision.as_mut().unwrap().index = 0;
    f.row(ENTITIES, 0, 1);
    f.vm.memory.write_word(ENTITIES + 512, 1023).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    assert_eq!(
        f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 1022),
        0
    );
    f.vm.memory.write_word(ENTITIES + 512, 1022).unwrap();
    assert_eq!(
        f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 1022),
        1023
    );
}

#[test]
fn qvm_capsule_queries_reject_explicitly_without_box_results() {
    let mut f = Fixture::new();
    f.collision_model();
    f.register(ENTITIES, 3).unwrap();
    f.row(ENTITIES, 0, 1);
    f.vm.memory.read_mut(512, 56).unwrap().fill(0xa5);
    for call in [43, 44] {
        assert!(f.call(call, &[0, 512, 600, 616, 0, 0, 1023, -1]).is_err());
    }
    assert_eq!(f.unknown.calls, 0);
    assert_eq!(f.vm.memory.read(512, 56).unwrap(), &[0xa5; 56]);
    f.vm.memory.write_word(ENTITIES + 424, 0x400).unwrap();
    let mins = f.vector(600, Vec3([-1.0; 3]));
    let maxs = f.vector(616, Vec3([1.0; 3]));
    assert!(f.call(33, &[0, mins, maxs, ENTITIES as i32]).is_err());
}

#[test]
fn qvm_trace_and_contact_use_live_targets_native_zero_and_raw_null_bounds() {
    let mut f = Fixture::new();
    f.collision_model();
    f.register(ENTITIES, 3).unwrap();
    f.row(ENTITIES, 0, 1);
    f.row(ENTITIES + STRIDE, 1, 1);
    f.vm.memory.write_word(ENTITIES + 512, 1023).unwrap();
    f.vm.memory
        .write_word(ENTITIES + STRIDE + 512, 1023)
        .unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    f.call(30, &[0, (ENTITIES + STRIDE) as i32]).unwrap();
    assert_eq!(
        f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 1023),
        0
    );
    assert_eq!(f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 0), 1);
    let bounds = f.runtime.server.area.bounds(f.binding(1));
    let pose = f.runtime.server.entities.columns.position[f.binding(1).slot as usize];
    f.vm.memory
        .write_vec3(ENTITIES + STRIDE + 488, Vec3([4.0, 0.0, 0.0]))
        .unwrap();
    assert_eq!(f.trace(Vec3([0.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 0), 1);
    assert!(f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32) > 0.5);
    assert_eq!(f.runtime.server.area.bounds(f.binding(1)), bounds);
    assert_eq!(
        f.runtime.server.entities.columns.position[f.binding(1).slot as usize],
        pose
    );
    f.call(31, &[0, (ENTITIES + STRIDE) as i32]).unwrap();
    let mins = f.vector(640, Vec3([3.5, -0.5, -0.5]));
    let maxs = f.vector(656, Vec3([4.5, 0.5, 0.5]));
    let live = f.runtime.server.entities.active().count();
    f.vm.memory
        .write_vec3(ENTITIES + STRIDE + 500, Vec3([f32::NAN; 3]))
        .unwrap();
    assert_eq!(
        f.call(33, &[0, mins, maxs, (ENTITIES + STRIDE) as i32]),
        Ok(1)
    );
    assert_eq!(f.runtime.server.entities.active().count(), live);
    assert!(f.runtime.server.area.bounds(f.binding(1)).is_none());
    // Nonzero raw bounds pointers masked to zero refer to real vectors.
    f.vm.memory.write_vec3(0, Vec3([1.0, 0.0, 0.0])).unwrap();
    f.vm.memory.write_vec3(16, Vec3([1.0, 0.0, 0.0])).unwrap();
    let start = f.vector(600, Vec3([-5.0, 0.0, 0.0]));
    let end = f.vector(616, Vec3([5.0, 0.0, 0.0]));
    assert_eq!(
        f.call(24, &[0, 512, start, 65536, 65552, end, 1023, -1]),
        Ok(0)
    );
    let shifted = f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32);
    assert_eq!(
        f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 1023),
        0
    );
    assert!(shifted < f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32));
    assert_eq!(
        f.call(24, &[0, 0, start, 0, 0, end, 1023, 1]),
        Err(Trap::Syscall)
    );
}

#[test]
fn qvm_trace_keeps_caller_rules_and_native_number_distinct_from_area_slot() {
    let mut f = Fixture::new();
    f.collision_model();
    f.register(ENTITIES, 3).unwrap();
    f.row(ENTITIES, 0, 1);
    f.vm.memory.write_word(ENTITIES + 512, 1023).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    let start = Vec3([-5.0, 0.0, 0.0]);
    let end = Vec3([5.0, 0.0, 0.0]);
    assert_eq!(f.trace(start, end, 1023), 0);
    assert_eq!(
        f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32),
        0.1875
    );
    f.trace_rules = RuleSetId::Quake2;
    assert_eq!(f.trace(start, end, 1023), 0);
    assert_eq!(
        f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32),
        0.196875
    );
    f.trace_rules = RuleSetId::Quake3;
    f.vm.memory.write_word(ENTITIES, 7).unwrap();
    assert_eq!(f.trace(start, end, 1023), 7);
    let mins = f.vector(600, Vec3([-10.0; 3]));
    let maxs = f.vector(616, Vec3([10.0; 3]));
    assert_eq!(f.call(32, &[0, mins, maxs, 700, 1]), Ok(1));
    assert_eq!(f.vm.memory.read_word(700).unwrap(), 0);
    // Startsolid alone with fraction one retains the clear native NONE.
    assert_eq!(f.trace(Vec3([-2.0, 0.0, 0.0]), end, 1023), 1023);
    assert_eq!(f.vm.memory.read_word(516).unwrap(), 1);
    assert_eq!(
        f32::from_bits(f.vm.memory.read_word(520).unwrap() as u32),
        1.0
    );
}

#[test]
fn qvm_unlinked_box_contact_requires_no_world_and_ignores_box_angles() {
    let mut f = Fixture::new();
    f.runtime.collision = None;
    f.trace_rules = RuleSetId::Quake3;
    f.register(ENTITIES, 1).unwrap();
    f.row(ENTITIES, 0, 1);
    f.vm.memory
        .write_vec3(ENTITIES + 500, Vec3([f32::NAN; 3]))
        .unwrap();
    let mins = f.vector(640, Vec3([-2.5, -0.5, -0.5]));
    let maxs = f.vector(656, Vec3([-1.5, 0.5, 0.5]));
    let live = f.runtime.server.entities.active().count();
    assert_eq!(f.call(33, &[0, mins, maxs, ENTITIES as i32]), Ok(1));
    assert_eq!(f.runtime.server.entities.active().count(), live);
}

#[test]
fn qvm_brush_model_uses_inline_ordinals_and_contact_does_not_link_a_target() {
    let mut f = Fixture::new();
    f.collision_model();
    f.register(ENTITIES, 3).unwrap();
    f.row(ENTITIES, 0, 1);
    f.vm.memory.write_string(256, 16, b"*1").unwrap();
    let events = f.runtime.server.events.sequence_range();
    assert_eq!(f.call(23, &[0, ENTITIES as i32, 256]), Ok(0));
    for (offset, value) in [(160, 1), (432, 1), (460, -1), (420, 1), (176, 0x00ff_ffff)] {
        assert_eq!(f.vm.memory.read_word(ENTITIES + offset).unwrap(), value);
    }
    assert_eq!(
        f.vm.memory.read_vec3(ENTITIES + 436).unwrap(),
        Vec3([-2.0; 3])
    );
    assert_eq!(
        f.vm.memory.read_vec3(ENTITIES + 448).unwrap(),
        Vec3([2.0; 3])
    );
    assert_eq!(f.runtime.server.events.sequence_range(), events);
    assert_eq!(
        f.trace(Vec3([-5.0, 0.0, 0.0]), Vec3([5.0, 0.0, 0.0]), 1023),
        0
    );
    assert_eq!(f.vm.memory.read_word(556).unwrap() as u32, 0x2002);
    f.call(31, &[0, ENTITIES as i32]).unwrap();
    f.vm.memory
        .write_vec3(ENTITIES + 500, Vec3([0.0, 45.0, 0.0]))
        .unwrap();
    let mins = f.vector(640, Vec3([-2.1, -0.1, -0.1]));
    let maxs = f.vector(656, Vec3([-1.9, 0.1, 0.1]));
    let before = f.runtime.server.entities.active().count();
    assert_eq!(f.call(33, &[0, mins, maxs, ENTITIES as i32]), Ok(1));
    assert_eq!(f.runtime.server.entities.active().count(), before);
    assert!(f.runtime.server.area.bounds(f.binding(0)).is_none());
    f.vm.memory.write_string(256, 16, b"*0").unwrap();
    assert_eq!(f.call(23, &[0, ENTITIES as i32, 256]), Ok(0));
    assert_eq!(f.vm.memory.read_word(ENTITIES + 160).unwrap(), 0);
    for name in [b"model".as_slice(), b"*-1", b"*2"] {
        f.vm.memory.write_string(256, 16, name).unwrap();
        assert_eq!(f.call(23, &[0, ENTITIES as i32, 256]), Err(Trap::Syscall));
    }
}

#[test]
fn qvm_entities_in_box_writes_native_ints_for_all_linked_roles_in_order() {
    let mut f = Fixture::new();
    f.collision_model();
    f.register(ENTITIES, 3).unwrap();
    for number in 0..3 {
        let address = ENTITIES + number * STRIDE;
        f.row(address, number as i32, if number == 2 { 0 } else { 1 });
        f.call(30, &[0, address as i32]).unwrap();
    }
    let mins = f.vector(640, Vec3([-10.0; 3]));
    let maxs = f.vector(656, Vec3([10.0; 3]));
    f.vm.memory.read_mut(1024, 20).unwrap().fill(0xa5);
    assert_eq!(f.call(32, &[0, mins, maxs, 1024, 3]), Ok(3));
    assert_eq!(
        (0..3)
            .map(|index| f.vm.memory.read_word(1024 + index * 4).unwrap())
            .collect::<Vec<_>>(),
        vec![2, 1, 0]
    );
    assert_eq!(f.vm.memory.read(1036, 8).unwrap(), &[0xa5; 8]);
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    assert_eq!(f.call(32, &[0, mins, maxs, 1024, 1]), Ok(1));
    assert_eq!(f.vm.memory.read_word(1024).unwrap(), 0);
    assert_eq!(f.call(32, &[0, mins, maxs, 0, 0]), Ok(0));
    assert_eq!(f.call(32, &[0, mins, maxs, 1024, -1]), Err(Trap::Syscall));
    assert_eq!(f.call(32, &[0, mins, maxs, 0, 1]), Err(Trap::Syscall));
    let mins = f.vector(640, Vec3([-4.0, -2.0, -2.0]));
    let maxs = f.vector(656, Vec3([-4.0, 2.0, 2.0]));
    assert_eq!(f.call(32, &[0, mins, maxs, 1024, 3]), Ok(1));
    assert_eq!(f.vm.memory.read_word(1024).unwrap(), 0);
}

#[test]
fn registered_player_values_use_native_identity_and_preserve_pose_roles() {
    use qa_core::primitives::{ClientId, NumericValue, PlayerTail};
    use qa_session::clients::Connection;
    let mut f = Fixture::with_clients(2);
    assert_eq!(
        f.runtime
            .server
            .connect(Connection::Remote, ModuleId(2), PlayerTail::None, None),
        Some(ClientId(0))
    );
    let native = NativeEntity {
        module: ModuleId(1),
        slot: 0,
    };
    let id = f
        .runtime
        .server
        .connect(
            Connection::Local,
            native.module,
            PlayerTail::Q3 { weapon_time: 777 },
            Some(native),
        )
        .unwrap();
    assert_eq!(id, ClientId(1));
    let player = &mut f.runtime.server.clients[id.0 as usize].player;
    player.movement_rules = RuleSetId::QuakeWorld;
    player.trace_rules = RuleSetId::Quake2;
    player.body.position = Vec3([123.25, -99.5, 40.0]);
    player.body.velocity = Vec3([-22.5, 66.25, 3.0]);
    player.view_angles = Vec3([10.5, 270.25, -8.0]);
    player.view_offset = Vec3([3.0, 4.0, 22.0]);
    player.movement.command_time_ms = 333;
    player.movement.delta_angles = Vec3([25.0, 5.0, -15.0]);
    player.movement.remaining_ms = 99;
    let before = (
        [
            player.body.position,
            player.body.velocity,
            player.body.mins,
            player.body.maxs,
        ],
        player.view_angles,
        player.view_offset,
        player.movement.command_time_ms,
        player.movement.delta_angles,
        player.movement.remaining_ms,
        player.movement.tuning,
    );
    f.row(ENTITIES, 0, 1);
    // Native pose/timing and private client padding must never enter stats-only import.
    f.vm.memory.read_mut(PLAYERS, 512).unwrap().fill(0x7f);
    let stats = std::array::from_fn::<_, 16, _>(|i| match i {
        0 => 71,
        3 => 29,
        14 => i32::MIN,
        15 => i32::MAX,
        _ => -123 - i as i32,
    });
    let persistent =
        std::array::from_fn::<_, 16, _>(|i| if i == 0 { -5 } else { 90_000 + i as i32 });
    for (offset, values) in [(184, stats), (248, persistent)] {
        for (i, value) in values.into_iter().enumerate() {
            f.vm.memory
                .write_word(PLAYERS + offset + (i * 4) as u64, value)
                .unwrap();
        }
    }
    f.publish(); // LocateGameData has not happened: values remain untouched.
    assert_eq!(f.runtime.server.clients[1].player.health, 0);
    f.register(ENTITIES, 1).unwrap();
    f.publish();
    let entity = f.runtime.server.clients[1].entity;
    assert_eq!(f.binding(0), entity);
    let row = &f.runtime.server.clients[1];
    let player = &row.player;
    assert_eq!((player.health, player.armor, player.score), (71, 29, -5));
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    assert!(!row.module_movement);
    assert_eq!(player.tail, PlayerTail::Q3 { weapon_time: 777 });
    assert_eq!(
        (
            [
                player.body.position,
                player.body.velocity,
                player.body.mins,
                player.body.maxs
            ],
            player.view_angles,
            player.view_offset,
            player.movement.command_time_ms,
            player.movement.delta_angles,
            player.movement.remaining_ms,
            player.movement.tuning
        ),
        before
    );
    let values = f.runtime.catalog.hud.values.native(RuleSetId::Quake3);
    for (bindings, expected) in [(&values.stats, stats), (&values.persistent, persistent)] {
        for (binding, value) in bindings.iter().zip(expected) {
            assert_eq!(
                player.values.get(binding.id),
                Some(NumericValue::integer(value))
            );
        }
    }
    assert_eq!(f.runtime.server.clients[0].player.health, 0);

    // Re-register a different base and stride; no mirrored pointer may survive.
    let replacement = PLAYERS + 8192;
    f.vm.memory.read_mut(replacement, 640).unwrap().fill(0);
    f.vm.memory.write_word(replacement + 184, 81).unwrap();
    f.vm.memory.write_word(replacement + 248, -9).unwrap();
    f.call(
        15,
        &[
            0,
            ENTITIES as i32,
            1,
            STRIDE as i32,
            replacement as i32,
            640,
        ],
    )
    .unwrap();
    f.publish();
    assert_eq!(
        (
            f.runtime.server.clients[1].player.health,
            f.runtime.server.clients[1].player.score
        ),
        (81, -9)
    );
    // Client.module and NativeEntity.module are separate checks, not interchangeable.
    f.runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
        module: ModuleId(2),
        slot: 0,
    });
    f.vm.memory.write_word(replacement + 184, 82).unwrap();
    f.publish();
    assert_eq!(f.runtime.server.clients[1].player.health, 81);
    f.runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(native);
    f.runtime.server.clients[1].module = ModuleId(2);
    f.publish();
    assert_eq!(f.runtime.server.clients[1].player.health, 81);
    f.runtime.server.clients[1].module = ModuleId(1);
    f.runtime.server.clients[1].connection = None;
    f.publish();
    assert_eq!(f.runtime.server.clients[1].player.health, 81);
    f.runtime.server.clients[1].connection = Some(Connection::Local);
    f.runtime.server.disconnect(id);
    f.publish();
    assert!(f.runtime.server.entities.resolve(entity).is_none());
    assert_eq!(f.runtime.server.clients[1].player.health, 0);
}

#[test]
fn registered_player_binding_requires_both_native_array_sizes() {
    let mut f = Fixture::new();
    let values = f.runtime.catalog.hud.values.native(RuleSetId::Quake3);
    for (stats, persistent) in [
        (&values.stats[..15], &values.persistent[..]),
        (&values.stats[..], &values.persistent[..15]),
    ] {
        assert_eq!(
            f.vm.bind_player_values(stats, persistent),
            Err(qa_compat::services::CallError::Capacity)
        );
    }
}

#[test]
fn registered_player_values_accept_wrapped_nonnull_qvm_zero_bases() {
    use qa_core::primitives::PlayerTail;
    use qa_session::clients::Connection;
    for (entity_base, player_base) in [(0, PLAYERS), (ENTITIES, 0), (0, 0)] {
        let mut f = Fixture::with_clients(2);
        f.runtime
            .server
            .connect(Connection::Remote, ModuleId(2), PlayerTail::None, None)
            .unwrap();
        let id = f
            .runtime
            .server
            .connect(
                Connection::Local,
                ModuleId(1),
                PlayerTail::None,
                Some(NativeEntity {
                    module: ModuleId(1),
                    slot: 0,
                }),
            )
            .unwrap();
        assert_eq!(id.0, 1);
        f.row(entity_base, 0, 1);
        f.vm.memory.write_word(player_base + 184, 777).unwrap();
        f.vm.memory.write_word(player_base + 248, -901).unwrap();
        f.call(
            15,
            &[
                0,
                if entity_base == 0 {
                    65536
                } else {
                    entity_base as i32
                },
                1,
                STRIDE as i32,
                if player_base == 0 {
                    65536
                } else {
                    player_base as i32
                },
                512,
            ],
        )
        .unwrap();
        f.publish();
        let player = &f.runtime.server.clients[id.0 as usize].player;
        assert_eq!((player.health, player.score), (777, -901));
        assert_eq!(f.binding(0), f.runtime.server.clients[id.0 as usize].entity);
    }
}

#[test]
fn qvm_native_snapshot_exclusion_is_published_without_narrowing_raw_flags() {
    let mut f = Fixture::new();
    f.row(ENTITIES, 0, 1);
    f.register(ENTITIES, 1).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    let entity = f.binding(0);
    for flags in [0x0100_0001, 0x0100_0000] {
        f.vm.memory.write_word(ENTITIES + 424, flags).unwrap();
        f.publish();
        let columns = &f.runtime.server.entities.columns;
        assert_eq!(columns.flags[entity.slot as usize], flags as u32);
        assert_eq!(
            columns.appearance[entity.slot as usize].snapshot_excluded,
            flags & 1 != 0,
        );
        assert_eq!(f.vm.memory.read_word(ENTITIES + 424).unwrap(), flags);
    }
}

#[test]
fn qvm_registered_entity_zero_links_with_word_flags_and_reuses_owned_lifetimes() {
    let mut f = Fixture::new();
    f.row(ENTITIES, 0, 1);
    f.row(ENTITIES + STRIDE, 1, 1);
    f.register(ENTITIES, 2).unwrap();
    f.vm.memory.write_word(ENTITIES, -1).unwrap();
    assert_eq!(f.call(30, &[0, ENTITIES as i32]), Err(Trap::Syscall));
    f.vm.memory.write_word(ENTITIES, 1024).unwrap();
    assert_eq!(f.call(30, &[0, ENTITIES as i32]), Err(Trap::Syscall));
    f.vm.memory.write_word(ENTITIES, 0).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    f.call(30, &[0, (ENTITIES + STRIDE) as i32]).unwrap();
    let zero = f.binding(0);
    let original = f.binding(1);
    assert_ne!(zero, original);
    assert_eq!(f.vm.memory.read_word(ENTITIES + 416).unwrap(), 1);
    assert_eq!(f.vm.memory.read_word(ENTITIES + 420).unwrap(), 1);
    assert_eq!(
        f.runtime.server.entities.columns.collision_owner[zero.slot as usize],
        CollisionOwner::Native(NativeEntity {
            module: ModuleId(1),
            slot: 1022
        })
    );
    assert_eq!(f.candidates(LinkFlags::SOLID), vec![original, zero]);
    f.publish();
    assert_eq!(
        f.runtime.server.entities.columns.appearance[zero.slot as usize].source,
        RuleSetId::Quake3
    );
    assert_eq!(
        f.runtime.server.entities.columns.appearance[zero.slot as usize].frame,
        11
    );
    assert_eq!(
        f.runtime.server.entities.columns.point_contents_pose[zero.slot as usize],
        Some(qa_core::primitives::EntityPose {
            position: Vec3([0.0, 0.0, 0.0]),
            angles: Vec3([0.0, 45.0, 0.0])
        })
    );
    let bounds = f.runtime.server.area.bounds(zero).unwrap();
    let body = f.runtime.server.entities.columns.body(zero.slot as usize);
    let pose = f.runtime.server.entities.columns.point_contents_pose[zero.slot as usize];
    f.vm.memory.write_word(ENTITIES + 424, 0x400).unwrap();
    f.vm.memory
        .write_vec3(ENTITIES + 488, Vec3([8.0, 0.0, 0.0]))
        .unwrap();
    assert_eq!(f.call(30, &[0, ENTITIES as i32]), Err(Trap::Syscall));
    let rejected_body = f.runtime.server.entities.columns.body(zero.slot as usize);
    assert_eq!(rejected_body.position, body.position);
    assert_eq!(rejected_body.velocity, body.velocity);
    assert_eq!(rejected_body.mins, body.mins);
    assert_eq!(rejected_body.maxs, body.maxs);
    assert_eq!(
        f.runtime.server.entities.columns.point_contents_pose[zero.slot as usize],
        pose
    );
    assert_eq!(f.runtime.server.area.bounds(zero), Some(bounds));
    assert_eq!(f.vm.memory.read_word(ENTITIES + 420).unwrap(), 1);
    f.vm.memory.write_word(ENTITIES + 424, 0).unwrap();
    f.vm.memory
        .write_vec3(ENTITIES + 488, Vec3([-3.0, 0.0, 0.0]))
        .unwrap();
    f.publish();
    assert_eq!(
        f.runtime.server.entities.columns.position[zero.slot as usize],
        Vec3([-3.0, 0.0, 0.0])
    );
    assert_eq!(f.runtime.server.area.bounds(zero), Some(bounds));
    f.vm.memory.write_word(ENTITIES + 416, 257).unwrap();
    f.call(31, &[0, ENTITIES as i32]).unwrap();
    assert_eq!(f.vm.memory.read_word(ENTITIES + 416).unwrap(), 0);
    assert_eq!(f.vm.memory.read_word(ENTITIES + 420).unwrap(), 1);
    assert!(f.runtime.server.entities.resolve(zero).is_some());
    assert_eq!(f.candidates(LinkFlags::SOLID), vec![original]);
    f.publish();
    assert_eq!(
        f.runtime.server.entities.columns.appearance[zero.slot as usize].models,
        [None; 4]
    );
    f.row(ENTITIES, 0, 1);
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    let replacement = f.binding(0);
    assert_ne!(replacement, zero);
    assert!(f.runtime.server.entities.resolve(zero).is_none());
    assert_eq!(f.candidates(LinkFlags::SOLID), vec![replacement, original]);
}

#[test]
fn qvm_locate_rejections_preserve_registered_rows_and_masked_zero_is_not_null() {
    let mut f = Fixture::new();
    f.row(0, 0, 1);
    f.register(65536, 1).unwrap();
    for arguments in [
        [0, 65536, -1, STRIDE as i32, PLAYERS as i32, 512],
        [0, 65536, 1025, STRIDE as i32, PLAYERS as i32, 512],
        [0, 65536, 1, 515, PLAYERS as i32, 512],
        [0, 65536, 1, STRIDE as i32, PLAYERS as i32, 467],
        [0, 65536, 1, STRIDE as i32, 65535, 512],
        [0, 0, 1, STRIDE as i32, PLAYERS as i32, 512],
    ] {
        assert_eq!(f.call(15, &arguments), Err(Trap::Syscall));
    }
    f.call(30, &[0, 65536]).unwrap();
    let entity = f.binding(0);
    assert_eq!(f.call(31, &[0, 0]), Err(Trap::Syscall));
    assert_eq!(f.call(30, &[0, 0]), Err(Trap::Syscall));
    assert_eq!(f.candidates(LinkFlags::SOLID), vec![entity]);
    f.call(31, &[0, 65536]).unwrap();
    assert!(f.candidates(LinkFlags::SOLID).is_empty());
    assert_eq!(f.vm.memory.read_word(416).unwrap(), 0);
}

#[test]
fn qvm_count_growth_borrowed_client_and_trigger_roles_use_the_shared_projection() {
    let mut f = Fixture::new();
    let client = f
        .runtime
        .server
        .entities
        .allocate(context().clock, ModuleId(1), AllocationPolicy::q3(0))
        .unwrap()
        .id;
    f.runtime.server.entities.columns.native_entity[client.slot as usize] = Some(NativeEntity {
        module: ModuleId(1),
        slot: 0,
    });
    f.row(ENTITIES, 0, 1);
    f.register(ENTITIES, 1).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    assert_eq!(f.binding(0), client);
    f.row(ENTITIES, 0, 1);
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    assert_eq!(f.binding(0), client);
    assert!(f.runtime.server.entities.resolve(client).is_some());
    f.row(ENTITIES + STRIDE, 1, 0x4000_0000);
    assert_eq!(
        f.call(30, &[0, (ENTITIES + STRIDE) as i32]),
        Err(Trap::Syscall)
    );
    let live = f.runtime.server.entities.active().count();
    f.register(ENTITIES, 2).unwrap();
    f.publish();
    assert_eq!(f.runtime.server.entities.active().count(), live);
    f.vm.memory
        .write_word(ENTITIES + STRIDE + 424, 0x400)
        .unwrap();
    assert_eq!(
        f.call(30, &[0, (ENTITIES + STRIDE) as i32]),
        Err(Trap::Syscall)
    );
    assert_eq!(f.runtime.server.entities.active().count(), live);
    assert_eq!(f.candidates(LinkFlags::LINKED), vec![client]);
    f.vm.memory.write_word(ENTITIES + STRIDE + 424, 0).unwrap();
    f.call(30, &[0, (ENTITIES + STRIDE) as i32]).unwrap();
    let trigger = f.binding(1);
    assert_eq!(f.candidates(LinkFlags::SOLID), vec![client]);
    assert_eq!(f.candidates(LinkFlags::TRIGGER), vec![trigger]);
    assert_eq!(f.candidates(LinkFlags::LINKED), vec![trigger, client]);
    f.publish();
    assert!(f.runtime.server.entities.columns.appearance[client.slot as usize].models[0].is_some());
    f.register(ENTITIES, 0).unwrap();
    f.publish();
    assert!(f.runtime.server.entities.resolve(client).is_some());
    assert_eq!(
        f.runtime.server.entities.columns.appearance[client.slot as usize].models,
        [None; 4]
    );
    assert!(f.runtime.server.entities.resolve(trigger).is_none());
}

#[test]
fn qvm_point_contents_reads_published_pose_between_imports_without_relink() {
    let mut f = Fixture::new();
    f.empty_collision();
    f.trace_rules = RuleSetId::Quake3;
    f.row(ENTITIES, 0, 1);
    f.register(ENTITIES, 1).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    let entity = f.binding(0);
    let old_bounds = f.runtime.server.area.bounds(entity).unwrap();
    let point = PLAYERS + 512;
    f.vm.memory
        .write_vec3(point, Vec3([-0.5, 0.0, 0.0]))
        .unwrap();
    assert_eq!(f.call(25, &[0, point as i32, 1023]).unwrap(), 0x0200_0000);
    f.vm.memory
        .write_vec3(ENTITIES + 92, Vec3([-2.0, 0.0, 0.0]))
        .unwrap();
    assert_eq!(f.call(25, &[0, point as i32, 1023]).unwrap(), 0);
    assert_eq!(f.runtime.server.area.bounds(entity), Some(old_bounds));
    assert_eq!(f.vm.memory.read_word(ENTITIES + 420).unwrap(), 1);
    assert_eq!(
        f.runtime.server.entities.columns.position[entity.slot as usize],
        Vec3([-2.0, 0.0, 0.0])
    );
}

#[test]
fn qvm_live_contents_reads_zero_contents_boxes_and_rejects_only_executed_targets() {
    let mut f = Fixture::new();
    // The inline-model case below needs a real model-zero resource, while
    // the other registration fixtures only require visibility metadata.
    f.empty_collision();
    f.trace_rules = RuleSetId::Quake3;
    let point = PLAYERS + 512;
    f.vm.memory
        .write_vec3(point, Vec3([-0.5, 0.0, 0.0]))
        .unwrap();
    // A world-only query does not require LocateGameData.
    assert_eq!(f.call(25, &[0, point as i32, 1023]).unwrap(), 0);
    f.row(ENTITIES, 0, 1);
    f.register(ENTITIES, 1).unwrap();
    f.call(30, &[0, ENTITIES as i32]).unwrap();
    let entity = f.binding(0);
    let bounds = f.runtime.server.area.bounds(entity);
    // The linked temporary box remains a BODY for PointContents even when
    // the native trace mask now sees r.contents==0.
    f.vm.memory.write_word(ENTITIES + 460, 0).unwrap();
    assert_eq!(f.call(25, &[0, point as i32, 1023]).unwrap(), 0x0200_0000);
    // Owner and physical pose are unused by this native contents operation.
    f.vm.memory.write_word(ENTITIES + 512, -1).unwrap();
    f.vm.memory
        .write_vec3(ENTITIES + 488, Vec3([f32::NAN; 3]))
        .unwrap();
    assert_eq!(f.call(25, &[0, point as i32, 1023]).unwrap(), 0x0200_0000);
    f.vm.memory.write_word(ENTITIES + 424, 0x400).unwrap();
    // Native pass filtering precedes the unsupported target decode.
    assert_eq!(f.call(25, &[0, point as i32, 0]).unwrap(), 0);
    assert_eq!(f.call(25, &[0, point as i32, 1023]), Err(Trap::Syscall));
    assert_eq!(f.runtime.server.area.bounds(entity), bounds);
    assert_eq!(f.vm.memory.read_word(ENTITIES + 420).unwrap(), 1);
    f.vm.memory.write_word(ENTITIES + 424, 0).unwrap();
    // bmodel is a native qboolean, not Q2's SOLID_BSP discriminant.
    f.vm.memory.write_word(ENTITIES + 432, 257).unwrap();
    f.vm.memory.write_word(ENTITIES + 160, 999).unwrap();
    assert_eq!(f.call(25, &[0, point as i32, 0]).unwrap(), 0);
    assert_eq!(f.call(25, &[0, point as i32, 1023]), Err(Trap::Syscall));
    assert_eq!(f.runtime.server.area.bounds(entity), bounds);
    // SVF_CAPSULE selects only temporary hulls; BSP model handles ignore it.
    f.vm.memory.write_word(ENTITIES + 160, 0).unwrap();
    f.vm.memory.write_word(ENTITIES + 424, 0x400).unwrap();
    assert_eq!(f.call(25, &[0, point as i32, 1023]).unwrap(), 0);
}
