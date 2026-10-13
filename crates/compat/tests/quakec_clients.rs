#[path = "support/quakec_program.rs"]
#[allow(dead_code)]
mod quakec_program;

use qa_compat::{
    quakec::{Builtins, Layout, QW_USERINFO_BYTES, Trap, Vm},
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices, ServiceStorage},
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{names::NameTable, primitives::*};
use qa_formats::program::quakec::{Image, Opcode, Statement};
use qa_session::{
    clients::{Connection, Server},
    dispatch::{FunctionBinding, FunctionTable},
};
use qa_world::{
    area::LinkOrder,
    collision::{CollisionStore, Contents},
    entities::AllocationPolicy,
};

const FIELDS: &[(u16, u16, &[u8])] = &[
    (3, 0, b"origin"),
    (3, 3, b"velocity"),
    (3, 6, b"angles"),
    (3, 9, b"mins"),
    (3, 12, b"maxs"),
    (3, 15, b"absmin"),
    (3, 18, b"absmax"),
    (2, 21, b"solid"),
    (2, 22, b"flags"),
    (4, 23, b"owner"),
    (4, 24, b"groundentity"),
    (2, 25, b"movetype"),
    (2, 26, b"health"),
    (2, 27, b"armorvalue"),
    (2, 28, b"armortype"),
    (2, 29, b"frags"),
    (2, 30, b"currentammo"),
    (2, 31, b"ammo_shells"),
    (2, 32, b"ammo_nails"),
    (2, 33, b"ammo_rockets"),
    (2, 34, b"ammo_cells"),
    (2, 35, b"items"),
    (2, 36, b"weapon"),
    (1, 37, b"weaponmodel"),
    (2, 38, b"weaponframe"),
    (3, 39, b"view_ofs"),
    (3, 42, b"punchangle"),
    (3, 45, b"v_angle"),
    (2, 48, b"fixangle"),
    (2, 49, b"waterlevel"),
    (2, 50, b"watertype"),
    (2, 51, b"teleport_time"),
    (2, 52, b"button0"),
    (2, 53, b"button2"),
    (2, 54, b"impulse"),
    (2, 55, b"team"),
    (1, 56, b"netname"),
    (2, 57, b"gravity"),
    (2, 58, b"maxspeed"),
    (6, 59, b"touch"),
    (2, 60, b"colormap"),
    (2, 61, b"modelindex"),
    (2, 62, b"frame"),
    (2, 63, b"skin"),
    (2, 64, b"effects"),
    (3, 65, b"avelocity"),
    (2, 68, b"items2"),
    (1, 69, b"classname"),
];

fn vm(mutate: impl FnOnce(&mut Image)) -> Vm {
    let mut globals = vec![
        (4, 34, &b"self"[..]),
        (2, 35, &b"time"[..]),
        (4, 36, &b"other"[..]),
        (2, 37, &b"frametime"[..]),
        (2, 38, &b"serverflags"[..]),
    ];
    for (index, name) in [
        b"SetNewParms".as_slice(),
        b"ClientConnect",
        b"PutClientInServer",
        b"PlayerPreThink",
        b"PlayerPostThink",
        b"ClientDisconnect",
    ]
    .into_iter()
    .enumerate()
    {
        globals.push((6, 70 + index as u16, name));
    }
    let parm_names: Vec<String> = (1..=16).map(|index| format!("parm{index}")).collect();
    for (index, name) in parm_names.iter().enumerate() {
        globals.push((2, 80 + index as u16, name.as_bytes()));
    }
    for (index, name) in [
        b"total_secrets".as_slice(),
        b"total_monsters",
        b"found_secrets",
        b"killed_monsters",
    ]
    .into_iter()
    .enumerate()
    {
        globals.push((2, 100 + index as u16, name));
    }
    let bytes = quakec_program::program_with_definitions(
        &[(Opcode::Return, 1, 0, 0)],
        &[0, 1, 1, 1, 1, 1, 1],
        &globals,
        FIELDS,
        72,
        128,
    );
    let mut image = Image::parse(&bytes, Some(5927)).unwrap();
    for index in 0..6 {
        image.globals[70 + index] = (index + 1) as u32;
    }
    mutate(&mut image);
    Vm::load(
        image,
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 2048,
            state_step: 0.1,
        },
    )
    .unwrap()
}
fn read(vm: &Vm, native: u32, name: &[u8]) -> u32 {
    let field = vm.image.field(name).unwrap();
    vm.entities
        .read_word(vm.field_address(native, field as u32, 1).unwrap())
        .unwrap() as u32
}
fn write(vm: &mut Vm, native: u32, name: &[u8], words: &[u32]) {
    let field = vm.image.field(name).unwrap();
    let address = vm.field_address(native, field as u32, words.len()).unwrap();
    for (index, word) in words.iter().enumerate() {
        vm.entities
            .write_word(address + index as u64 * 4, *word as i32)
            .unwrap();
    }
}
fn number(vm: &mut Vm, native: u32, name: &[u8], value: f32) {
    write(vm, native, name, &[value.to_bits()]);
}
fn vector(vm: &mut Vm, native: u32, name: &[u8], value: Vec3) {
    write(vm, native, name, &value.0.map(f32::to_bits));
}
fn read_vector(vm: &Vm, native: u32, name: &[u8]) -> Vec3 {
    let field = vm.image.field(name).unwrap();
    vm.entities
        .read_vec3(vm.field_address(native, field as u32, 3).unwrap())
        .unwrap()
}
struct NoImports;
impl Builtins for NoImports {
    fn call(&mut self, _: &mut Vm, _: u32, _: usize) -> Result<(), Trap> {
        Err(Trap::Builtin)
    }
}
struct NoQueries;
impl qa_movement::TraceServices for NoQueries {
    fn trace(
        &mut self,
        _: qa_world::collision::TraceQuery,
    ) -> Result<qa_world::collision::Trace, qa_world::collision::TraceError> {
        panic!("NOCLIP must not trace");
    }
    fn point_contents(
        &self,
        _: Vec3,
        _: qa_world::collision::EntityTracePolicy,
    ) -> Result<Contents, qa_world::collision::TraceError> {
        panic!("NOCLIP must not query contents");
    }
}
fn bind_callbacks(vm: &mut Vm) {
    let entries = [6, 5, 4, 3, 2, 1, 0];
    let table = FunctionTable::<Server>::load([(
        ModuleId(4),
        RuleSetId::QuakeWorld,
        Vec::from(entries.map(|entry| FunctionBinding {
            entry,
            call: |_, _, _, _| true,
        })),
    )])
    .unwrap();
    vm.bind_thinks(table.module_binding(ModuleId(4)), &entries);
}
fn with_client(
    vm: &mut Vm,
    run: impl FnOnce(&mut Vm, &mut EngineServices<'_>, CallContext, u32, ClientId),
) {
    with_native_client(vm, RuleSetId::QuakeWorld, run)
}
fn with_native_client(
    vm: &mut Vm,
    source: RuleSetId,
    run: impl FnOnce(&mut Vm, &mut EngineServices<'_>, CallContext, u32, ClientId),
) {
    let mut names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
    let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
    let mut server = Server::load(
        2,
        80,
        registry.items.len() + 1,
        32,
        registry.weapons.len() + 1,
        33,
    )
    .unwrap();
    let context = CallContext {
        module: ModuleId(4),
        clock: ThinkTime::Seconds(10.0),
        server_frame: 1,
        native_rules: source,
        trace_rules: RuleSetId::Quake2,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
        console: Context {
            source: RuleSetId::Quake2Rerelease,
            ..Context::default()
        },
        allocation: AllocationPolicy::EDICT,
        link_order: LinkOrder::Tail,
    };
    let anchor = server
        .entities
        .allocate(context.clock, context.module, context.allocation)
        .unwrap()
        .id;
    server.entities.columns.native_entity[anchor.slot as usize] = Some(NativeEntity {
        module: context.module,
        slot: 0,
    });
    let native_identity = NativeEntity {
        module: context.module,
        slot: if source == RuleSetId::QuakeWorld {
            17
        } else {
            2
        },
    };
    let client = server
        .connect(Connection::Local, context.module, Some(native_identity))
        .unwrap();
    let entity = server.clients[client.0 as usize].entity;
    assert_ne!(entity.slot as i32, native_identity.slot);
    vm.seed_entities(&server.entities, context.module, context.native_rules, 2)
        .unwrap();
    bind_callbacks(vm);
    vm.bind_client(&server.entities, native_identity, entity)
        .unwrap();
    let player = &mut server.clients[client.0 as usize].player;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake2;
    qa_movement::set_bounds(player);
    let native = native_identity.slot as u32 * vm.stride as u32;
    let mut cvars = Cvars::new().unwrap();
    vm.bind_client_settings(&cvars, &registry).unwrap();
    let values: Vec<_> = (0..32)
        .map(|id| ValueBinding {
            id: ValueId(id),
            width: ValueWidth::Signed32,
            reset: ValueReset::Life,
        })
        .collect();
    vm.bind_player_values(
        &values,
        (source == RuleSetId::Quake).then_some(ValueBinding {
            id: ValueId(32),
            width: ValueWidth::Signed32,
            reset: ValueReset::Life,
        }),
    )
    .unwrap();
    let mut input = qa_input::Input::load();
    let mut commands = CommandBuffer::new();
    let vfs = qa_content::vfs::Vfs::default();
    let localization = qa_content::localization::Catalog::default();
    let layout = qa_core::configstrings::ConfigLayout::for_rules(context.native_rules);
    let mut storage = ServiceStorage::load_native(
        &[(context.module, layout.count, context.native_rules)],
        0,
        &cvars,
    )
    .unwrap();
    storage.bind_lifetime(context.module, anchor).unwrap();
    let geometry = CollisionStore::new();
    let mut scratch = geometry.scratch();
    let mut services = EngineServices {
        server: &mut server,
        input: &mut input,
        cvars: &mut cvars,
        commands: &mut commands,
        client_connection: None,
        renderer: None,
        client_audio: None,
        vfs: &vfs,
        model_loader: None,
        model_lookup: None,
        sound_loader: None,
        sound_bank: None,
        localization: &localization,
        registry: &registry,
        names: &mut names,
        storage: &mut storage,
        geometry: &geometry,
        world: None,
        visibility: None,
        portals: None,
        scratch: &mut scratch,
    };
    run(vm, &mut services, context, native, client);
}

#[test]
fn client_callbacks_use_current_native_words_and_qw_context_phases() {
    let mut vm = vm(|_| {});
    with_client(&mut vm, |vm, _, _, native, _| {
        vm.image.globals[34] = 111;
        vm.image.globals[35] = 9.25f32.to_bits();
        vm.image.globals[36] = 222;
        assert_eq!(vm.client_count(), Ok(32));
        assert_eq!(
            vm.client_callback(ClientCallback::Connect),
            Ok(CallbackId(4))
        );
        let restore = vm
            .prepare_client_call(
                ClientCallback::NewParameters,
                native,
                ThinkTime::Seconds(15.0),
            )
            .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[111, 9.25f32.to_bits(), 222]);
        vm.finish_client_call(restore).unwrap();
        for phase in [
            ClientCallback::Connect,
            ClientCallback::Begin,
            ClientCallback::BeforeMove,
            ClientCallback::AfterMove,
        ] {
            let restore = vm
                .prepare_client_call(phase, native, ThinkTime::Seconds(15.0))
                .unwrap();
            assert_eq!(&vm.image.globals[34..37], &[native, 15.0f32.to_bits(), 222]);
            vm.finish_client_call(restore).unwrap();
            vm.image.globals[34] = 777;
            vm.image.globals[35] = 7.0f32.to_bits();
        }
        let restore = vm
            .prepare_client_call(ClientCallback::Disconnect, native, ThinkTime::Seconds(99.0))
            .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[native, 7.0f32.to_bits(), 222]);
        vm.finish_client_call(restore).unwrap();
        vm.image.globals[71] = 6;
        assert_eq!(
            vm.client_callback(ClientCallback::Connect),
            Ok(CallbackId(0))
        );
        vm.image.globals[71] = 0;
        assert_eq!(
            vm.client_callback(ClientCallback::Connect),
            Ok(CallbackId(6))
        );
        assert_eq!(
            vm.client_callback(ClientCallback::Command),
            Err(CallError::Unsupported)
        );
        let restore = vm.prepare_touch_call(0, native, None).unwrap();
        assert_eq!(&vm.image.globals[34..37], &[0, 7.0f32.to_bits(), native]);
        vm.finish_client_call(restore).unwrap();
        assert_eq!(&vm.image.globals[34..37], &[native, 7.0f32.to_bits(), 222]);
        let restore = vm
            .prepare_touch_call(0, native, Some(ThinkTime::Seconds(20.0)))
            .unwrap();
        assert_eq!(vm.image.globals[35], 20.0f32.to_bits());
        vm.finish_client_call(restore).unwrap();
        write(vm, native, b"touch", &[6]);
        assert_eq!(vm.touch_callback(native), Ok(Some(CallbackId(0))));
        write(vm, native, b"touch", &[0]);
        assert_eq!(vm.touch_callback(native), Ok(None));
        let parms = std::array::from_fn(|index| 0x80000000u32.wrapping_add(index as u32));
        vm.write_spawn_parms(parms).unwrap();
        assert_eq!(vm.read_spawn_parms().unwrap(), parms);
    });
}

#[test]
fn spawn_clears_only_selected_reserved_row_and_retains_raw_name() {
    let mut vm = vm(|_| {});
    with_client(&mut vm, |vm, services, context, native, client| {
        number(vm, native, b"health", 77.0);
        number(vm, 0, b"health", 123.0);
        let speed = services
            .cvars
            .bind(
                "sv_maxspeed",
                qa_console::views::Context {
                    source: RuleSetId::QuakeWorld,
                    ..Context::default()
                },
            )
            .unwrap();
        services.cvars.force_write(speed, "333").unwrap();
        vm.prepare_client_spawn(services, context, native, b"\\name\\raw\xff", 0)
            .unwrap();
        assert_eq!(read(vm, 0, b"health"), 123.0f32.to_bits());
        assert_eq!(read(vm, native, b"health"), 0);
        assert_eq!(read(vm, native, b"colormap"), 17.0f32.to_bits());
        assert_eq!(read(vm, native, b"team"), 0);
        assert_eq!(read(vm, native, b"gravity"), 1.0f32.to_bits());
        assert_eq!(read(vm, native, b"maxspeed"), 333.0f32.to_bits());
        let name = read(vm, native, b"netname");
        assert_eq!(vm.strings.cstring(u64::from(name)).unwrap(), b"raw\xff");
        assert_eq!(
            services.server.clients[client.0 as usize]
                .player
                .movement_rules,
            RuleSetId::Quake3
        );
        assert_eq!(
            services.server.clients[client.0 as usize]
                .player
                .trace_rules,
            RuleSetId::Quake2
        );
        assert_eq!(
            vm.prepare_client_spawn(services, context, native, &[b'a'; QW_USERINFO_BYTES], 0),
            Err(CallError::Memory)
        );
    });
}

#[test]
fn nq_callbacks_preserve_connect_globals_and_restore_only_native_saved_words() {
    let mut vm = vm(|image| {
        image.statements = [
            (Opcode::StoreEnt, [110, 34, 0]),
            (Opcode::StoreF, [111, 35, 0]),
            (Opcode::StoreEnt, [112, 36, 0]),
            (Opcode::Return, [1, 0, 0]),
            (Opcode::StoreEnt, [34, 113, 0]),
            (Opcode::StoreF, [35, 114, 0]),
            (Opcode::StoreEnt, [36, 115, 0]),
            (Opcode::Return, [1, 0, 0]),
            (Opcode::StoreEnt, [110, 34, 0]),
            (Opcode::StoreF, [111, 35, 0]),
            (Opcode::StoreEnt, [112, 36, 0]),
            (Opcode::Invalid, [0, 0, 0]),
        ]
        .map(|(opcode, operands)| Statement {
            opcode,
            source_opcode: opcode as u16,
            operands,
        })
        .to_vec()
        .into_boxed_slice();
        image.functions[2].first_statement = 0;
        image.functions[3].first_statement = 4;
        image.functions[6].first_statement = 8;
    });
    with_native_client(&mut vm, RuleSetId::Quake, |vm, _, context, native, _| {
        for (event, callback) in [
            (ClientCallback::NewParameters, 5),
            (ClientCallback::Connect, 4),
            (ClientCallback::Begin, 3),
            (ClientCallback::BeforeMove, 2),
            (ClientCallback::AfterMove, 1),
            (ClientCallback::Disconnect, 0),
        ] {
            assert_eq!(vm.client_callback(event), Ok(CallbackId(callback)));
        }
        vm.image.globals[34] = 111;
        vm.image.globals[35] = 9.25f32.to_bits();
        vm.image.globals[36] = 222;
        let restore = vm
            .prepare_client_call(ClientCallback::NewParameters, native, context.clock)
            .unwrap();
        vm.finish_client_call(restore).unwrap();
        assert_eq!(&vm.image.globals[34..37], &[111, 9.25f32.to_bits(), 222]);
        let parms = std::array::from_fn(|index| 0x7f80_1234u32.wrapping_add(index as u32));
        vm.write_spawn_parms(parms).unwrap();
        assert_eq!(vm.read_spawn_parms().unwrap(), parms);

        vm.image.globals[110..113].copy_from_slice(&[333, 7.5f32.to_bits(), 444]);
        let restore = vm
            .prepare_client_call(ClientCallback::Connect, native, context.clock)
            .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[native, 10.0f32.to_bits(), 222]);
        vm.call(&mut NoImports, 2, 64, false).unwrap();
        vm.finish_client_call(restore).unwrap();
        let restore = vm
            .prepare_client_call(ClientCallback::Begin, native, context.clock)
            .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[333, 7.5f32.to_bits(), 444]);
        vm.call(&mut NoImports, 3, 64, false).unwrap();
        vm.finish_client_call(restore).unwrap();
        assert_eq!(&vm.image.globals[113..116], &[333, 7.5f32.to_bits(), 444]);

        vm.image.globals[110..113].copy_from_slice(&[777, 9.5f32.to_bits(), 888]);
        let restore = vm
            .prepare_client_call(ClientCallback::Disconnect, native, ThinkTime::Seconds(99.0))
            .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[native, 7.5f32.to_bits(), 444]);
        assert_eq!(vm.call(&mut NoImports, 6, 64, false), Err(Trap::Statement));
        vm.finish_client_call(restore).unwrap();
        assert_eq!(&vm.image.globals[34..37], &[333, 9.5f32.to_bits(), 888]);

        let restore = vm
            .prepare_touch_call(0, native, Some(ThinkTime::Seconds(12.0)))
            .unwrap();
        vm.image.globals[34] = 999;
        vm.image.globals[35] = 13.0f32.to_bits();
        vm.image.globals[36] = 1111;
        vm.finish_client_call(restore).unwrap();
        assert_eq!(&vm.image.globals[34..37], &[333, 13.0f32.to_bits(), 888]);
    });
}

#[test]
fn nq_spawn_uses_native_colors_and_keeps_qw_only_fields_cleared() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            number(vm, 0, b"health", 123.0);
            number(vm, native, b"health", 77.0);
            number(vm, native, b"gravity", 4.0);
            number(vm, native, b"maxspeed", 999.0);
            vm.prepare_client_spawn(services, context, native, b"\\name\\raw\x80", 0x2d)
                .unwrap();
            assert_eq!(read(vm, 0, b"health"), 123.0f32.to_bits());
            assert_eq!(read(vm, native, b"health"), 0);
            assert_eq!(read(vm, native, b"colormap"), 2.0f32.to_bits());
            assert_eq!(read(vm, native, b"team"), 14.0f32.to_bits());
            assert_eq!(read(vm, native, b"gravity"), 0);
            assert_eq!(read(vm, native, b"maxspeed"), 0);
            assert_eq!(
                vm.strings
                    .cstring(u64::from(read(vm, native, b"netname")))
                    .unwrap(),
                b"raw\x80"
            );
            assert_eq!(vm.client_phase(context, native), Ok(Motion::None));
            let player = &services.server.clients[id.0 as usize].player;
            assert_eq!(
                (player.movement_rules, player.trace_rules),
                (RuleSetId::Quake3, RuleSetId::Quake2)
            );
            assert_eq!(context.console.source, RuleSetId::Quake2Rerelease);
        },
    );
}

#[test]
fn nq_spawn_validates_executed_destinations_before_clearing_native_words() {
    let mut vm = vm(|image| {
        image
            .fielddefs
            .iter_mut()
            .find(|field| image.names.get(field.name) == Some(b"team".as_slice()))
            .unwrap()
            .value_type = 1;
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            number(vm, native, b"health", 77.0);
            assert_eq!(
                vm.prepare_client_spawn(services, context, native, b"\\name\\raw", 13),
                Err(CallError::Memory)
            );
            assert_eq!(read(vm, native, b"health"), 77.0f32.to_bits());
            assert!(vm.client_callback(ClientCallback::BeforeMove).is_ok());
        },
    );
}

#[test]
fn nq_ingress_ignores_fixangle_but_preserves_zero_impulse_and_body_pose() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            vm.prepare_client_spawn(services, context, native, b"", 0)
                .unwrap();
            number(vm, native, b"fixangle", 1.0);
            number(vm, native, b"impulse", 7.0);
            vector(vm, native, b"angles", Vec3([40.0, 50.0, 60.0]));
            vector(vm, native, b"velocity", Vec3([1.0, 2.0, 3.0]));
            vm.image.globals[37] = 0.125f32.to_bits();
            vm.receive_client_command(
                context,
                native,
                &UserCmd {
                    view_angles: Vec3([30.0, 90.0, 12.0]),
                    buttons: buttons::ATTACK | buttons::JUMP,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                read_vector(vm, native, b"v_angle"),
                Vec3([30.0, 90.0, 12.0])
            );
            assert_eq!(read(vm, native, b"button0"), 1.0f32.to_bits());
            assert_eq!(read(vm, native, b"button2"), 1.0f32.to_bits());
            assert_eq!(read(vm, native, b"impulse"), 7.0f32.to_bits());
            vm.receive_client_command(
                context,
                native,
                &UserCmd {
                    view_angles: Vec3([12.0, 34.0, 56.0]),
                    impulse: 9,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                read_vector(vm, native, b"v_angle"),
                Vec3([12.0, 34.0, 56.0])
            );
            assert_eq!(read(vm, native, b"button0"), 0);
            assert_eq!(read(vm, native, b"button2"), 0);
            assert_eq!(read(vm, native, b"impulse"), 9.0f32.to_bits());
            assert_eq!(read_vector(vm, native, b"angles"), Vec3([40.0, 50.0, 60.0]));
            assert_eq!(read_vector(vm, native, b"velocity"), Vec3([1.0, 2.0, 3.0]));
            assert_eq!(vm.image.globals[37], 0.125f32.to_bits());
            assert_eq!(
                vm.client_set_angles(native),
                Ok(Some(Vec3([40.0, 50.0, 60.0])))
            );
            vm.clear_client_set_angles(native).unwrap();
            assert_eq!(vm.client_set_angles(native), Ok(None));
            assert_eq!(
                vm.receive_client_command(context, 0, &UserCmd::default()),
                Err(CallError::Entity)
            );
            assert_eq!(
                vm.receive_client_command(context, 3 * vm.stride as u32, &UserCmd::default()),
                Err(CallError::Entity)
            );
        },
    );
}

#[test]
fn native_client_phase_uses_integer_motion_without_admitting_packet_movement() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            for (value, phase) in [
                (0.75, Motion::None),
                (8.75, Motion::Noclip),
                (3.0, Motion::Unsupported),
                (f32::NAN, Motion::Unsupported),
            ] {
                number(vm, native, b"movetype", value);
                assert_eq!(vm.client_phase(context, native), Ok(phase));
            }
            assert_eq!(
                vm.prepare_client_command(services, context, native, &UserCmd::default()),
                Err(CallError::Unsupported)
            );
            assert_eq!(
                vm.read_client(services, context, native),
                Err(CallError::Unsupported)
            );
            assert_eq!(
                vm.write_client(services, context, native, Vec3::default()),
                Err(CallError::Unsupported)
            );
        },
    );
    let mut missing = self::vm(|image| {
        image.fielddefs = image
            .fielddefs
            .iter()
            .copied()
            .filter(|field| image.names.get(field.name) != Some(b"movetype".as_slice()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
    });
    with_native_client(
        &mut missing,
        RuleSetId::Quake,
        |vm, _, context, native, _| {
            assert!(vm.client_callback(ClientCallback::BeforeMove).is_ok());
            assert_eq!(vm.client_phase(context, native), Err(CallError::Memory));
        },
    );
}

#[test]
fn native_client_velocity_uses_shared_sanitizer_without_bounds_or_angular_fields() {
    let mut vm = vm(|image| {
        image.fielddefs = image
            .fielddefs
            .iter()
            .copied()
            .filter(|field| {
                ![
                    b"angles".as_slice(),
                    b"avelocity",
                    b"mins",
                    b"maxs",
                    b"absmin",
                    b"absmax",
                ]
                .contains(&image.names.get(field.name).unwrap())
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            let maximum = services
                .cvars
                .bind(
                    "sv_maxvelocity",
                    Context {
                        source: RuleSetId::Quake,
                        ..Context::default()
                    },
                )
                .unwrap();
            services.cvars.force_write(maximum, "4").unwrap();
            let maximum = vm.client_max_velocity(services.cvars).unwrap();
            assert_eq!(maximum, 4.0);
            vector(vm, native, b"origin", Vec3([f32::NAN, f32::INFINITY, -2.0]));
            vector(vm, native, b"velocity", Vec3([f32::INFINITY, 3.0, -99.0]));
            let classname = vm.insert_string(b"raw\x80_player").unwrap() as u32;
            write(vm, native, b"classname", &[classname]);
            assert_eq!(vm.client_classname(native).unwrap(), b"raw\x80_player");
            vm.prepare_client_velocity(&mut services.server.entities, context, native)
                .unwrap();
            let entity = services.server.clients[id.0 as usize].entity;
            let checked = qa_session::physics::check_velocity(
                &mut services.server.entities,
                entity,
                maximum,
                |entities, stage, _invalid| vm.write_noclip(entities, context, native, stage),
            )
            .unwrap();
            assert_eq!((checked.invalid_velocity, checked.invalid_origin), (1, 3));
            assert_eq!(read_vector(vm, native, b"origin"), Vec3([0.0, 0.0, -2.0]));
            assert_eq!(read_vector(vm, native, b"velocity"), Vec3([0.0, 3.0, -4.0]));
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3([0.0, 0.0, -2.0])
            );
            assert!(services.server.area.bounds(entity).is_none());
            assert!(services.server.disconnect(id));
            assert_eq!(
                vm.prepare_client_velocity(&mut services.server.entities, context, native),
                Err(CallError::Entity)
            );
        },
    );
}

#[test]
fn native_client_velocity_reloads_shifted_origin_aliases_after_every_scalar_write() {
    let mut vm = vm(|image| {
        image
            .fielddefs
            .iter_mut()
            .find(|field| image.names.get(field.name) == Some(b"velocity".as_slice()))
            .unwrap()
            .offset = 1;
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            vector(vm, native, b"origin", Vec3([10.0, -8.0, 20.0]));
            vector(vm, native, b"velocity", Vec3([f32::NAN, 10.0, -10.0]));
            vm.prepare_client_velocity(&mut services.server.entities, context, native)
                .unwrap();
            let entity = services.server.clients[id.0 as usize].entity;
            let checked = qa_session::physics::check_velocity(
                &mut services.server.entities,
                entity,
                4.0,
                |entities, stage, _invalid| vm.write_noclip(entities, context, native, stage),
            )
            .unwrap();
            assert_eq!((checked.invalid_velocity, checked.invalid_origin), (1, 0));
            assert_eq!(read_vector(vm, native, b"origin"), Vec3([10.0, 0.0, 4.0]));
            assert_eq!(read_vector(vm, native, b"velocity"), Vec3([0.0, 4.0, -4.0]));
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3([10.0, 0.0, 4.0])
            );
            assert_eq!(
                services.server.entities.columns.velocity[entity.slot as usize],
                Vec3([0.0, 4.0, -4.0])
            );
        },
    );
}

#[test]
fn nq_control_returns_before_missing_punch_for_exact_none_only() {
    let mut vm = vm(|image| {
        image.fielddefs = image
            .fielddefs
            .iter()
            .copied()
            .filter(|field| {
                ![b"punchangle".as_slice(), b"health", b"fixangle", b"v_angle"]
                    .contains(&image.names.get(field.name).unwrap())
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            assert!(
                vm.prepare_control(services, context, native, 0.1)
                    .unwrap()
                    .is_none()
            );
            number(vm, native, b"movetype", 0.5);
            assert!(matches!(
                vm.prepare_control(services, context, native, 0.1),
                Err(CallError::Memory)
            ));
        },
    );
}

#[test]
fn nq_control_decays_punch_before_raw_fractional_health_and_uses_prior_roll() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            number(vm, native, b"movetype", 8.0);
            number(vm, native, b"health", 0.5);
            vector(vm, native, b"punchangle", Vec3([3.0, 4.0, 0.0]));
            vector(vm, native, b"v_angle", Vec3([12.0, 20.0, 1.0]));
            vector(vm, native, b"angles", Vec3::default());
            vector(vm, native, b"velocity", Vec3([0.0, 100.0, 0.0]));
            services.server.clients[id.0 as usize].player.health = 0;
            let options = vm
                .prepare_control(services, context, native, 0.13)
                .unwrap()
                .unwrap();
            let decayed = Vec3([0.6f32 * 3.7f32, 0.8f32 * 3.7f32, 0.0]);
            assert_eq!(
                read_vector(vm, native, b"punchangle").0.map(f32::to_bits),
                decayed.0.map(f32::to_bits)
            );
            let expected = Vec3([-(12.0 + decayed.0[0]) / 3.0, 20.0 + decayed.0[1], -4.0]);
            assert_eq!(
                read_vector(vm, native, b"angles").0.map(f32::to_bits),
                expected.0.map(f32::to_bits)
            );
            match options.phase {
                qa_movement::MovePhase::Control { body_angles, clock } => {
                    assert_eq!(body_angles, expected);
                    assert_eq!(clock, context.clock);
                }
                _ => panic!("native control phase required"),
            }
        },
    );
}

#[test]
fn nq_punch_decay_uses_native_double_provider_seconds_before_float_store() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            number(vm, native, b"movetype", 8.0);
            number(vm, native, b"health", 1.0);
            vector(vm, native, b"punchangle", Vec3([1.0, 0.0, 0.0]));
            vm.prepare_control(services, context, native, 0.0710135)
                .unwrap()
                .unwrap();
            assert_eq!(read(vm, native, b"punchangle"), 0x3e94_692f);
            assert_ne!(
                read(vm, native, b"punchangle"),
                (1.0f32 - 10.0f32 * 0.0710135f32).to_bits()
            );
        },
    );
}

#[test]
fn nq_control_rereads_health_after_overlapping_punch_decay() {
    let mut vm = vm(|image| {
        image
            .fielddefs
            .iter_mut()
            .find(|field| image.names.get(field.name) == Some(b"health".as_slice()))
            .unwrap()
            .offset = 42;
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            number(vm, native, b"movetype", 8.0);
            vector(vm, native, b"punchangle", Vec3([0.5, 0.0, 0.0]));
            vector(vm, native, b"angles", Vec3([11.0, 22.0, 33.0]));
            assert!(
                vm.prepare_control(services, context, native, 0.1)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(read(vm, native, b"health"), 0);
            assert_eq!(read_vector(vm, native, b"angles"), Vec3([11.0, 22.0, 33.0]));
        },
    );
}

#[test]
fn nq_control_captures_view_before_scalar_angles_and_rereads_fixangle_after_roll() {
    for fix_alias in [false, true] {
        let mut vm = vm(|image| {
            image
                .fielddefs
                .iter_mut()
                .find(|field| image.names.get(field.name) == Some(b"v_angle".as_slice()))
                .unwrap()
                .offset = 5;
            if fix_alias {
                image
                    .fielddefs
                    .iter_mut()
                    .find(|field| image.names.get(field.name) == Some(b"fixangle".as_slice()))
                    .unwrap()
                    .offset = 8;
            }
        });
        with_native_client(
            &mut vm,
            RuleSetId::Quake,
            |vm, services, context, native, _| {
                number(vm, native, b"movetype", 8.0);
                number(vm, native, b"health", 1.0);
                vector(vm, native, b"velocity", Vec3([0.0, 100.0, 0.0]));
                vector(vm, native, b"angles", Vec3([30.0, 60.0, 0.0]));
                vector(vm, native, b"v_angle", Vec3([12.0, 30.0, 60.0]));
                let prior = read_vector(vm, native, b"angles");
                let roll = qa_gameplay::rules::appearance::AliasRoll {
                    angle: services
                        .cvars
                        .native_number(
                            services.cvars.find("cl_rollangle").unwrap(),
                            context.native_rules,
                        )
                        .unwrap(),
                    speed: services
                        .cvars
                        .native_number(
                            services.cvars.find("cl_rollspeed").unwrap(),
                            context.native_rules,
                        )
                        .unwrap(),
                }
                .calculate(prior, read_vector(vm, native, b"velocity"))
                    * 4.0;
                vm.prepare_control(services, context, native, 0.1)
                    .unwrap()
                    .unwrap();
                let expected = if fix_alias {
                    Vec3([prior.0[0], prior.0[1], roll])
                } else {
                    Vec3([-4.0, 30.0, roll])
                };
                assert_eq!(
                    read_vector(vm, native, b"angles").0.map(f32::to_bits),
                    expected.0.map(f32::to_bits)
                );
            },
        );
    }
}

#[test]
fn nq_exact_noclip_admission_rejects_waterjump_and_fractional_alternatives() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            number(vm, native, b"health", 1.0);
            for (mode, flags, accepted) in [
                (8.0, 0.0, true),
                (8.5, 0.0, false),
                (8.0, 2048.0, false),
                (3.0, 0.0, false),
            ] {
                number(vm, native, b"movetype", mode);
                number(vm, native, b"flags", flags);
                assert_eq!(vm.client_noclip(context, native).is_ok(), accepted);
                assert_eq!(
                    vm.prepare_control(services, context, native, 0.1).is_ok(),
                    accepted
                );
            }
        },
    );
}

#[test]
fn nq_frame_pose_keeps_native_origin_absolute_time_and_captured_phase() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            number(vm, native, b"movetype", 8.0);
            vm.client_noclip(context, native).unwrap();
            number(vm, native, b"movetype", 0.0);
            number(vm, native, b"teleport_time", 10.25);
            vector(vm, native, b"origin", Vec3([100.0, 200.0, 300.0]));
            vector(vm, native, b"velocity", Vec3([3.0, -2.0, 6.0]));
            vector(vm, native, b"mins", Vec3([-7.0, -8.0, -9.0]));
            let entity = services.server.clients[id.0 as usize].entity;
            let player = &mut services.server.clients[id.0 as usize].player;
            let bounds = (player.body.mins, player.body.maxs);
            player.movement.water_jump_seconds = 7.0;
            vm.read_frame_client(&services.server.entities, context, native, entity, player)
                .unwrap();
            assert_eq!(player.body.position, Vec3([100.0, 200.0, 300.0]));
            assert_eq!(player.body.velocity, Vec3([3.0, -2.0, 6.0]));
            assert_eq!((player.body.mins, player.body.maxs), bounds);
            assert_eq!(player.movement.teleport_hold_until, 10.25);
            assert_eq!(player.movement.water_jump_seconds, 7.0);
            assert_eq!(player.movement.mode, MovementMode::Noclip);
            assert_eq!(
                (player.movement_rules, player.trace_rules),
                (RuleSetId::Quake3, RuleSetId::Quake2)
            );
        },
    );
}

#[test]
fn nq_frame_scalar_integration_rereads_shifted_native_aliases_without_table_mutation() {
    let mut vm = vm(|image| {
        image
            .fielddefs
            .iter_mut()
            .find(|field| image.names.get(field.name) == Some(b"velocity".as_slice()))
            .unwrap()
            .offset = 1;
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            vector(vm, native, b"origin", Vec3([10.0, 20.0, 30.0]));
            vector(vm, native, b"velocity", Vec3([2.0, 3.0, 4.0]));
            let entity = services.server.clients[id.0 as usize].entity;
            let player = &mut services.server.clients[id.0 as usize].player;
            player.movement_rules = RuleSetId::Quake;
            vm.read_frame_client(&services.server.entities, context, native, entity, player)
                .unwrap();
            let common_origin = services.server.entities.columns.position[entity.slot as usize];
            let result = qa_movement::pmove(
                UserCmd::default(),
                player,
                &mut NoQueries,
                qa_movement::MoveOptions {
                    phase: qa_movement::MovePhase::Integrate { seconds: 0.5 },
                    ..Default::default()
                },
                |player, stage| {
                    vm.write_frame_scalar(
                        &services.server.entities,
                        context,
                        native,
                        entity,
                        player,
                        stage,
                    )
                },
            )
            .unwrap();
            assert_eq!((result.steps, result.traces), (1, 0));
            assert_eq!(player.body.position, Vec3([11.0, 3.5, 5.0]));
            assert_eq!(player.body.velocity, Vec3([3.5, 5.0, 4.0]));
            assert_eq!(read_vector(vm, native, b"origin"), player.body.position);
            assert_eq!(read_vector(vm, native, b"velocity"), player.body.velocity);
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                common_origin
            );
            assert!(services.server.area.bounds(entity).is_none());
        },
    );
}

#[test]
fn nq_frame_scalar_rejects_reused_binding_before_any_native_write() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            vector(vm, native, b"origin", Vec3([1.0, 2.0, 3.0]));
            let entity = services.server.clients[id.0 as usize].entity;
            assert!(services.server.disconnect(id));
            let mut player = PlayerState::default();
            player.body.position = Vec3([99.0; 3]);
            assert_eq!(
                vm.write_frame_scalar(
                    &services.server.entities,
                    context,
                    native,
                    entity,
                    &mut player,
                    MotionWrite::Origin(0)
                ),
                Err(CallError::Entity)
            );
            assert_eq!(read_vector(vm, native, b"origin"), Vec3([1.0, 2.0, 3.0]));
        },
    );
}

#[test]
fn native_player_values_views_and_fractional_alive_state_use_one_client() {
    let mut vm = vm(|_| {});
    with_client(&mut vm, |vm, services, context, native, id| {
        vm.prepare_client_spawn(services, context, native, b"\\name\\player", 0)
            .unwrap();
        vector(vm, native, b"origin", Vec3([100.0, 200.0, 300.0]));
        vector(vm, native, b"velocity", Vec3([1.25, 2.5, 3.75]));
        vector(vm, native, b"mins", Vec3([-16.0, -16.0, -24.0]));
        vector(vm, native, b"maxs", Vec3([16.0, 16.0, 32.0]));
        vector(vm, native, b"v_angle", Vec3([12.0, 34.0, 56.0]));
        vector(vm, native, b"view_ofs", Vec3([0.0, 0.0, 22.0]));
        vector(vm, native, b"punchangle", Vec3([1.0, 2.0, 3.0]));
        for (name, value) in [
            (b"health".as_slice(), 0.5),
            (b"armorvalue", 50.0),
            (b"armortype", 0.6),
            (b"frags", 7.0),
            (b"currentammo", 13.0),
            (b"ammo_shells", 13.0),
            (b"ammo_nails", 21.0),
            (b"ammo_rockets", 4.0),
            (b"ammo_cells", 9.0),
            (b"items", (2 | 16384 | 131072 | 4194304) as f32),
            (b"weapon", 2.0),
            (b"weaponframe", 5.0),
            (b"waterlevel", 2.0),
            (b"watertype", -3.0),
            (b"teleport_time", 0.125),
            (b"solid", 3.0),
        ] {
            number(vm, native, name, value);
        }
        let range = services.storage.resources(context.module).unwrap()[0];
        let model =
            (ENGINE_CALLS.resource_index)(services, context.module, range, b"progs/v_shot2.mdl")
                .unwrap();
        let model_name = vm.insert_string(b"progs/v_shot2.mdl").unwrap() as u32;
        write(vm, native, b"weaponmodel", &[model_name]);
        vm.image.globals[38] = 2.0f32.to_bits();
        vm.image.globals[100] = 3.0f32.to_bits();
        let offset = vm.read_client(services, context, native).unwrap();
        let player = &services.server.clients[id.0 as usize].player;
        assert_eq!(offset, Vec3([-1.0, -1.0, 0.0]));
        assert_eq!(player.body.position, Vec3([99.0, 199.0, 300.0]));
        assert_eq!(player.health, 0);
        assert_eq!(player.movement.mode, MovementMode::Walk);
        assert_eq!(player.movement_rules, RuleSetId::Quake3);
        assert_eq!(player.trace_rules, RuleSetId::Quake2);
        assert_eq!(player.view_offset, Vec3([0.0, 0.0, 22.0]));
        assert_eq!(player.punch_angles, Vec3([1.0, 2.0, 3.0]));
        assert_eq!(
            player.view_weapon.model,
            ModelRef::native(ResourceNamespace::Module(context.module), model)
        );
        assert_eq!(player.view_weapon.frame, 5);
        assert_eq!(player.values.get(ValueId(5)).unwrap().as_integer(), 5);
        assert_eq!(
            player.inventory.weapon,
            services
                .registry
                .native_weapon(RuleSetId::QuakeWorld, 2)
                .unwrap()
        );
        assert_eq!(player.values.get(ValueId(0)).unwrap().as_integer(), 0);
        assert_eq!(
            player.values.get(ValueId(2)).unwrap().as_integer(),
            model as i32
        );
        assert_eq!(player.values.get(ValueId(11)).unwrap().as_integer(), 3);
        assert_eq!(
            player.values.get(ValueId(15)).unwrap().0,
            (2 | 16384 | 131072 | 4194304) | (2 << 28)
        );
        let shells = services
            .registry
            .native_item(RuleSetId::QuakeWorld, 11)
            .unwrap();
        assert_eq!(player.inventory.item_counts[shells.0 as usize], 13);
        let player = &mut services.server.clients[id.0 as usize].player;
        player.body.position = Vec3([109.0, 219.0, 330.0]);
        player.body.velocity = Vec3([8.0, 9.0, 10.0]);
        player.view_angles = Vec3([7.0, 8.0, 9.0]);
        player.movement.water_level = 1;
        player.movement.water_contents = Contents::SLIME.0;
        player.movement.water_jump_seconds = 0.0625;
        player.movement.grounded = true;
        player.movement.ground = Some(
            services
                .server
                .entities
                .find_native(
                    NativeEntity {
                        module: context.module,
                        slot: 0,
                    },
                    None,
                )
                .unwrap(),
        );
        vm.write_client(services, context, native, offset).unwrap();
        assert_eq!(read(vm, native, b"origin"), 110.0f32.to_bits());
        assert_eq!(read(vm, native, b"groundentity"), 0);
        assert_eq!(read(vm, native, b"flags"), 512.0f32.to_bits());
        assert_eq!(read(vm, native, b"watertype"), (-4.0f32).to_bits());
        assert_eq!(read(vm, native, b"teleport_time"), 0.0625f32.to_bits());
        assert!(
            services
                .server
                .area
                .bounds(services.server.clients[id.0 as usize].entity)
                .is_some()
        );
        vm.image.globals[38] = 3.0f32.to_bits();
        vm.project_entities(services, context).unwrap();
        assert_eq!(
            services.server.clients[id.0 as usize]
                .player
                .values
                .get(ValueId(15))
                .unwrap()
                .0,
            (2 | 16384 | 131072 | 4194304) | (3 << 28)
        );
    });
}

#[test]
fn command_fixangle_impulse_and_touch_globals_preserve_native_decisions() {
    let mut vm = vm(|_| {});
    with_client(&mut vm, |vm, services, context, native, _| {
        vm.prepare_client_spawn(services, context, native, b"", 0)
            .unwrap();
        number(vm, native, b"health", 100.0);
        number(vm, native, b"impulse", 7.0);
        let command = UserCmd {
            duration_ms: 50,
            view_angles: Vec3([30.0, 90.0, 0.0]),
            buttons: buttons::ATTACK | buttons::JUMP,
            ..UserCmd::default()
        };
        vm.prepare_client_command(services, context, native, &command)
            .unwrap();
        assert_eq!(read(vm, native, b"v_angle"), 30.0f32.to_bits());
        assert_eq!(read(vm, native, b"angles"), (-10.0f32).to_bits());
        assert_eq!(read(vm, native, b"button0"), 1.0f32.to_bits());
        assert_eq!(read(vm, native, b"button2"), 1.0f32.to_bits());
        assert_eq!(read(vm, native, b"impulse"), 7.0f32.to_bits());
        assert_eq!(vm.image.globals[37], 0.05f32.to_bits());
        number(vm, native, b"fixangle", 1.0);
        vector(vm, native, b"angles", Vec3([40.0, 50.0, 0.0]));
        vm.prepare_client_command(
            services,
            context,
            native,
            &UserCmd {
                duration_ms: 255,
                impulse: 9,
                ..UserCmd::default()
            },
        )
        .unwrap();
        assert_eq!(read(vm, native, b"v_angle"), 30.0f32.to_bits());
        assert_eq!(read(vm, native, b"angles"), 40.0f32.to_bits());
        assert_eq!(read(vm, native, b"impulse"), 9.0f32.to_bits());
        assert_eq!(vm.image.globals[37], 0.1f32.to_bits());
        assert_eq!(
            vm.client_set_angles(native),
            Ok(Some(Vec3([40.0, 50.0, 0.0])))
        );
        assert_eq!(read(vm, native, b"fixangle"), 1.0f32.to_bits());
        vm.clear_client_set_angles(native).unwrap();
        assert_eq!(vm.client_set_angles(native), Ok(None));
    });
}

#[test]
fn command_shifted_vectors_reread_native_components_after_each_store() {
    let mut vm = vm(|image| {
        let definition = image
            .fielddefs
            .iter_mut()
            .find(|field| image.names.get(field.name) == Some(&b"v_angle"[..]))
            .unwrap();
        definition.offset = 5;
    });
    with_client(&mut vm, |vm, services, context, native, _| {
        vm.prepare_client_spawn(services, context, native, b"", 0)
            .unwrap();
        number(vm, native, b"health", 100.0);
        vector(vm, native, b"velocity", Vec3([4.0, 5.0, 6.0]));
        vector(vm, native, b"angles", Vec3([1.0, 2.0, 3.0]));
        let command = UserCmd {
            duration_ms: 50,
            view_angles: Vec3([30.0, 90.0, 0.0]),
            ..Default::default()
        };
        vm.prepare_client_command(services, context, native, &command)
            .unwrap();
        // VectorCopy writes v_angle at words 5..7. Pitch writes word 6,
        // which is then the live source for the scalar yaw store at word 7.
        let angles = Vec3([-10.0, -10.0, 3.0]);
        let velocity = Vec3([4.0, 5.0, 30.0]);
        let roll = qa_gameplay::rules::appearance::AliasRoll {
            angle: services
                .cvars
                .native_number(
                    services.cvars.find("cl_rollangle").unwrap(),
                    context.native_rules,
                )
                .unwrap(),
            speed: services
                .cvars
                .native_number(
                    services.cvars.find("cl_rollspeed").unwrap(),
                    context.native_rules,
                )
                .unwrap(),
        }
        .calculate(angles, velocity)
            * 4.0;
        assert_eq!(
            read_vector(vm, native, b"angles").0.map(f32::to_bits),
            [(-10.0f32).to_bits(), (-10.0f32).to_bits(), roll.to_bits()]
        );
        assert_eq!(read(vm, native, b"v_angle"), 30.0f32.to_bits());
        assert_eq!(read_vector(vm, native, b"velocity"), velocity);
    });
}

#[test]
fn command_button_and_impulse_aliases_change_later_native_decisions() {
    let mut vm = vm(|image| {
        for (name, offset) in [
            (b"health".as_slice(), 52),
            (b"fixangle".as_slice(), 53),
            (b"velocity".as_slice(), 54),
        ] {
            image
                .fielddefs
                .iter_mut()
                .find(|field| image.names.get(field.name) == Some(name))
                .unwrap()
                .offset = offset;
        }
    });
    with_client(&mut vm, |vm, services, context, native, _| {
        vm.prepare_client_spawn(services, context, native, b"", 0)
            .unwrap();
        number(vm, native, b"health", 0.0);
        number(vm, native, b"fixangle", 1.0);
        vector(vm, native, b"v_angle", Vec3([12.0, 34.0, 56.0]));
        vector(vm, native, b"angles", Vec3([40.0, 50.0, 60.0]));
        vector(vm, native, b"velocity", Vec3([1.0, 2.0, 3.0]));
        vm.prepare_client_command(
            services,
            context,
            native,
            &UserCmd {
                duration_ms: 50,
                view_angles: Vec3([30.0, 90.0, 0.0]),
                buttons: buttons::ATTACK,
                impulse: 9,
                ..Default::default()
            },
        )
        .unwrap();
        // The initial fixangle skips view copying. Attack then makes health
        // positive; jump clears fixangle; impulse becomes the live velocity x.
        let roll = qa_gameplay::rules::appearance::AliasRoll {
            angle: services
                .cvars
                .native_number(
                    services.cvars.find("cl_rollangle").unwrap(),
                    context.native_rules,
                )
                .unwrap(),
            speed: services
                .cvars
                .native_number(
                    services.cvars.find("cl_rollspeed").unwrap(),
                    context.native_rules,
                )
                .unwrap(),
        }
        .calculate(Vec3([-4.0, 34.0, 60.0]), Vec3([9.0, 2.0, 3.0]))
            * 4.0;
        assert_eq!(
            read_vector(vm, native, b"angles").0.map(f32::to_bits),
            [(-4.0f32).to_bits(), 34.0f32.to_bits(), roll.to_bits()]
        );
        assert_eq!(read(vm, native, b"v_angle"), 12.0f32.to_bits());
        assert_eq!(read(vm, native, b"health"), 1.0f32.to_bits());
        assert_eq!(read(vm, native, b"fixangle"), 0);
        assert_eq!(read(vm, native, b"velocity"), 9.0f32.to_bits());
    });
}

#[test]
fn missing_client_abi_is_only_rejected_when_selected_for_either_qc_source() {
    let mut missing = vm(|image| {
        image
            .globaldefs
            .iter_mut()
            .find(|field| image.names.get(field.name) == Some(&b"ClientConnect"[..]))
            .unwrap()
            .value_type = 2
    });
    with_client(&mut missing, |vm, _, _, _, _| {
        assert_eq!(
            vm.client_callback(ClientCallback::Connect),
            Err(CallError::Memory)
        );
        assert!(vm.client_callback(ClientCallback::Begin).is_ok());
    });
    let mut nq = vm(|_| {});
    let mut entities = qa_world::entities::EntityTable::new(80, 2).unwrap();
    let anchor = entities
        .allocate(
            ThinkTime::Seconds(1.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    entities.columns.native_entity[anchor.slot as usize] = Some(NativeEntity {
        module: ModuleId(4),
        slot: 0,
    });
    nq.seed_entities(&entities, ModuleId(4), RuleSetId::Quake, 1)
        .unwrap();
    bind_callbacks(&mut nq);
    assert_eq!(nq.client_count(), Ok(1));
    assert_eq!(
        nq.client_callback(ClientCallback::Connect),
        Ok(CallbackId(4))
    );
    assert_eq!(nq.read_spawn_parms(), Ok([0; 16]));
}

#[test]
fn absent_native_player_fields_preserve_common_and_raw_state() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|image| {
            let absent = [
                b"health".as_slice(),
                b"armorvalue",
                b"frags",
                b"currentammo",
                b"ammo_shells",
                b"ammo_nails",
                b"ammo_rockets",
                b"ammo_cells",
                b"items",
                b"weapon",
                b"weaponmodel",
                b"weaponframe",
            ];
            image.fielddefs = image
                .fielddefs
                .iter()
                .copied()
                .filter(|definition| {
                    !image
                        .names
                        .get(definition.name)
                        .is_some_and(|name| absent.contains(&name))
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            image.globaldefs = image
                .globaldefs
                .iter()
                .copied()
                .filter(|definition| {
                    !image.names.get(definition.name).is_some_and(|name| {
                        [
                            b"total_secrets".as_slice(),
                            b"total_monsters",
                            b"found_secrets",
                            b"killed_monsters",
                        ]
                        .contains(&name)
                    })
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
        });
        with_native_client(&mut vm, source, |vm, services, context, _, id| {
            let player = &mut services.server.clients[id.0 as usize].player;
            player.health = 71;
            player.armor = 33;
            player.frags = 9;
            player.score = 9;
            player.view_weapon.model =
                ModelRef::native(ResourceNamespace::Module(ModuleId(99)), 12);
            player.view_weapon.frame = 7;
            player.inventory.weapon = WeaponId(3);
            player.inventory.collectibles = 5;
            player.inventory.item_counts[1] = 6;
            for index in 0..33 {
                assert!(
                    ValueBinding {
                        id: ValueId(index),
                        width: ValueWidth::Signed32,
                        reset: ValueReset::Life
                    }
                    .import(&mut player.values, 700 + index)
                );
            }
            vm.project_entities(services, context).unwrap();
            let player = &services.server.clients[id.0 as usize].player;
            assert_eq!(
                (player.health, player.armor, player.frags, player.score),
                (71, 33, 9, 9)
            );
            assert_eq!(
                player.view_weapon.model,
                ModelRef::native(ResourceNamespace::Module(ModuleId(99)), 12)
            );
            assert_eq!(player.view_weapon.frame, 7);
            assert_eq!(player.inventory.weapon, WeaponId(3));
            assert_eq!(player.inventory.collectibles, 5);
            assert_eq!(player.inventory.item_counts[1], 6);
            for index in 0..33 {
                assert_eq!(
                    player.values.get(ValueId(index)).unwrap().as_integer(),
                    700 + index as i32
                );
            }
        });
    }
}

#[test]
fn nq_published_players_use_raw_clientdata_items_without_reusing_stat_fifteen() {
    let mut vm = vm(|image| {
        image.fielddefs = image
            .fielddefs
            .iter()
            .copied()
            .filter(|definition| image.names.get(definition.name) != Some(b"items2".as_slice()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            assert_eq!(vm.client_count(), Ok(2));
            assert_eq!(
                vm.client_callback(ClientCallback::Connect),
                Ok(CallbackId(4))
            );
            assert_eq!(
                vm.read_client(services, context, native),
                Err(CallError::Unsupported)
            );
            assert_eq!(
                vm.prepare_client_command(services, context, native, &UserCmd::default()),
                Err(CallError::Unsupported)
            );
            let old_body = services.server.clients[id.0 as usize].player.body;
            let old_mode = services.server.clients[id.0 as usize].player.movement.mode;
            let stat15 = 0x8123_4567;
            assert!(
                ValueBinding {
                    id: ValueId(15),
                    width: ValueWidth::Signed32,
                    reset: ValueReset::Life
                }
                .import(
                    &mut services.server.clients[id.0 as usize].player.values,
                    stat15
                )
            );
            for (name, value) in [
                (b"health".as_slice(), 0.5),
                (b"armorvalue", 50.0),
                (b"armortype", 0.6),
                (b"frags", 7.0),
                (b"currentammo", 13.0),
                (b"ammo_shells", 13.0),
                (b"ammo_nails", 21.0),
                (b"ammo_rockets", 4.0),
                (b"ammo_cells", 9.0),
                (b"items", (2 | 16384 | 131072 | 4194304) as f32),
                (b"weapon", 2.0),
                (b"weaponframe", 5.0),
            ] {
                number(vm, native, name, value);
            }
            vector(vm, native, b"view_ofs", Vec3([0.0, 0.0, 22.0]));
            vector(vm, native, b"punchangle", Vec3([1.0, 2.0, 3.0]));
            let range = services.storage.resources(context.module).unwrap()[0];
            let model = (ENGINE_CALLS.resource_index)(
                services,
                context.module,
                range,
                b"progs/v_shot2.mdl",
            )
            .unwrap();
            let model_name = vm.insert_string(b"progs/v_shot2.mdl").unwrap() as u32;
            write(vm, native, b"weaponmodel", &[model_name]);
            vm.image.globals[38] = 2.0f32.to_bits();
            vm.image.globals[100] = 3.0f32.to_bits();
            vm.project_entities(services, context).unwrap();
            assert_eq!(read(vm, native, b"health"), 0.5f32.to_bits());
            let player = &services.server.clients[id.0 as usize].player;
            assert_eq!(player.health, 0);
            assert_eq!(player.armor, 50);
            assert_eq!((player.frags, player.score), (7, 7));
            assert_eq!(player.values.get(ValueId(0)).unwrap().as_integer(), 0);
            assert_eq!(player.values.get(ValueId(3)).unwrap().as_integer(), 13);
            assert_eq!(player.values.get(ValueId(11)).unwrap().as_integer(), 3);
            assert_eq!(player.values.get(ValueId(15)).unwrap().0, stat15);
            assert_eq!(
                player.values.get(ValueId(32)).unwrap().0,
                (2 | 16384 | 131072 | 4194304) | (2 << 28)
            );
            assert_eq!(player.inventory.collectibles, 2);
            assert_eq!(
                player.inventory.weapon,
                services
                    .registry
                    .native_weapon(RuleSetId::Quake, 2)
                    .unwrap()
            );
            for (ordinal, count) in [(11, 13), (12, 21), (13, 4), (14, 9)] {
                let item = services
                    .registry
                    .native_item(RuleSetId::Quake, ordinal)
                    .unwrap();
                assert_eq!(player.inventory.item_counts[item.0 as usize], count);
            }
            let armor = services.registry.native_item(RuleSetId::Quake, 3).unwrap();
            assert_eq!(player.armor_type, armor);
            assert_eq!(
                player.view_weapon.model,
                ModelRef::native(ResourceNamespace::Module(context.module), model)
            );
            assert_eq!(player.view_weapon.frame, 5);
            assert_eq!(player.view_offset, Vec3([0.0, 0.0, 22.0]));
            assert_eq!(player.punch_angles, Vec3([1.0, 2.0, 3.0]));
            assert_eq!(player.body.position, old_body.position);
            assert_eq!(player.body.velocity, old_body.velocity);
            assert_eq!(player.body.mins, old_body.mins);
            assert_eq!(player.body.maxs, old_body.maxs);
            assert_eq!(player.movement.mode, old_mode);
            assert_eq!(
                (player.movement_rules, player.trace_rules),
                (RuleSetId::Quake3, RuleSetId::Quake2)
            );
        },
    );
}

#[test]
fn qc_items2_is_only_selected_for_nq_clientdata_and_replaces_serverflag_bits() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|_| {});
        with_native_client(&mut vm, source, |vm, services, context, native, id| {
            number(vm, native, b"items", 2.0);
            number(
                vm,
                native,
                b"items2",
                if source == RuleSetId::Quake {
                    64.0
                } else {
                    f32::NAN
                },
            );
            vm.image.globals[38] = 3.0f32.to_bits();
            vm.project_entities(services, context).unwrap();
            let player = &services.server.clients[id.0 as usize].player;
            let (raw, at) = if source == RuleSetId::Quake {
                (2 | (64 << 23), ValueId(32))
            } else {
                (2 | (3 << 28), ValueId(15))
            };
            assert_eq!(player.values.get(at).unwrap().0, raw);
            assert_eq!(
                player.inventory.collectibles,
                if source == RuleSetId::Quake { 2 } else { 3 }
            );
            assert_eq!(
                player.inventory.item_counts
                    [services.registry.native_item(source, 11).unwrap().0 as usize],
                0
            );
        });
    }
}

#[test]
fn callback_native_function_width_is_not_limited_to_sixteen_bits() {
    let mut vm = vm(|image| {
        let function = image.functions[1].clone();
        let mut functions = image.functions.to_vec();
        functions.resize(65537, function);
        image.functions = functions.into_boxed_slice();
        image.globals[71] = 65536;
    });
    let entities = qa_world::entities::EntityTable::new(80, 2).unwrap();
    vm.seed_entities(&entities, ModuleId(4), RuleSetId::QuakeWorld, 1)
        .unwrap();
    let entries = [65536];
    let table = FunctionTable::<Server>::load([(
        ModuleId(4),
        RuleSetId::QuakeWorld,
        vec![FunctionBinding {
            entry: 65536,
            call: |_, _, _, _| true,
        }],
    )])
    .unwrap();
    vm.bind_thinks(table.module_binding(ModuleId(4)), &entries);
    assert_eq!(
        vm.client_callback(ClientCallback::Connect),
        Ok(CallbackId(0))
    );
}
