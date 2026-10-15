use qa_compat::{
    abi::{Q3_SERVER, QvmCalls, UnknownCalls},
    memory::ModuleMemory,
    qvm::{SystemCalls, Vm},
    services::{CallContext, CallError, EngineServices, ServiceStorage},
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    names::NameTable,
    primitives::{
        ClientId, ClockObservation, ModuleId, MovementMode, NativeEntity, NativePlayerTail,
        NumericValue, PlayerDamageSample, PlayerSignal, PlayerSignals, PlayerValueBindings,
        Q3StatLayout, ResourceNamespace, RuleSetId, SoundRef, ThinkTime, ValueBinding, ValueId,
        ValueReset, ValueWidth, Vec3, WeaponReadiness,
    },
    sys_events::EventTime,
};
use qa_formats::program::qvm::{Image, Opcode};
use qa_session::clients::{Connection, Server};
use qa_world::{area::LinkOrder, collision::CollisionStore, entities::AllocationPolicy};

#[path = "support/shaders.rs"]
mod shaders;
use shaders::ShaderLoader;

const ENTITIES: u64 = 4096;
const PLAYERS: u64 = 16384;
const ENTITY_STRIDE: u64 = 608;
const PLAYER_STRIDE: u64 = 640;
const MODULE: ModuleId = ModuleId(1);
const NATIVE: NativeEntity = NativeEntity {
    module: MODULE,
    slot: 0,
};
const SIGNAL_SAMPLE: PlayerSignals = PlayerSignals {
    sequence: 0x8001_0001,
    events: [
        PlayerSignal {
            code: 0x8002_0301,
            parameter: 0x8101_0005,
        },
        PlayerSignal {
            code: 0xffff_ffff,
            parameter: 0x8202_0006,
        },
    ],
    external: PlayerSignal {
        code: 0x8303_0307,
        parameter: 0x8404_0008,
    },
    damage: PlayerDamageSample {
        sequence: 0x8505_0009,
        yaw: 0x8606_000a,
        pitch: 0x8707_000b,
        count: 0x8808_000c,
    },
};
const SIGNAL_CELLS: [(u64, u32); 11] = [
    (108, 0x8001_0001),
    (112, 0x8002_0301),
    (116, 0xffff_ffff),
    (120, 0x8101_0005),
    (124, 0x8202_0006),
    (128, 0x8303_0307),
    (132, 0x8404_0008),
    (168, 0x8505_0009),
    (172, 0x8606_000a),
    (176, 0x8707_000b),
    (180, 0x8808_000c),
];

fn bindings(first: u32) -> [ValueBinding; 16] {
    std::array::from_fn(|index| ValueBinding {
        id: ValueId(first + index as u32),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    })
}
fn player_bindings() -> [[ValueBinding; 16]; 4] {
    std::array::from_fn(|index| bindings(index as u32 * 16))
}
fn value_bindings(arrays: &[[ValueBinding; 16]; 4]) -> PlayerValueBindings<'_> {
    PlayerValueBindings {
        arrays: std::array::from_fn(|index| arrays[index].as_slice()),
        items: None,
    }
}
fn context() -> CallContext {
    CallContext {
        module: MODULE,
        clock: ThinkTime::Milliseconds(900),
        server_frame: 12,
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake2,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::QuakeWorld),
        console: Context {
            source: RuleSetId::Quake,
            ..Context::default()
        },
        allocation: AllocationPolicy::q3(0),
        link_order: LinkOrder::Head,
    }
}
struct Fixture {
    server: Server,
    input: qa_input::Input,
    cvars: Cvars,
    commands: CommandBuffer,
    vfs: qa_content::vfs::Vfs,
    localization: qa_content::localization::Catalog,
    registry: qa_gameplay::registry::Registry,
    names: NameTable,
    storage: ServiceStorage,
    geometry: CollisionStore,
    scratch: qa_world::collision::TraceScratch,
    unknown: UnknownCalls,
}
impl Fixture {
    fn new() -> Self {
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let mut cvars = Cvars::new().unwrap();
        let view = cvars
            .bind(
                "sv_maxclients",
                Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
            )
            .unwrap();
        cvars.force_write(view, "2").unwrap();
        let mut storage =
            ServiceStorage::load_native(&[(MODULE, 1024, RuleSetId::Quake3)], 0, &cvars).unwrap();
        storage.bind_client_capacity(MODULE, 2).unwrap();
        let mut server = Server::load(
            2,
            16,
            registry.items.len() + 1,
            registry.powerup_capacity(),
            registry.weapons.len() + 1,
            64,
        )
        .unwrap();
        assert_eq!(
            server.connect(Connection::Remote, ModuleId(2), None),
            Some(ClientId(0))
        );
        assert_eq!(
            server.connect(Connection::Local, MODULE, Some(NATIVE)),
            Some(ClientId(1))
        );
        let client = &mut server.clients[1];
        assert_ne!(client.entity.slot, NATIVE.slot as u32);
        client.module_movement = true;
        client.player.movement_rules = RuleSetId::QuakeWorld;
        client.player.trace_rules = RuleSetId::Quake2;
        client.player.movement.tuning.gravity_multiplier = 2.0;
        client.player.movement.tuning.speed_multiplier = 0.5;
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        Self {
            server,
            input: qa_input::Input::load(),
            cvars,
            commands: CommandBuffer::new(),
            vfs: Default::default(),
            localization: Default::default(),
            registry,
            names,
            storage,
            geometry,
            scratch,
            unknown: UnknownCalls::load(1).unwrap(),
        }
    }
    fn engine(&mut self) -> EngineServices<'_> {
        EngineServices {
            server: &mut self.server,
            callbacks: None,
            client_disconnect: None,
            input: &mut self.input,
            editor: None,
            cvars: &mut self.cvars,
            commands: &mut self.commands,
            client_connection: None,
            client_collision: None,
            renderer: None,
            scene: None,
            client_audio: None,
            vfs: &self.vfs,
            model_loader: None,
            model_lookup: None,
            sound_loader: None,
            sound_bank: None,
            localization: &self.localization,
            registry: &self.registry,
            names: &mut self.names,
            storage: &mut self.storage,
            geometry: &self.geometry,
            world: None,
            visibility: None,
            portals: None,
            scratch: &mut self.scratch,
        }
    }
    fn call(
        &mut self,
        vm: &mut Vm,
        number: u32,
        arguments: &[i32],
    ) -> Result<i32, qa_compat::qvm::Trap> {
        let mut unknown = std::mem::replace(&mut self.unknown, UnknownCalls::load(1).unwrap());
        let result = {
            let mut engine = self.engine();
            let mut calls = QvmCalls {
                services: &mut engine,
                table: &Q3_SERVER,
                context: context(),
                platform_time: EventTime(0),
                command: &[],
                unknown: &mut unknown,
            };
            calls.begin_call(vm)?;
            calls.call(vm, number, arguments)
        };
        self.unknown = unknown;
        result
    }
    fn register(&mut self, vm: &mut Vm, entities: u64, players: u64, stride: u64) {
        self.call(
            vm,
            15,
            &[
                0,
                entities as i32,
                1,
                ENTITY_STRIDE as i32,
                players as i32,
                stride as i32,
            ],
        )
        .unwrap();
        vm.bind_client(&self.server.entities, NATIVE, self.server.clients[1].entity)
            .unwrap();
    }
    fn publish(&mut self, vm: &mut Vm) -> Result<(), CallError> {
        self.publish_at(vm, EventTime::from_milliseconds(1234))
    }
    fn publish_at(&mut self, vm: &mut Vm, time: EventTime) -> Result<(), CallError> {
        self.publish_with_clock(vm, time, context().clock)
    }
    fn publish_with_clock(
        &mut self,
        vm: &mut Vm,
        time: EventTime,
        clock: ThinkTime,
    ) -> Result<(), CallError> {
        let mut context = context();
        context.clock = clock;
        vm.publish_entities(&mut self.engine(), context, time, Some(8))
    }
}
fn qvm(instructions: &[(Opcode, i32)], registry: &qa_gameplay::registry::Registry) -> Vm {
    let mut code = Vec::new();
    for &(opcode, argument) in instructions {
        code.push(opcode as u8);
        match opcode.operand_bytes() {
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
    let mut vm = Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    vm.configure_q3_game(Q3StatLayout::BASE);
    vm.bind_player_values(value_bindings(&player_bindings()), registry)
        .unwrap();
    vm
}
fn entity(memory: &mut ModuleMemory<'_>, address: u64) {
    memory
        .read_mut(address, ENTITY_STRIDE as usize)
        .unwrap()
        .fill(0);
    memory.write_word(address + 512, 1023).unwrap();
    memory
        .write_vec3(address + 436, Vec3([-15.0, -15.0, -24.0]))
        .unwrap();
    memory
        .write_vec3(address + 448, Vec3([15.0, 15.0, 32.0]))
        .unwrap();
    memory
        .write_vec3(address + 488, Vec3([-900.0, 2.0, 3.0]))
        .unwrap();
}

// Original q_shared.h declaration order, independent of the wire/ABI mapper.
fn public_entity_cells() -> [u32; 52] {
    [
        0,
        2,
        0x8101_0001,
        3,
        i32::MIN as u32 + 41,
        1234567,
        101.25f32.to_bits(),
        102.5f32.to_bits(),
        103.75f32.to_bits(),
        (-11.25f32).to_bits(),
        12.5f32.to_bits(),
        13.75f32.to_bits(),
        2,
        i32::MAX as u32 - 51,
        7654321,
        81.25f32.to_bits(),
        82.5f32.to_bits(),
        83.75f32.to_bits(),
        21.25f32.to_bits(),
        (-22.5f32).to_bits(),
        23.75f32.to_bits(),
        i32::MIN as u32 + 17,
        i32::MAX as u32 - 17,
        201.25f32.to_bits(),
        202.5f32.to_bits(),
        203.75f32.to_bits(),
        301.25f32.to_bits(),
        302.5f32.to_bits(),
        303.75f32.to_bits(),
        401.25f32.to_bits(),
        402.5f32.to_bits(),
        403.75f32.to_bits(),
        501.25f32.to_bits(),
        502.5f32.to_bits(),
        503.75f32.to_bits(),
        0,
        1023,
        1022,
        0x8123_4567,
        0x8001_0009,
        7,
        0x8100_0007,
        0x8100_0008,
        0x8100_0009,
        0x8100_000a,
        0x8100_0300,
        0x8100_000b,
        0x8100_000c,
        0x8100_000d,
        0x8100_000e,
        0x8100_000f,
        0x8100_0010,
    ]
}
fn verify_entity_fields(f: &Fixture) {
    use qa_core::primitives::{EntityModelData, EntityPose, EntityRecord, NativeEntityRef};
    let entity = f.server.clients[1].entity;
    let slot = entity.slot as usize;
    let columns = &f.server.entities.columns;
    assert_eq!(columns.position[slot], Vec3([-900., 2., 3.]));
    assert_eq!(columns.angles[slot], Vec3::default());
    assert_eq!(columns.velocity[slot], Vec3::default());
    let appearance = columns.appearance[slot];
    let published = appearance.presentation.unwrap();
    assert_eq!(published.namespace, ResourceNamespace::Module(MODULE));
    assert_eq!(
        published.pose,
        EntityPose {
            position: Vec3([201.25, 202.5, 203.75]),
            angles: Vec3([401.25, 402.5, 403.75]),
        }
    );
    assert_eq!(columns.point_contents_pose[slot], Some(published.pose));
    assert_eq!(
        published.trajectories.position.base,
        Vec3([101.25, 102.5, 103.75])
    );
    assert_eq!(
        published.secondary_pose.position,
        Vec3([301.25, 302.5, 303.75])
    );
    assert_eq!(
        published.model_data,
        EntityModelData::Item {
            index: 7,
            secondary: 0x8100_0007u32 as i32
        }
    );
    assert_eq!(appearance.models, [None; 4]); // Item7 is not CS_MODELS7.
    assert_eq!(
        appearance.native.other_entity,
        Some(NativeEntityRef::entity(
            ResourceNamespace::Module(MODULE),
            0
        ))
    );
    let record = EntityRecord {
        position: columns.position[slot],
        velocity: columns.velocity[slot],
        angles: columns.angles[slot],
        appearance,
    };
    let mut words = [0; qa_network::states::ENTITY_WORDS];
    assert!(
        qa_network::projection::EntityProjection::load_native_q3().reduce(
            &record,
            &qa_network::projection::EntityContext::default(),
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal()),
        )
    );
    let mut bytes = [0xa5; qa_compat::abi::Q3_ENTITY_STATE_BYTES + 8];
    let mut memory = ModuleMemory::borrow(0x1234_0000, &mut bytes).unwrap();
    qa_compat::abi::write_q3_entity_state(&mut memory, 0x1234_0004, 0, &words).unwrap();
    for (index, expected) in public_entity_cells().into_iter().enumerate() {
        assert_eq!(
            memory.read_word(0x1234_0004 + index as u64 * 4).unwrap() as u32,
            expected,
            "public native entity cell{index}"
        );
    }
    assert_eq!(memory.read_word(0x1234_0000).unwrap() as u32, 0xa5a5a5a5);
    assert_eq!(memory.read_word(0x1234_00d4).unwrap() as u32, 0xa5a5a5a5);
}

fn qvm_entity_publication_preserves_all_fields_and_separate_physical_pose() {
    let mut f = Fixture::new();
    let mut operations = vec![(Opcode::Enter, 16)];
    for (index, value) in public_entity_cells().into_iter().enumerate() {
        operations.extend([
            (Opcode::Const, ENTITIES as i32 + index as i32 * 4),
            (Opcode::Const, value as i32),
            (Opcode::Store4, 0),
        ]);
    }
    operations.extend([
        (Opcode::Const, ENTITIES as i32 + 416),
        (Opcode::Const, 1),
        (Opcode::Store4, 0),
        (Opcode::Const, 0),
        (Opcode::Leave, 16),
    ]);
    let mut vm = qvm(&operations, &f.registry);
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    let common = f.server.clients[1].entity;
    let old_area = f.server.area.bounds(common);
    let old_owner = f.server.entities.columns.native_entity[common.slot as usize];
    {
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut engine = f.engine();
        let mut calls = QvmCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(0),
            command: &[],
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [8; 10], 1000, false), Ok(0));
    }
    f.publish(&mut vm).unwrap();
    verify_entity_fields(&f);
    assert_eq!(f.server.area.bounds(common), old_area);
    assert_eq!(
        f.server.entities.columns.native_entity[common.slot as usize],
        old_owner
    );
    assert_eq!(
        f.server.clients[1].player.movement_rules,
        RuleSetId::QuakeWorld
    );
    assert_eq!(f.server.clients[1].player.trace_rules, RuleSetId::Quake2);
}
fn player(memory: &mut ModuleMemory<'_>, address: u64) {
    memory
        .read_mut(address, PLAYER_STRIDE as usize)
        .unwrap()
        .fill(0xa5);
    for (offset, value) in [
        (0, -1234567),
        (4, 2),
        (12, 1 | 2 | 32 | 64 | 256),
        (16, 70000),
        (44, -197000071),
        (48, 1200),
        (52, 250),
        (56, -65537),
        (60, 131073),
        (64, i32::MAX - 1),
        (68, 1022),
        (108, 0),
        (112, 0),
        (116, 0),
        (120, 0),
        (124, 0),
        (128, 0),
        (132, 0),
        (140, 0),
        (144, 4),
        (164, -71),
        (168, 0),
        (172, 0),
        (176, 0),
        (180, 0),
        (444, 0),
    ] {
        memory.write_word(address + offset, value).unwrap();
    }
    memory
        .write_vec3(address + 20, Vec3([-0.0, f32::from_bits(1), 17.75]))
        .unwrap();
    memory
        .write_vec3(address + 32, Vec3([19.125, -0.0, -3.5]))
        .unwrap();
    memory
        .write_vec3(address + 152, Vec3([45.25, -0.0, 721.5]))
        .unwrap();
    for (offset, first) in [(184, 0), (248, 16), (376, 32), (312, 48)] {
        for index in 0..16 {
            memory
                .write_word(
                    address + offset + index * 4,
                    match first + index {
                        0 => 79,
                        2 => (1 << 2) | (1 << 4) | (1 << 15),
                        3 => 27,
                        14 => i32::MIN,
                        15 => i32::MAX,
                        16 => -9,
                        34 => -1,
                        35 => 16_777_217,
                        46 => i32::MIN,
                        47 => i32::MAX,
                        49 => i32::MAX - 1,
                        _ => 90000 + index as i32,
                    },
                )
                .unwrap();
        }
    }
}
fn verify(f: &Fixture) {
    let row = &f.server.clients[1];
    let p = &row.player;
    assert_eq!(
        p.body.position.0.map(f32::to_bits),
        [(-0.0f32).to_bits(), 1, 17.75f32.to_bits()]
    );
    assert_eq!(
        p.body.velocity.0.map(f32::to_bits),
        [
            19.125f32.to_bits(),
            (-0.0f32).to_bits(),
            (-3.5f32).to_bits()
        ]
    );
    assert_eq!(
        p.view_angles.0.map(f32::to_bits),
        [45.25f32.to_bits(), (-0.0f32).to_bits(), 721.5f32.to_bits()]
    );
    assert_eq!(p.body.mins, Vec3([-15.0, -15.0, -24.0]));
    assert_eq!(p.body.maxs, Vec3([15.0, 15.0, 32.0]));
    assert_eq!(p.view_offset.0[2], -71.0);
    assert_eq!(p.movement.mode, MovementMode::Spectator);
    assert_eq!(
        (p.movement_rules, p.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    assert_eq!(p.movement.command_time_ms, -1234567);
    assert_eq!(p.movement.remaining_ms, 70000);
    assert_eq!(
        p.movement.delta_angles.degrees().0.map(f32::to_bits),
        [-65537, 131073, i32::MAX - 1].map(|v| qa_core::math::short_to_angle(v).to_bits())
    );
    assert_eq!(
        p.movement.delta_angles.0[2].units(qa_core::math::AngleShortForm::Factored),
        i32::MAX - 1
    );
    for (projection, word, expected) in [
        (
            qa_network::projection::PlayerProjection::load_native_q3(
                value_bindings(&player_bindings()),
                Q3StatLayout::BASE,
            ),
            36,
            (i32::MAX - 1) as u32,
        ),
        (
            qa_network::projection::PlayerProjection::load(
                qa_network::commands::packet::Protocol::Quake3_68,
                &[],
            ),
            36,
            65534,
        ),
        (
            qa_network::projection::PlayerProjection::load(
                qa_network::commands::packet::Protocol::Quake2_34,
                &[],
            ),
            12,
            (-2i32) as u32,
        ),
        (
            qa_network::projection::PlayerProjection::load(
                qa_network::commands::packet::Protocol::Quake2Repro1038,
                &[],
            ),
            12,
            qa_core::math::short_to_angle(i32::MAX - 1).to_bits(),
        ),
    ] {
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            p,
            &Default::default(),
            &mut words,
            |_| None,
            |sound| Some(sound.ordinal),
        ));
        assert_eq!(words[word], expected);
    }
    assert!(p.movement.grounded);
    assert!(p.movement.ground.is_none()); // WORLD is not common entity slot1022.
    assert!(p.movement.ducked && p.movement.jump_held);
    assert_eq!(p.movement.tuning.gravity, Some(600.0.into()));
    assert_eq!(p.movement.tuning.max_speed, Some(500.0.into()));
    let tail = p.native.unwrap();
    assert_eq!(tail.source, RuleSetId::Quake3);
    assert_eq!(tail.namespace, ResourceNamespace::Module(MODULE));
    assert_eq!(tail.signals, Some(PlayerSignals::default()));
    assert_eq!(tail.flags, 1 | 2 | 32 | 64 | 256);
    assert_eq!((tail.gravity, tail.speed), (Some(1200), Some(250)));
    assert_eq!((tail.client_number, tail.weapon_number), (Some(0), Some(4)));
    assert_eq!(
        p.inventory.weapon,
        f.registry.native_weapon(RuleSetId::Quake3, 4).unwrap()
    );
    assert_eq!(
        (
            p.movement.tuning.gravity_multiplier,
            p.movement.tuning.speed_multiplier
        ),
        (2.0, 0.5)
    );
    assert_eq!((p.health, p.armor, p.score), (79, 27, -9));
    assert_eq!(
        p.values.get(ValueId(14)),
        Some(NumericValue::integer(i32::MIN))
    );
    assert_eq!(
        p.values.get(ValueId(15)),
        Some(NumericValue::integer(i32::MAX))
    );
    assert_eq!(
        p.weapon_state.unwrap().namespace,
        ResourceNamespace::Module(MODULE)
    );
    assert_eq!(
        p.weapon_state.unwrap().readiness,
        WeaponReadiness::RemainingMilliseconds(-197000071)
    );
    for (id, expected) in [
        (34, -1),
        (35, 16_777_217),
        (46, i32::MIN),
        (47, i32::MAX),
        (49, i32::MAX - 1),
    ] {
        assert_eq!(
            p.values.get(ValueId(id)),
            Some(NumericValue::integer(expected))
        );
    }
    let ammo = |native| {
        f.registry
            .weapon(f.registry.native_weapon(RuleSetId::Quake3, native).unwrap())
            .unwrap()
            .ammo
            .unwrap()
    };
    assert_eq!(p.inventory.item_counts[ammo(2).0 as usize], -1);
    assert_eq!(p.inventory.item_counts[ammo(3).0 as usize], 16_777_217);
    for (native, owned) in [(2, true), (3, false), (4, true)] {
        let item = f
            .registry
            .weapon(f.registry.native_weapon(RuleSetId::Quake3, native).unwrap())
            .unwrap()
            .item;
        assert_eq!(p.inventory.item_counts[item.0 as usize], i32::from(owned));
    }
    assert_eq!(
        f.server.entities.columns.position[row.entity.slot as usize],
        Vec3([-900.0, 2.0, 3.0])
    );
    assert_eq!(f.server.clients[0].player.health, 0);
    assert!(f.server.area.bounds(row.entity).is_none());
}
fn qvm_native_memory_preserves_full_widths_roles_and_unlinked_player_origin() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    f.publish(&mut vm).unwrap();
    verify(&f);
    assert_eq!(vm.memory.read_word(PLAYERS + 64).unwrap(), i32::MAX - 1);
    // An engine-owned client continues publishing native arrays/readiness only.
    f.server.clients[1].module_movement = false;
    let p = &f.server.clients[1].player;
    let before = (
        [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
        p.view_angles,
        p.movement,
        p.native,
        p.inventory.weapon,
    );
    vm.memory
        .write_vec3(PLAYERS + 20, Vec3([222.0; 3]))
        .unwrap();
    vm.memory
        .write_vec3(PLAYERS + 152, Vec3([333.0; 3]))
        .unwrap();
    vm.memory.write_word(PLAYERS + 0, 42).unwrap();
    vm.memory.write_word(PLAYERS + 184, 81).unwrap();
    vm.memory.write_word(PLAYERS + 12, 0).unwrap();
    vm.memory.write_word(PLAYERS + 48, 0).unwrap();
    vm.memory.write_word(PLAYERS + 52, 0).unwrap();
    vm.memory.write_word(PLAYERS + 140, 1).unwrap();
    vm.memory.write_word(PLAYERS + 144, 999).unwrap();
    vm.memory.write_word(PLAYERS + 184 + 2 * 4, 1 << 3).unwrap();
    vm.memory.write_word(PLAYERS + 376 + 3 * 4, 29).unwrap();
    vm.memory
        .write_word(PLAYERS + 312 + 1 * 4, -123456789)
        .unwrap();
    f.publish(&mut vm).unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(
        (
            [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
            p.view_angles,
            p.movement,
            p.native,
            p.inventory.weapon,
        ),
        before
    );
    assert_eq!(p.health, 81);
    let shotgun = f
        .registry
        .weapon(f.registry.native_weapon(RuleSetId::Quake3, 3).unwrap())
        .unwrap();
    assert_eq!(p.inventory.item_counts[shotgun.item.0 as usize], 1);
    assert_eq!(
        p.inventory.item_counts[shotgun.ammo.unwrap().0 as usize],
        29
    );
    assert_eq!(
        p.values.get(ValueId(49)),
        Some(NumericValue::integer(-123456789))
    );
}

fn qvm_metadata_preserves_raw_native_words_and_zero_multiplier_tuning() {
    use qa_core::primitives::MovementMagnitude;
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    let residual = 0x8000_0000 | 512 | 4096 | 8192;
    vm.memory
        .write_word(PLAYERS + 12, (residual | 1 | 32) as i32)
        .unwrap();
    vm.memory.write_word(PLAYERS + 48, 16_777_217).unwrap();
    vm.memory.write_word(PLAYERS + 52, -16_777_219).unwrap();
    vm.memory.write_word(PLAYERS + 140, 1).unwrap();
    vm.memory.write_word(PLAYERS + 144, 65539).unwrap();
    f.publish(&mut vm).unwrap();
    let p = &mut f.server.clients[1].player;
    assert_eq!(p.native.unwrap().gravity, Some(16_777_217));
    assert_eq!(p.native.unwrap().speed, Some(-16_777_219));
    assert_eq!(p.native.unwrap().client_number, Some(1));
    assert_eq!(p.native.unwrap().weapon_number, Some(65539));
    assert_eq!(p.inventory.weapon, Default::default());
    p.movement.ducked = false;
    let projection = qa_network::projection::PlayerProjection::load_native_q3(
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    let context = qa_network::projection::PlayerContext {
        namespace: Some(ResourceNamespace::Module(MODULE)),
        client_number: Some(0),
        weapon_number: Some(0),
        ..Default::default()
    };
    let mut words = [0; qa_network::states::PLAYER_WORDS];
    assert!(projection.reduce(
        p,
        &context,
        &mut words,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(words[19], residual | 32);
    assert_eq!(words[24], 16_777_217);
    assert_eq!(words[25], (-16_777_219i32) as u32);
    assert_eq!((words[40], words[41]), (1, 65539));
    // Native NONE and client zero are values, even with nonzero prior state.
    p.movement.tuning.gravity_multiplier = 0.0;
    p.movement.tuning.speed_multiplier = 0.0;
    vm.memory.write_word(PLAYERS + 48, 1200).unwrap();
    vm.memory.write_word(PLAYERS + 52, 250).unwrap();
    vm.memory.write_word(PLAYERS + 140, 0).unwrap();
    vm.memory.write_word(PLAYERS + 144, 0).unwrap();
    f.publish(&mut vm).unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(p.native.unwrap().client_number, Some(0));
    assert_eq!(p.native.unwrap().weapon_number, Some(0));
    assert_eq!(p.inventory.weapon, Default::default());
    assert_eq!(
        p.movement.tuning.gravity,
        Some(MovementMagnitude::Effective(1200.0))
    );
    assert_eq!(
        p.movement.tuning.max_speed,
        Some(MovementMagnitude::Effective(250.0))
    );
    let effective = qa_movement::effective_tuning(p.movement_rules, p.movement.tuning);
    assert_eq!((effective.gravity, effective.speed), (1200.0, 250.0));
}
fn qvm_export_relocation_zero_address_and_stale_bindings() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[
            (Opcode::Enter, 16),
            (Opcode::Const, PLAYERS as i32 + 20),
            (Opcode::Const, 42.5f32.to_bits() as i32),
            (Opcode::Store4, 0),
            (Opcode::Const, 0),
            (Opcode::Leave, 16),
        ],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    {
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut engine = f.engine();
        let mut calls = QvmCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(0),
            command: &[],
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [0; 10], 100, false), Ok(0));
    }
    f.publish(&mut vm).unwrap();
    assert_eq!(f.server.clients[1].player.body.position.0[0], 42.5);
    // A real nonzero QVM pointer can wrap to address0; null remains rejected.
    player(&mut vm.memory, 0);
    let wrapped = vm.memory.len() as u64;
    f.register(&mut vm, ENTITIES, wrapped, 768);
    f.publish(&mut vm).unwrap();
    verify(&f);
    assert!(
        f.call(
            &mut vm,
            15,
            &[0, ENTITIES as i32, 1, ENTITY_STRIDE as i32, 0, 768]
        )
        .is_err()
    );
    f.publish(&mut vm).unwrap();
    verify(&f); // Failed registration preserves previous checked memory.
    let id = f.server.clients[1].entity;
    f.server.entities.columns.owner[id.slot as usize] = ModuleId(2);
    assert_eq!(
        vm.bind_client(&f.server.entities, NATIVE, id),
        Err(CallError::Entity)
    );
    vm.memory.write_word(184, 99).unwrap();
    f.publish(&mut vm).unwrap();
    assert_eq!(f.server.clients[1].player.health, 79);
    f.server.entities.columns.owner[id.slot as usize] = MODULE;
    assert!(f.server.disconnect(ClientId(1)));
    assert_eq!(
        vm.bind_client(&f.server.entities, NATIVE, id),
        Err(CallError::Entity)
    );
    f.publish(&mut vm).unwrap();
    assert_eq!(f.server.clients[1].player.health, 0);
}
fn locate_uses_the_frozen_module_capacity_after_cvar_mutation() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    f.publish(&mut vm).unwrap();
    let view = f
        .cvars
        .bind(
            "sv_maxclients",
            Context {
                source: RuleSetId::Quake3,
                ..Context::default()
            },
        )
        .unwrap();
    f.cvars.force_write(view, "1").unwrap();
    let one_row = vm.memory.len() as u64 - 768;
    assert!(
        f.call(
            &mut vm,
            15,
            &[
                0,
                ENTITIES as i32,
                1,
                ENTITY_STRIDE as i32,
                one_row as i32,
                768
            ]
        )
        .is_err()
    );
    f.publish(&mut vm).unwrap();
    verify(&f);
    f.cvars.force_write(view, "64").unwrap();
    let two_rows = vm.memory.len() as u64 - 2 * 768;
    player(&mut vm.memory, two_rows);
    f.register(&mut vm, ENTITIES, two_rows, 768);
    f.publish(&mut vm).unwrap();
    verify(&f);
}

fn qvm_stat_layout_and_inventory_use_cached_bindings_and_queued_acquisition_time() {
    for layout in [Q3StatLayout::BASE, Q3StatLayout::TEAM_ARENA] {
        let mut f = Fixture::new();
        let mut vm = qvm(
            &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
            &f.registry,
        );
        vm.configure_q3_game(layout);
        vm.bind_player_values(value_bindings(&player_bindings()), &f.registry)
            .unwrap();
        assert_eq!(vm.player_stat_layout(), layout);
        entity(&mut vm.memory, ENTITIES);
        player(&mut vm.memory, PLAYERS);
        for index in 0..16 {
            vm.memory.write_word(PLAYERS + 184 + index * 4, 0).unwrap();
        }
        vm.memory
            .write_word(PLAYERS + 184 + u64::from(layout.health) * 4, 73)
            .unwrap();
        vm.memory
            .write_word(
                PLAYERS + 184 + u64::from(layout.weapons) * 4,
                (1 << 3) | (1 << 15),
            )
            .unwrap();
        vm.memory
            .write_word(PLAYERS + 184 + u64::from(layout.armor) * 4, 31)
            .unwrap();
        f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
        f.publish(&mut vm).unwrap();
        let shotgun = f
            .registry
            .weapon(f.registry.native_weapon(RuleSetId::Quake3, 3).unwrap())
            .unwrap();
        let weapon_item = shotgun.item.0 as usize;
        let ammo_item = shotgun.ammo.unwrap().0 as usize;
        let p = &f.server.clients[1].player;
        assert_eq!((p.health, p.armor), (73, 31));
        assert_eq!(p.inventory.item_counts[weapon_item], 1);
        assert_eq!(p.inventory.item_counts[ammo_item], 16_777_217);
        assert_eq!(p.inventory.item_acquired_at[weapon_item], 1.234);
        assert_eq!(p.inventory.item_acquired_at[ammo_item], 1.234);
        assert_eq!(p.movement.command_time_ms, -1234567);
        // Removal followed by reacquisition must use the new queued time, not
        // provider clock900 or the previously retained acquisition time.
        vm.memory
            .write_word(PLAYERS + 184 + u64::from(layout.weapons) * 4, 1 << 15)
            .unwrap();
        vm.memory.write_word(PLAYERS + 376 + 3 * 4, 0).unwrap();
        f.publish_at(&mut vm, EventTime::from_milliseconds(2345))
            .unwrap();
        assert_eq!(
            f.server.clients[1].player.inventory.item_counts[weapon_item],
            0
        );
        vm.memory
            .write_word(PLAYERS + 184 + u64::from(layout.weapons) * 4, 1 << 3)
            .unwrap();
        vm.memory.write_word(PLAYERS + 376 + 3 * 4, 29).unwrap();
        f.publish_at(&mut vm, EventTime::from_milliseconds(3456))
            .unwrap();
        let p = &f.server.clients[1].player;
        assert_eq!(p.inventory.item_acquired_at[weapon_item], 3.456);
        assert_eq!(p.inventory.item_acquired_at[ammo_item], 3.456);
        assert_eq!(
            p.values.get(ValueId(49)),
            Some(NumericValue::integer(i32::MAX - 1))
        );
    }
}

fn qvm_array_admission_and_inventory_capacity_reject_before_player_mutation() {
    for reject_inventory in [false, true] {
        let mut f = Fixture::new();
        let mut vm = qvm(
            &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
            &f.registry,
        );
        entity(&mut vm.memory, ENTITIES);
        player(&mut vm.memory, PLAYERS);
        f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
        let mut arrays = player_bindings();
        for index in 0..4 {
            let mut values = value_bindings(&arrays);
            values.arrays[index] = &values.arrays[index][..15];
            assert_eq!(
                vm.bind_player_values(values, &f.registry),
                Err(CallError::Capacity)
            );
        }
        f.publish(&mut vm).unwrap(); // A rejected rebind preserves the old metadata.
        let p = &mut f.server.clients[1].player;
        p.health = -77;
        p.armor = -19;
        p.body.position = Vec3([222.0; 3]);
        if reject_inventory {
            p.inventory.item_counts = vec![0; 1].into_boxed_slice();
            p.inventory.item_acquired_at = vec![0.0; 1].into_boxed_slice();
        } else {
            arrays[3][15].id = ValueId(64);
            vm.bind_player_values(value_bindings(&arrays), &f.registry)
                .unwrap();
        }
        let p = &f.server.clients[1].player;
        let before = (
            [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
            p.movement,
            p.view_angles,
            p.native,
            p.prediction,
            p.inventory.weapon,
        );
        vm.memory.write_word(PLAYERS + 184, 999).unwrap();
        vm.memory.write_word(PLAYERS + 376 + 3 * 4, 999).unwrap();
        assert_eq!(f.publish(&mut vm), Err(CallError::Capacity));
        let p = &f.server.clients[1].player;
        assert_eq!((p.health, p.armor), (-77, -19));
        assert_eq!(p.values.get(ValueId(0)), Some(NumericValue::integer(79)));
        assert_eq!(
            p.values.get(ValueId(35)),
            Some(NumericValue::integer(16_777_217))
        );
        assert_eq!(
            (
                [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
                p.movement,
                p.view_angles,
                p.native,
                p.prediction,
                p.inventory.weapon
            ),
            before
        );
    }
}

fn qvm_powerups_use_the_owning_native_clock_and_stats_only_publication() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    for index in 0..16 {
        vm.memory.write_word(PLAYERS + 312 + index * 4, 0).unwrap();
    }
    // Native time9000s and queued time100s are deliberately unrelated.
    for (native, deadline) in [
        (0, i32::MAX),
        (1, 9_012_500),
        (2, 8_999_000),
        (7, 9_000_000),
        (10, 9_000_000),
        (15, 9_033_000),
    ] {
        vm.memory
            .write_word(PLAYERS + 312 + native * 4, deadline)
            .unwrap();
    }
    let quad = *f.registry.native_powerup(RuleSetId::Quake3, 1).unwrap();
    let battlesuit = *f.registry.native_powerup(RuleSetId::Quake3, 2).unwrap();
    let flag = *f.registry.native_powerup(RuleSetId::Quake3, 7).unwrap();
    let scout = *f.registry.native_powerup(RuleSetId::Quake3, 10).unwrap();
    let invulnerability = *f.registry.native_powerup(RuleSetId::Quake3, 14).unwrap();
    assert!(!invulnerability.owns_item);
    f.server.clients[1].player.inventory.item_counts[invulnerability.item.0 as usize] = 1;
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    f.publish_with_clock(
        &mut vm,
        EventTime::from_milliseconds(100_000),
        ThinkTime::Milliseconds(9_000_000),
    )
    .unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(
        p.native_clock,
        Some(ClockObservation {
            native: ThinkTime::Milliseconds(9_000_000),
            queued: EventTime::from_milliseconds(100_000),
        })
    );
    assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 112.5);
    assert_eq!(
        p.inventory.powerup_until[usize::from(battlesuit.id.0)],
        99.0
    );
    assert_eq!(p.inventory.powerup_until[usize::from(flag.id.0)], 100.0);
    assert_eq!(p.inventory.powerup_until[usize::from(scout.id.0)], 100.0);
    assert_eq!(p.inventory.item_counts[quad.item.0 as usize], 1);
    assert_eq!(p.inventory.item_counts[battlesuit.item.0 as usize], 0);
    assert_eq!(p.inventory.item_counts[flag.item.0 as usize], 1);
    assert_eq!(p.inventory.item_counts[scout.item.0 as usize], 1);
    assert_eq!(p.inventory.item_counts[invulnerability.item.0 as usize], 1);
    assert_eq!(p.inventory.item_acquired_at[quad.item.0 as usize], 100.0);
    assert_eq!(
        p.values.get(ValueId(48)),
        Some(NumericValue::integer(i32::MAX))
    );
    assert_eq!(
        p.values.get(ValueId(49)),
        Some(NumericValue::integer(9_012_500))
    );
    assert_eq!(
        p.values.get(ValueId(63)),
        Some(NumericValue::integer(9_033_000))
    );

    f.server.clients[1].module_movement = false;
    let p = &f.server.clients[1].player;
    let before = (
        [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
        p.movement,
        p.view_angles,
        p.native,
        p.prediction,
        p.inventory.weapon,
    );
    vm.memory
        .write_vec3(PLAYERS + 20, Vec3([777.0; 3]))
        .unwrap();
    vm.memory
        .write_vec3(PLAYERS + 152, Vec3([888.0; 3]))
        .unwrap();
    vm.memory.write_word(PLAYERS + 12, 4096).unwrap();
    vm.memory.write_word(PLAYERS + 144, 999).unwrap();
    for (native, deadline) in [(1, 9_014_000), (2, 0), (7, 9_000_000), (14, 9_001_000)] {
        vm.memory
            .write_word(PLAYERS + 312 + native * 4, deadline)
            .unwrap();
    }
    f.publish_with_clock(
        &mut vm,
        EventTime::from_milliseconds(103_000),
        ThinkTime::Milliseconds(9_001_000),
    )
    .unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(
        (
            [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
            p.movement,
            p.view_angles,
            p.native,
            p.prediction,
            p.inventory.weapon,
        ),
        before
    );
    assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 116.0);
    assert_eq!(
        p.native_clock,
        Some(ClockObservation {
            native: ThinkTime::Milliseconds(9_001_000),
            queued: EventTime::from_milliseconds(103_000),
        })
    );
    assert_eq!(p.inventory.powerup_until[usize::from(battlesuit.id.0)], 0.0);
    assert_eq!(p.inventory.powerup_until[usize::from(flag.id.0)], 102.0);
    assert_eq!(p.inventory.item_counts[flag.item.0 as usize], 0);
    assert_eq!(
        p.inventory.powerup_until[usize::from(invulnerability.id.0)],
        103.0
    );
    assert_eq!(p.inventory.item_counts[invulnerability.item.0 as usize], 1);
    assert_eq!(
        p.values.get(ValueId(49)),
        Some(NumericValue::integer(9_014_000))
    );
}

fn qvm_powerup_capacity_and_clock_admission_preserve_player_state_on_rejection() {
    for invalid_clock in [false, true] {
        let mut f = Fixture::new();
        let mut vm = qvm(
            &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
            &f.registry,
        );
        entity(&mut vm.memory, ENTITIES);
        player(&mut vm.memory, PLAYERS);
        f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
        f.publish_with_clock(
            &mut vm,
            EventTime::from_milliseconds(137),
            ThinkTime::Milliseconds(5000),
        )
        .unwrap();
        assert_eq!(
            f.server.clients[1].player.native_clock,
            Some(ClockObservation {
                native: ThinkTime::Milliseconds(5000),
                queued: EventTime::from_milliseconds(137),
            })
        );
        let clock = if invalid_clock {
            ThinkTime::Seconds(9000.0)
        } else {
            let capacity = f.registry.powerup_capacity();
            f.server.clients[1].player.inventory.powerup_until =
                vec![0.0; capacity - 1].into_boxed_slice();
            ThinkTime::Milliseconds(9_000_000)
        };
        let p = &f.server.clients[1].player;
        let before = (
            [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
            p.movement,
            p.view_angles,
            p.native,
            p.prediction,
            p.inventory.weapon,
            p.health,
            p.armor,
            p.values.get(ValueId(49)),
            p.native_clock,
        );
        vm.memory
            .write_vec3(PLAYERS + 20, Vec3([999.0; 3]))
            .unwrap();
        vm.memory.write_word(PLAYERS + 184, 999).unwrap();
        vm.memory.write_word(PLAYERS + 316, 9_015_000).unwrap();
        assert_eq!(
            f.publish_with_clock(&mut vm, EventTime::from_milliseconds(100_000), clock),
            Err(CallError::Capacity)
        );
        let p = &f.server.clients[1].player;
        assert_eq!(
            (
                [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
                p.movement,
                p.view_angles,
                p.native,
                p.prediction,
                p.inventory.weapon,
                p.health,
                p.armor,
                p.values.get(ValueId(49)),
                p.native_clock,
            ),
            before
        );
    }
}

fn qvm_powerups_observe_the_signed_clock_supplied_to_the_guest() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[
            (Opcode::Enter, 16),
            (Opcode::Const, PLAYERS as i32 + 316),
            (Opcode::Local, 28),
            (Opcode::Load4, 0),
            (Opcode::Const, 1000),
            (Opcode::Add, 0),
            (Opcode::Store4, 0),
            (Opcode::Const, 0),
            (Opcode::Leave, 16),
        ],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    for index in 0..16 {
        vm.memory.write_word(PLAYERS + 312 + index * 4, 0).unwrap();
    }
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    let quad = *f.registry.native_powerup(RuleSetId::Quake3, 1).unwrap();
    for native_clock in [0x8000_0010i64, 0x1_8000_0010] {
        let queued = EventTime::from_milliseconds(137);
        let mut context = context();
        context.clock = ThinkTime::Milliseconds(native_clock);
        {
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut engine = f.engine();
            let mut calls = QvmCalls {
                services: &mut engine,
                table: &Q3_SERVER,
                context,
                platform_time: queued,
                command: &[],
                unknown: &mut unknown,
            };
            assert_eq!(
                vm.call(
                    &mut calls,
                    [8, native_clock as i32, 0, 0, 0, 0, 0, 0, 0, 0],
                    100,
                    false
                ),
                Ok(0)
            );
        }
        assert_eq!(vm.memory.read_word(PLAYERS + 316), Ok(-2_147_482_632));
        f.publish_with_clock(&mut vm, queued, context.clock)
            .unwrap();
        let p = &f.server.clients[1].player;
        assert_eq!(p.inventory.item_counts[quad.item.0 as usize], 1);
        assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 1.137);
        assert_eq!(
            p.native_clock,
            Some(ClockObservation {
                native: ThinkTime::Milliseconds(-2_147_483_632),
                queued,
            })
        );
        assert_eq!(
            p.values.get(ValueId(49)),
            Some(NumericValue::integer(-2_147_482_632))
        );
    }
}

fn qvm_nonframe_exports_keep_the_frame_clock_and_current_acquisition_time() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    for index in 0..16 {
        vm.memory.write_word(PLAYERS + 312 + index * 4, 0).unwrap();
    }
    vm.memory.write_word(PLAYERS + 312 + 10 * 4, 5000).unwrap();
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    f.publish_with_clock(
        &mut vm,
        EventTime::from_milliseconds(137),
        ThinkTime::Milliseconds(5000),
    )
    .unwrap();
    let scout = *f.registry.native_powerup(RuleSetId::Quake3, 10).unwrap();
    let quad = *f.registry.native_powerup(RuleSetId::Quake3, 1).unwrap();
    let frame_clock = Some(ClockObservation {
        native: ThinkTime::Milliseconds(5000),
        queued: EventTime::from_milliseconds(137),
    });
    assert_eq!(
        f.server.clients[1].player.inventory.item_counts[scout.item.0 as usize],
        1
    );
    vm.memory.write_word(PLAYERS + 316, 6000).unwrap();
    let mut context = context();
    context.clock = ThinkTime::Milliseconds(5100);
    vm.publish_entities(
        &mut f.engine(),
        context,
        EventTime::from_milliseconds(237),
        Some(7),
    )
    .unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(p.native_clock, frame_clock);
    assert_eq!(p.inventory.item_counts[scout.item.0 as usize], 1);
    assert_eq!(p.inventory.powerup_until[usize::from(scout.id.0)], 0.137);
    assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 1.137);
    assert_eq!(p.inventory.item_acquired_at[quad.item.0 as usize], 0.237);

    // A rejected RunFrame publication cannot replace the last admitted sample.
    let capacity = f.registry.powerup_capacity();
    let timers = std::mem::replace(
        &mut f.server.clients[1].player.inventory.powerup_until,
        vec![0.0; capacity - 1].into_boxed_slice(),
    );
    assert_eq!(
        f.publish_with_clock(&mut vm, EventTime::from_milliseconds(237), context.clock),
        Err(CallError::Capacity)
    );
    f.server.clients[1].player.inventory.powerup_until = timers;
    context.clock = ThinkTime::Milliseconds(5500);
    vm.publish_entities(
        &mut f.engine(),
        context,
        EventTime::from_milliseconds(637),
        Some(7),
    )
    .unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(p.native_clock, frame_clock);
    assert_eq!(p.inventory.item_counts[scout.item.0 as usize], 1);
    assert_eq!(p.inventory.item_acquired_at[quad.item.0 as usize], 0.237);

    f.publish_with_clock(&mut vm, EventTime::from_milliseconds(637), context.clock)
        .unwrap();
    let p = &f.server.clients[1].player;
    assert_eq!(p.inventory.item_counts[scout.item.0 as usize], 0);
    assert_eq!(p.inventory.powerup_until[usize::from(scout.id.0)], 0.137);
    assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 1.137);
    assert_eq!(
        p.native_clock,
        Some(ClockObservation {
            native: ThinkTime::Milliseconds(5500),
            queued: EventTime::from_milliseconds(637),
        })
    );
}

fn qvm_player_loop_sound_uses_public_memory_and_the_owning_namespace() {
    let mut f = Fixture::new();
    let mut vm = qvm(
        &[
            (Opcode::Enter, 16),
            (Opcode::Const, PLAYERS as i32 + 444),
            (Opcode::Local, 32),
            (Opcode::Load4, 0),
            (Opcode::Store4, 0),
            (Opcode::Const, 0),
            (Opcode::Leave, 16),
        ],
        &f.registry,
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    vm.memory.write_word(PLAYERS + 168, 0x1234_5678).unwrap();
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    let invoke = |vm: &mut Vm, f: &mut Fixture, ordinal: u32| {
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut engine = f.engine();
        let mut calls = QvmCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime::from_milliseconds(1234),
            command: &[],
            unknown: &mut unknown,
        };
        vm.call(
            &mut calls,
            [8, 900, ordinal as i32, 0, 0, 0, 0, 0, 0, 0],
            100,
            false,
        )
    };
    let projection = qa_network::projection::PlayerProjection::load_native_q3(
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    for ordinal in [0x8001_0005, 0x1_0005, 0] {
        f.server.clients[1].player.loop_sound =
            SoundRef::native(ResourceNamespace::Client(ClientId(0)), 7);
        assert_eq!(invoke(&mut vm, &mut f, ordinal), Ok(0));
        f.publish(&mut vm).unwrap();
        let p = &f.server.clients[1].player;
        assert_eq!(
            p.loop_sound,
            SoundRef::native(ResourceNamespace::Module(MODULE), ordinal)
        );
        assert_eq!(p.movement_rules, RuleSetId::QuakeWorld);
        assert_eq!(p.trace_rules, RuleSetId::Quake2);
        assert_eq!(vm.memory.read_word(PLAYERS + 444), Ok(ordinal as i32));
        assert_eq!(vm.memory.read_word(PLAYERS + 168), Ok(0x1234_5678));
        for namespace in [
            ResourceNamespace::Module(MODULE),
            ResourceNamespace::Module(ModuleId(9)),
            ResourceNamespace::Client(ClientId(1)),
        ] {
            let mut words = [0; qa_network::states::PLAYER_WORDS];
            assert!(projection.reduce(
                p,
                &qa_network::projection::PlayerContext {
                    namespace: Some(namespace),
                    ..Default::default()
                },
                &mut words,
                |_| None,
                |sound| (sound.namespace == namespace).then_some(sound.ordinal),
            ));
            assert_eq!(
                words[47],
                if namespace == ResourceNamespace::Module(MODULE) {
                    ordinal
                } else {
                    0
                }
            );
        }
    }
    // Engine-owned movement keeps its loop reference through stats-only import.
    f.server.clients[1].module_movement = false;
    let foreign = SoundRef::native(ResourceNamespace::Module(ModuleId(9)), 29);
    f.server.clients[1].player.loop_sound = foreign;
    for ordinal in [0, 0x8001_0005] {
        assert_eq!(invoke(&mut vm, &mut f, ordinal), Ok(0));
        f.publish(&mut vm).unwrap();
        assert_eq!(f.server.clients[1].player.loop_sound, foreign);
    }
}

fn qvm_player_signals_preserve_native_cells_and_publication_boundaries() {
    let mut f = Fixture::new();
    let mut instructions = vec![(Opcode::Enter, 16)];
    for (offset, value) in SIGNAL_CELLS {
        instructions.extend([
            (Opcode::Const, (PLAYERS + offset) as i32),
            (Opcode::Const, value as i32),
            (Opcode::Local, 32),
            (Opcode::Load4, 0),
            (Opcode::Add, 0),
            (Opcode::Store4, 0),
        ]);
    }
    instructions.extend([(Opcode::Const, 0), (Opcode::Leave, 16)]);
    let mut vm = qvm(&instructions, &f.registry);
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    let invoke = |vm: &mut Vm, f: &mut Fixture, increment: i32| {
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut engine = f.engine();
        let mut calls = QvmCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime::from_milliseconds(1234),
            command: &[],
            unknown: &mut unknown,
        };
        vm.call(
            &mut calls,
            [8, 900, increment, 0, 0, 0, 0, 0, 0, 0],
            200,
            false,
        )
    };
    let mut foreign =
        NativePlayerTail::new(RuleSetId::Quake2, ResourceNamespace::Client(ClientId(0)));
    foreign.signals = Some(PlayerSignals::default());
    f.server.clients[1].player.native = Some(foreign);
    assert_eq!(invoke(&mut vm, &mut f, 0), Ok(0));
    f.publish(&mut vm).unwrap();
    let accepted = f.server.clients[1].player.native.unwrap();
    assert_eq!(accepted.source, RuleSetId::Quake3);
    assert_eq!(accepted.namespace, ResourceNamespace::Module(MODULE));
    assert_eq!(accepted.signals, Some(SIGNAL_SAMPLE));
    assert_eq!(
        f.server.clients[1].player.movement_rules,
        RuleSetId::QuakeWorld
    );
    assert_eq!(f.server.clients[1].player.trace_rules, RuleSetId::Quake2);
    for (offset, value) in SIGNAL_CELLS {
        assert_eq!(vm.memory.read_word(PLAYERS + offset), Ok(value as i32));
    }
    let projection = qa_network::projection::PlayerProjection::load_native_q3(
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    for (source, namespace) in [
        (RuleSetId::Quake3, ResourceNamespace::Module(MODULE)),
        (RuleSetId::Quake3, ResourceNamespace::Module(ModuleId(9))),
        (RuleSetId::Quake3, ResourceNamespace::Client(ClientId(1))),
        (RuleSetId::Quake2, ResourceNamespace::Module(MODULE)),
    ] {
        let p = &mut f.server.clients[1].player;
        p.native.as_mut().unwrap().source = source;
        let mut words = [0; qa_network::states::PLAYER_WORDS];
        assert!(projection.reduce(
            p,
            &qa_network::projection::PlayerContext {
                namespace: Some(namespace),
                ..Default::default()
            },
            &mut words,
            |_| None,
            |_| None,
        ));
        let native = source == RuleSetId::Quake3 && namespace == ResourceNamespace::Module(MODULE);
        assert_eq!(
            [13, 16, 18, 38, 39, 23, 27, 29, 30, 31, 32].map(|word| words[word]),
            SIGNAL_CELLS.map(|(_, value)| if native { value } else { 0 })
        );
    }
    f.server.clients[1].player.native = Some(accepted);

    f.server.clients[1].module_movement = false;
    assert_eq!(invoke(&mut vm, &mut f, 1), Ok(0));
    f.publish(&mut vm).unwrap();
    assert_eq!(f.server.clients[1].player.native, Some(accepted));
    for (offset, value) in SIGNAL_CELLS {
        assert_eq!(
            vm.memory.read_word(PLAYERS + offset),
            Ok(value.wrapping_add(1) as i32)
        );
    }

    // Failed publication keeps the sample even after the guest changed it.
    f.server.clients[1].module_movement = true;
    assert_eq!(invoke(&mut vm, &mut f, 2), Ok(0));
    assert_eq!(
        f.publish_with_clock(
            &mut vm,
            EventTime::from_milliseconds(2234),
            ThinkTime::Seconds(900.0),
        ),
        Err(CallError::Capacity)
    );
    assert_eq!(f.server.clients[1].player.native, Some(accepted));

    let old_entity = f.server.clients[1].entity;
    f.server.entities.columns.owner[old_entity.slot as usize] = ModuleId(2);
    f.publish(&mut vm).unwrap();
    assert_eq!(f.server.clients[1].player.native, Some(accepted));
    f.server.entities.columns.owner[old_entity.slot as usize] = MODULE;
    assert!(f.server.disconnect(ClientId(1)));
    assert!(f.server.clients[1].player.native.is_none());
    f.publish(&mut vm).unwrap();
    assert!(f.server.clients[1].player.native.is_none());
    assert_eq!(
        vm.bind_client(&f.server.entities, NATIVE, old_entity),
        Err(CallError::Entity)
    );
    assert_eq!(
        f.server.connect(Connection::Local, MODULE, Some(NATIVE)),
        Some(ClientId(1))
    );
    assert_ne!(f.server.clients[1].entity, old_entity);
    assert!(f.server.clients[1].player.native.is_none());
    f.server.clients[1].module_movement = true;
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    assert_eq!(invoke(&mut vm, &mut f, 0), Ok(0));
    f.publish(&mut vm).unwrap();
    assert_eq!(
        f.server.clients[1].player.native.unwrap().signals,
        Some(SIGNAL_SAMPLE)
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn owned_native_export_imports_public_player_memory_only_after_success() {
    use qa_compat::native::{NamedExport, NativeCalls};
    use qa_formats::program::native::{Image as NativeImage, LoadRole};
    use qa_platform::native::NativeScalar;
    use std::time::Duration;
    let mut file = native_image::pe_export_fixture(64);
    native_image::put(&mut file, 208, 12288, 4);
    native_image::put(&mut file, 392 + 8, 8192, 4);
    native_image::put(&mut file, 392 + 16, 8192, 4);
    native_image::pe_rva(&mut file, 0x1140, 0x1200, 4);
    file.resize(8704, 0);
    native_image::pe_text(&mut file, 0x1200, &[0x31, 0xc0, 0xc3]);
    let image = NativeImage::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = qa_compat::native::Vm::map_image(
        image,
        &[NamedExport {
            name: b"GetGameAPI",
            command: Some(8),
            parameters: &[NativeScalar::I32, NativeScalar::I32],
            result: NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let mut f = Fixture::new();
    vm.configure_q3_game(Q3StatLayout::BASE);
    vm.bind_player_values(value_bindings(&player_bindings()), &f.registry)
        .unwrap();
    let native_entities = base + 0x1800;
    let native_players = base + 0x2400;
    let mut code = vec![0x48, 0x83, 0xec, 56, 0xb9, 15, 0, 0, 0, 0x48, 0xba];
    code.extend(native_entities.to_le_bytes());
    code.extend([0x41, 0xb8, 1, 0, 0, 0, 0x41, 0xb9]);
    code.extend((ENTITY_STRIDE as u32).to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend(native_players.to_le_bytes());
    code.extend([0x48, 0x89, 0x44, 0x24, 32, 0x48, 0xc7, 0x44, 0x24, 40]);
    code.extend((PLAYER_STRIDE as u32).to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend(vm.import_callback().to_le_bytes());
    code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 56, 0x31, 0xc0, 0xc3]);
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        entity(&mut memory, native_entities);
        player(&mut memory, native_players);
        memory.write(base + 0x1200, &code).unwrap();
    }
    let mut unknown = UnknownCalls::load(1).unwrap();
    let invoke = |vm: &mut qa_compat::native::Vm,
                  f: &mut Fixture,
                  unknown: &mut UnknownCalls,
                  clock: ThinkTime,
                  time: EventTime| {
        let mut context = context();
        context.clock = clock;
        let mut engine = f.engine();
        vm.call(
            &mut NativeCalls {
                services: &mut engine,
                table: &Q3_SERVER,
                context,
                platform_time: time,
                command: &[],
                raw_args: b"",
                unknown,
                native_player: None,
            },
            0,
            &[match clock {
                ThinkTime::Milliseconds(value) => value as u32 as u64,
                ThinkTime::Seconds(_) => 0,
            }],
        )
    };
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(900),
            EventTime::from_milliseconds(4567),
        )
        .unwrap(),
        0
    );
    vm.bind_client(&f.server.entities, NATIVE, f.server.clients[1].entity)
        .unwrap();
    verify(&f);
    let mut published_entity = vec![0x48, 0xb8];
    published_entity.extend(native_entities.to_le_bytes());
    for (index, value) in public_entity_cells().into_iter().enumerate() {
        published_entity.extend([0xc7, 0x80]);
        published_entity.extend((index as u32 * 4).to_le_bytes());
        published_entity.extend(value.to_le_bytes());
    }
    published_entity.extend([0xc7, 0x80]);
    published_entity.extend(416u32.to_le_bytes());
    published_entity.extend(1u32.to_le_bytes());
    published_entity.extend(&code);
    let area_before = f.server.area.bounds(f.server.clients[1].entity);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &published_entity)
        .unwrap();
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(900),
            EventTime::from_milliseconds(4567)
        )
        .unwrap(),
        0
    );
    verify_entity_fields(&f);
    assert_eq!(
        f.server.area.bounds(f.server.clients[1].entity),
        area_before
    );
    let weapon_item = f
        .registry
        .weapon(f.registry.native_weapon(RuleSetId::Quake3, 4).unwrap())
        .unwrap()
        .item
        .0 as usize;
    assert_eq!(
        f.server.clients[1].player.inventory.item_acquired_at[weapon_item],
        4.567
    );
    assert_eq!(vm.player_stat_layout(), Q3StatLayout::BASE);
    let quad = *f.registry.native_powerup(RuleSetId::Quake3, 1).unwrap();
    // The actual child receives the signed32 export clock and computes +1000.
    let mut wrapped = vec![0x48, 0xb8];
    wrapped.extend(native_players.to_le_bytes());
    wrapped.extend([0x81, 0xc2]);
    wrapped.extend(1000i32.to_le_bytes());
    wrapped.extend([0x89, 0x90]);
    wrapped.extend(316u32.to_le_bytes());
    wrapped.extend(&code);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &wrapped)
        .unwrap();
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(0x8000_0010),
            EventTime::from_milliseconds(137),
        )
        .unwrap(),
        0
    );
    let p = &f.server.clients[1].player;
    assert_eq!(p.inventory.item_counts[quad.item.0 as usize], 1);
    assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 1.137);
    assert_eq!(
        p.native_clock,
        Some(ClockObservation {
            native: ThinkTime::Milliseconds(-2_147_483_632),
            queued: EventTime::from_milliseconds(137),
        })
    );
    assert_eq!(
        p.values.get(ValueId(49)),
        Some(NumericValue::integer(-2_147_482_632))
    );
    let mut moved = vec![0x48, 0xb8];
    moved.extend(native_players.to_le_bytes());
    moved.extend([0xc7, 0x40, 20]);
    moved.extend(123.5f32.to_bits().to_le_bytes());
    moved.extend([0xc7, 0x40, 48]);
    moved.extend(16_777_217i32.to_le_bytes());
    moved.extend([0xc7, 0x40, 52]);
    moved.extend((-16_777_219i32).to_le_bytes());
    moved.extend([0xc7, 0x80]);
    moved.extend(316u32.to_le_bytes());
    moved.extend(9_012_500i32.to_le_bytes());
    moved.extend([0xc7, 0x80]);
    moved.extend(444u32.to_le_bytes());
    moved.extend(0x8001_0005u32.to_le_bytes());
    for (offset, value) in SIGNAL_CELLS {
        moved.extend([0xc7, 0x80]);
        moved.extend((offset as u32).to_le_bytes());
        moved.extend(value.to_le_bytes());
    }
    moved.extend(&code);
    f.server.clients[1].player.loop_sound =
        SoundRef::native(ResourceNamespace::Client(ClientId(0)), 5);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &moved)
        .unwrap();
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(9_000_000),
            EventTime::from_milliseconds(100_000),
        )
        .unwrap(),
        0
    );
    assert_eq!(f.server.clients[1].player.body.position.0[0], 123.5);
    let accepted_loop = SoundRef::native(ResourceNamespace::Module(MODULE), 0x8001_0005);
    assert_eq!(f.server.clients[1].player.loop_sound, accepted_loop);
    assert_eq!(
        f.server.clients[1].player.inventory.powerup_until[usize::from(quad.id.0)],
        112.5
    );
    assert_eq!(
        f.server.clients[1].player.values.get(ValueId(49)),
        Some(NumericValue::integer(9_012_500))
    );
    let accepted_tail = f.server.clients[1].player.native;
    assert_eq!(accepted_tail.unwrap().gravity, Some(16_777_217));
    assert_eq!(accepted_tail.unwrap().speed, Some(-16_777_219));
    assert_eq!(accepted_tail.unwrap().signals, Some(SIGNAL_SAMPLE));
    let mut clear_loop = vec![0x48, 0xb8];
    clear_loop.extend(native_players.to_le_bytes());
    clear_loop.extend([0xc7, 0x80]);
    clear_loop.extend(444u32.to_le_bytes());
    clear_loop.extend(0u32.to_le_bytes());
    clear_loop.extend(&code);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &clear_loop)
        .unwrap();
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(9_000_000),
            EventTime::from_milliseconds(100_000),
        )
        .unwrap(),
        0
    );
    assert_eq!(f.server.clients[1].player.loop_sound, None);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &moved)
        .unwrap();
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(9_000_000),
            EventTime::from_milliseconds(100_000),
        )
        .unwrap(),
        0
    );
    assert_eq!(f.server.clients[1].player.loop_sound, accepted_loop);
    f.server.clients[1].module_movement = false;
    let p = &f.server.clients[1].player;
    let before = (
        [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
        p.movement,
        p.view_angles,
        p.native,
        p.prediction,
        p.inventory.weapon,
        p.loop_sound,
    );
    let mut stats_only = vec![0x48, 0xb8];
    stats_only.extend(native_players.to_le_bytes());
    stats_only.extend([0xc7, 0x40, 20]);
    stats_only.extend(555.5f32.to_bits().to_le_bytes());
    stats_only.extend([0xc7, 0x40, 12]);
    stats_only.extend(4096i32.to_le_bytes());
    stats_only.extend([0xc7, 0x40, 48]);
    stats_only.extend(7i32.to_le_bytes());
    stats_only.extend([0xc7, 0x80]);
    stats_only.extend(316u32.to_le_bytes());
    stats_only.extend(9_014_000i32.to_le_bytes());
    stats_only.extend([0xc7, 0x80]);
    stats_only.extend(444u32.to_le_bytes());
    stats_only.extend(0u32.to_le_bytes());
    for (offset, _) in SIGNAL_CELLS {
        stats_only.extend([0xc7, 0x80]);
        stats_only.extend((offset as u32).to_le_bytes());
        stats_only.extend(0u32.to_le_bytes());
    }
    stats_only.extend(&code);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &stats_only)
        .unwrap();
    assert_eq!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(9_001_000),
            EventTime::from_milliseconds(103_000),
        )
        .unwrap(),
        0
    );
    let p = &f.server.clients[1].player;
    assert_eq!(
        (
            [p.body.position, p.body.velocity, p.body.mins, p.body.maxs],
            p.movement,
            p.view_angles,
            p.native,
            p.prediction,
            p.inventory.weapon,
            p.loop_sound,
        ),
        before
    );
    assert_eq!(p.inventory.powerup_until[usize::from(quad.id.0)], 116.0);
    let accepted_clock = Some(ClockObservation {
        native: ThinkTime::Milliseconds(9_001_000),
        queued: EventTime::from_milliseconds(103_000),
    });
    assert_eq!(p.native_clock, accepted_clock);
    assert_eq!(p.native.unwrap().signals, Some(SIGNAL_SAMPLE));
    assert_eq!(
        p.values.get(ValueId(49)),
        Some(NumericValue::integer(9_014_000))
    );
    // A failed native export does not publish bytes written before its trap.
    f.server.clients[1].module_movement = true;
    let mut failing = vec![0x48, 0xb8];
    failing.extend(native_players.to_le_bytes());
    failing.extend([0xc7, 0x40, 20]);
    failing.extend(777.5f32.to_bits().to_le_bytes());
    failing.extend([0xc7, 0x40, 48]);
    failing.extend(7i32.to_le_bytes());
    failing.extend([0xc7, 0x80]);
    failing.extend((376 + 3 * 4u32).to_le_bytes());
    failing.extend(999i32.to_le_bytes());
    failing.extend([0xc7, 0x80]);
    failing.extend(316u32.to_le_bytes());
    failing.extend(9_900_000i32.to_le_bytes());
    failing.extend([0xc7, 0x80]);
    failing.extend(444u32.to_le_bytes());
    failing.extend(17u32.to_le_bytes());
    for (offset, value) in SIGNAL_CELLS {
        failing.extend([0xc7, 0x80]);
        failing.extend((offset as u32).to_le_bytes());
        failing.extend(value.wrapping_add(1).to_le_bytes());
    }
    failing.extend([0x48, 0xb8]);
    failing.extend(native_entities.to_le_bytes());
    failing.extend([0xc7, 0x40, 92]);
    failing.extend(999.5f32.to_bits().to_le_bytes());
    failing.extend([0x0f, 0x0b]);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &failing)
        .unwrap();
    assert!(
        invoke(
            &mut vm,
            &mut f,
            &mut unknown,
            ThinkTime::Milliseconds(9_002_000),
            EventTime::from_milliseconds(105_000),
        )
        .is_err()
    );
    assert_eq!(f.server.clients[1].player.body.position.0[0], 123.5);
    assert_eq!(f.server.clients[1].player.native, accepted_tail);
    assert_eq!(
        f.server.clients[1].player.native.unwrap().signals,
        Some(SIGNAL_SAMPLE)
    );
    assert_eq!(f.server.clients[1].player.loop_sound, accepted_loop);
    assert_eq!(f.server.clients[1].player.native_clock, accepted_clock);
    verify_entity_fields(&f);
    assert_eq!(
        f.server.clients[1].player.inventory.powerup_until[usize::from(quad.id.0)],
        116.0
    );
    assert_eq!(
        f.server.clients[1].player.values.get(ValueId(49)),
        Some(NumericValue::integer(9_014_000))
    );
    assert_eq!(
        f.server.clients[1].player.values.get(ValueId(35)),
        Some(NumericValue::integer(16_777_217))
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn owned_pe64_shader_imports_borrow_only_the_parked_child_and_share_cold_dispatch() {
    use native_image::*;
    use qa_compat::abi::Q3_CLIENT;
    use qa_compat::native::{NamedExport, NativeCalls, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 208, 12288, 4);
    put(&mut file, 392 + 8, 8192, 4);
    put(&mut file, 392 + 16, 8192, 4);
    pe_rva(&mut file, 0x1140, 0x1200, 4);
    file.resize(8704, 0);
    // The shared PE fixture advertises DllMain at RVA 0x1080. Keep its
    // attach/detach entry executable and successful, separate from vmMain.
    pe_text(&mut file, 0x1080, &[0xb8, 1, 0, 0, 0, 0xc3]);
    pe_text(&mut file, 0x1200, &[0x31, 0xc0, 0xc3]);
    let mut vm = Vm::map_image(
        Image::parse(&file, None, LoadRole::Library).unwrap(),
        &[
            NamedExport {
                name: b"GetGameAPI",
                command: Some(39),
                parameters: &[NativeScalar::Word; 2],
                result: NativeScalar::Word,
            },
            NamedExport {
                name: b"GetGameAPI",
                command: Some(57),
                parameters: &[NativeScalar::Word; 2],
                result: NativeScalar::Word,
            },
        ],
        &[],
        std::time::Duration::from_secs(3),
    )
    .unwrap();
    let base = vm.process.base();
    let mut code = vec![0x48, 0x83, 0xec, 0x28, 0x48, 0xb8];
    code.extend(vm.import_callback().to_le_bytes());
    code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        memory.write(base + 0x1200, &code).unwrap();
        memory.write(base + 0x1800, b"ui/pic\0").unwrap();
    }
    let mut fixture = Fixture::new();
    let mut shader = ShaderLoader::default();
    let mut unknown = UnknownCalls::load(1).unwrap();
    {
        let mut engine = fixture.engine();
        engine.model_loader = Some(&mut shader);
        let mut calls = NativeCalls {
            services: &mut engine,
            table: &Q3_CLIENT,
            context: context(),
            platform_time: EventTime(17),
            command: &[],
            raw_args: b"",
            unknown: &mut unknown,
            native_player: None,
        };
        for ordinal in [0, 1] {
            assert_eq!(vm.call(&mut calls, ordinal, &[base + 0x1800]).unwrap(), 42);
        }
    }
    assert_eq!(
        shader.calls,
        vec![
            (b"ui/pic".to_vec(), RuleSetId::Quake3, true),
            (b"ui/pic".to_vec(), RuleSetId::Quake3, false)
        ]
    );
    assert_eq!(unknown.calls, 0);
}

#[path = "support/drop_client.rs"]
mod drop_client;

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    drop_client::run();
    qvm_native_memory_preserves_full_widths_roles_and_unlinked_player_origin();
    qvm_metadata_preserves_raw_native_words_and_zero_multiplier_tuning();
    qvm_export_relocation_zero_address_and_stale_bindings();
    locate_uses_the_frozen_module_capacity_after_cvar_mutation();
    qvm_stat_layout_and_inventory_use_cached_bindings_and_queued_acquisition_time();
    qvm_array_admission_and_inventory_capacity_reject_before_player_mutation();
    qvm_powerups_use_the_owning_native_clock_and_stats_only_publication();
    qvm_powerup_capacity_and_clock_admission_preserve_player_state_on_rejection();
    qvm_powerups_observe_the_signed_clock_supplied_to_the_guest();
    qvm_nonframe_exports_keep_the_frame_clock_and_current_acquisition_time();
    qvm_player_loop_sound_uses_public_memory_and_the_owning_namespace();
    qvm_player_signals_preserve_native_cells_and_publication_boundaries();
    qvm_entity_publication_preserves_all_fields_and_separate_physical_pose();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    owned_native_export_imports_public_player_memory_only_after_success();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    owned_pe64_shader_imports_borrow_only_the_parked_child_and_share_cold_dispatch();
    println!("Q3 public player memory and owned shader imports: all checks passed");
}
