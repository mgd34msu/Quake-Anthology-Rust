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
        ClientId, ModuleId, MovementMode, NativeEntity, NumericValue, ResourceNamespace, RuleSetId,
        ThinkTime, ValueBinding, ValueId, ValueReset, ValueWidth, Vec3, WeaponReadiness,
    },
    sys_events::EventTime,
};
use qa_formats::program::qvm::{Image, Opcode};
use qa_session::clients::{Connection, Server};
use qa_world::{area::LinkOrder, collision::CollisionStore, entities::AllocationPolicy};

const ENTITIES: u64 = 4096;
const PLAYERS: u64 = 16384;
const ENTITY_STRIDE: u64 = 608;
const PLAYER_STRIDE: u64 = 640;
const MODULE: ModuleId = ModuleId(1);
const NATIVE: NativeEntity = NativeEntity {
    module: MODULE,
    slot: 0,
};

fn bindings(first: u32) -> [ValueBinding; 16] {
    std::array::from_fn(|index| ValueBinding {
        id: ValueId(first + index as u32),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    })
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
        let mut server = Server::load(2, 16, 1, 1, 1, 32).unwrap();
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
            input: &mut self.input,
            cvars: &mut self.cvars,
            commands: &mut self.commands,
            client_commands: None,
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
        vm.publish_entities(&mut self.engine(), context())
    }
}
fn qvm(instructions: &[(Opcode, i32)]) -> Vm {
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
    vm.configure_q3_game();
    vm.bind_player_values(&bindings(0), &bindings(16)).unwrap();
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
        (140, 0),
        (144, 4),
        (164, -71),
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
    for (offset, first) in [(184, 0), (248, 16)] {
        for index in 0..16 {
            memory
                .write_word(
                    address + offset + index * 4,
                    match first + index {
                        0 => 79,
                        3 => 27,
                        14 => i32::MIN,
                        15 => i32::MAX,
                        16 => -9,
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
            qa_network::projection::PlayerProjection::load_native_q3(&bindings(0), &bindings(16)),
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
        assert!(projection.reduce(p, &Default::default(), &mut words, |_| None));
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
    assert_eq!(
        f.server.entities.columns.position[row.entity.slot as usize],
        Vec3([-900.0, 2.0, 3.0])
    );
    assert_eq!(f.server.clients[0].player.health, 0);
    assert!(f.server.area.bounds(row.entity).is_none());
}
fn qvm_native_memory_preserves_full_widths_roles_and_unlinked_player_origin() {
    let mut f = Fixture::new();
    let mut vm = qvm(&[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)]);
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
}

fn qvm_metadata_preserves_raw_native_words_and_zero_multiplier_tuning() {
    use qa_core::primitives::MovementMagnitude;
    let mut f = Fixture::new();
    let mut vm = qvm(&[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)]);
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
    let projection = qa_network::projection::PlayerProjection::load_native_q3(&[], &[]);
    let context = qa_network::projection::PlayerContext {
        namespace: Some(ResourceNamespace::Module(MODULE)),
        client_number: Some(0),
        weapon_number: Some(0),
        ..Default::default()
    };
    let mut words = [0; qa_network::states::PLAYER_WORDS];
    assert!(projection.reduce(p, &context, &mut words, |_| None));
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
    let mut vm = qvm(&[
        (Opcode::Enter, 16),
        (Opcode::Const, PLAYERS as i32 + 20),
        (Opcode::Const, 42.5f32.to_bits() as i32),
        (Opcode::Store4, 0),
        (Opcode::Const, 0),
        (Opcode::Leave, 16),
    ]);
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
    let mut vm = qvm(&[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)]);
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
            command: None,
            parameters: &[],
            result: NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    vm.configure_q3_game();
    vm.bind_player_values(&bindings(0), &bindings(16)).unwrap();
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
    let mut f = Fixture::new();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let invoke = |vm: &mut qa_compat::native::Vm, f: &mut Fixture, unknown: &mut UnknownCalls| {
        let mut engine = f.engine();
        vm.call(
            &mut NativeCalls {
                services: &mut engine,
                table: &Q3_SERVER,
                context: context(),
                platform_time: EventTime(0),
                command: &[],
                raw_args: b"",
                unknown,
                native_player: None,
            },
            0,
            &[],
        )
    };
    assert_eq!(invoke(&mut vm, &mut f, &mut unknown).unwrap(), 0);
    vm.bind_client(&f.server.entities, NATIVE, f.server.clients[1].entity)
        .unwrap();
    verify(&f);
    let mut moved = vec![0x48, 0xb8];
    moved.extend(native_players.to_le_bytes());
    moved.extend([0xc7, 0x40, 20]);
    moved.extend(123.5f32.to_bits().to_le_bytes());
    moved.extend([0xc7, 0x40, 48]);
    moved.extend(16_777_217i32.to_le_bytes());
    moved.extend([0xc7, 0x40, 52]);
    moved.extend((-16_777_219i32).to_le_bytes());
    moved.extend(&code);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &moved)
        .unwrap();
    assert_eq!(invoke(&mut vm, &mut f, &mut unknown).unwrap(), 0);
    assert_eq!(f.server.clients[1].player.body.position.0[0], 123.5);
    let accepted_tail = f.server.clients[1].player.native;
    assert_eq!(accepted_tail.unwrap().gravity, Some(16_777_217));
    assert_eq!(accepted_tail.unwrap().speed, Some(-16_777_219));
    // A failed native export does not publish bytes written before its trap.
    let mut failing = vec![0x48, 0xb8];
    failing.extend(native_players.to_le_bytes());
    failing.extend([0xc7, 0x40, 20]);
    failing.extend(777.5f32.to_bits().to_le_bytes());
    failing.extend([0xc7, 0x40, 48]);
    failing.extend(7i32.to_le_bytes());
    failing.extend([0x0f, 0x0b]);
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &failing)
        .unwrap();
    assert!(invoke(&mut vm, &mut f, &mut unknown).is_err());
    assert_eq!(f.server.clients[1].player.body.position.0[0], 123.5);
    assert_eq!(f.server.clients[1].player.native, accepted_tail);
}

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    qvm_native_memory_preserves_full_widths_roles_and_unlinked_player_origin();
    qvm_metadata_preserves_raw_native_words_and_zero_multiplier_tuning();
    qvm_export_relocation_zero_address_and_stale_bindings();
    locate_uses_the_frozen_module_capacity_after_cvar_mutation();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    owned_native_export_imports_public_player_memory_only_after_success();
    println!("Q3 public player memory: all checks passed");
}
