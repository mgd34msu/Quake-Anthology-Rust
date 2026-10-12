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
    f.runtime
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
