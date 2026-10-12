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
}
fn context() -> CallContext {
    CallContext {
        module: ModuleId(1),
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake,
        sound_rules: RuleSetId::Quake2,
        server_frame: 0,
        clock: ThinkTime::Milliseconds(0),
        console: Context::default(),
        allocation: AllocationPolicy::q3(0),
        link_order: LinkOrder::Head,
    }
}
impl Fixture {
    fn new() -> Self {
        let mut runtime = Runtime::load(1, []).unwrap();
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
        Self {
            runtime,
            console,
            storage,
            vm,
            unknown: UnknownCalls::load(1).unwrap(),
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
            context: context(),
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
