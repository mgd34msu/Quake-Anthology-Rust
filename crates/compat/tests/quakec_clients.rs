#[path = "support/quakec_program.rs"]
#[allow(dead_code)]
mod quakec_program;

use qa_compat::{
    quakec::{Layout, QW_USERINFO_BYTES, Vm},
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices, ServiceStorage},
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{names::NameTable, primitives::*};
use qa_formats::program::quakec::{Image, Opcode};
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
    let mut names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
    let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
    let mut server = Server::load(
        2,
        80,
        registry.items.len() + 1,
        32,
        registry.weapons.len() + 1,
        32,
    )
    .unwrap();
    let context = CallContext {
        module: ModuleId(4),
        clock: ThinkTime::Seconds(10.0),
        server_frame: 1,
        native_rules: RuleSetId::QuakeWorld,
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
    vm.seed_entities(&server.entities, context.module, context.native_rules, 2)
        .unwrap();
    bind_callbacks(vm);
    let native_identity = NativeEntity {
        module: context.module,
        slot: 17,
    };
    let client = server
        .connect(Connection::Local, context.module, Some(native_identity))
        .unwrap();
    let entity = server.clients[client.0 as usize].entity;
    assert_ne!(entity.slot, 17);
    vm.bind_client(&server.entities, native_identity, entity)
        .unwrap();
    let player = &mut server.clients[client.0 as usize].player;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake2;
    qa_movement::set_bounds(player);
    let native = 17 * vm.stride as u32;
    let mut cvars = Cvars::new().unwrap();
    vm.bind_client_settings(&cvars, &registry).unwrap();
    let values: Vec<_> = (0..32)
        .map(|id| ValueBinding {
            id: ValueId(id),
            width: ValueWidth::Signed32,
            reset: ValueReset::Life,
        })
        .collect();
    vm.bind_player_values(&values).unwrap();
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
        client_commands: None,
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
        vm.prepare_client_call(
            ClientCallback::NewParameters,
            native,
            ThinkTime::Seconds(15.0),
        )
        .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[111, 9.25f32.to_bits(), 222]);
        for phase in [
            ClientCallback::Connect,
            ClientCallback::Begin,
            ClientCallback::BeforeMove,
            ClientCallback::AfterMove,
        ] {
            vm.prepare_client_call(phase, native, ThinkTime::Seconds(15.0))
                .unwrap();
            assert_eq!(&vm.image.globals[34..37], &[native, 15.0f32.to_bits(), 222]);
            vm.image.globals[34] = 777;
            vm.image.globals[35] = 7.0f32.to_bits();
        }
        vm.prepare_client_call(ClientCallback::Disconnect, native, ThinkTime::Seconds(99.0))
            .unwrap();
        assert_eq!(&vm.image.globals[34..37], &[native, 7.0f32.to_bits(), 222]);
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
        vm.prepare_touch_call(0, native, None).unwrap();
        assert_eq!(&vm.image.globals[34..37], &[0, 7.0f32.to_bits(), native]);
        vm.prepare_touch_call(0, native, Some(ThinkTime::Seconds(20.0)))
            .unwrap();
        assert_eq!(vm.image.globals[35], 20.0f32.to_bits());
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
        vm.prepare_client_spawn(services, context, native, b"\\name\\raw\xff")
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
            vm.prepare_client_spawn(services, context, native, &[b'a'; QW_USERINFO_BYTES]),
            Err(CallError::Memory)
        );
    });
}

#[test]
fn native_player_values_views_and_fractional_alive_state_use_one_client() {
    let mut vm = vm(|_| {});
    with_client(&mut vm, |vm, services, context, native, id| {
        vm.prepare_client_spawn(services, context, native, b"\\name\\player")
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
        vm.prepare_client_spawn(services, context, native, b"")
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
        vm.prepare_client_spawn(services, context, native, b"")
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
        let field = vm.image.field(b"angles").unwrap();
        assert_eq!(
            vm.entities
                .read_vec3(vm.field_address(native, field as u32, 3).unwrap())
                .unwrap()
                .0
                .map(f32::to_bits),
            [(-10.0f32).to_bits(), (-10.0f32).to_bits(), roll.to_bits()]
        );
        assert_eq!(read(vm, native, b"v_angle"), 30.0f32.to_bits());
        assert_eq!(
            vm.entities
                .read_vec3(vm.field_address(native, 3, 3).unwrap())
                .unwrap(),
            velocity
        );
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
        vm.prepare_client_spawn(services, context, native, b"")
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
        let field = vm.image.field(b"angles").unwrap();
        assert_eq!(
            vm.entities
                .read_vec3(vm.field_address(native, field as u32, 3).unwrap())
                .unwrap()
                .0
                .map(f32::to_bits),
            [(-4.0f32).to_bits(), 34.0f32.to_bits(), roll.to_bits()]
        );
        assert_eq!(read(vm, native, b"v_angle"), 12.0f32.to_bits());
        assert_eq!(read(vm, native, b"health"), 1.0f32.to_bits());
        assert_eq!(read(vm, native, b"fixangle"), 0);
        assert_eq!(read(vm, native, b"velocity"), 9.0f32.to_bits());
    });
}

#[test]
fn missing_client_abi_is_only_rejected_when_selected_and_nq_is_explicit() {
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
    assert_eq!(nq.client_count(), Err(CallError::Unsupported));
    assert_eq!(
        nq.client_callback(ClientCallback::Connect),
        Err(CallError::Unsupported)
    );
    assert_eq!(nq.read_spawn_parms(), Err(CallError::Unsupported));
}

#[test]
fn absent_native_player_fields_preserve_common_and_raw_state() {
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
    with_client(&mut vm, |vm, services, context, _, id| {
        let player = &mut services.server.clients[id.0 as usize].player;
        player.health = 71;
        player.armor = 33;
        player.frags = 9;
        player.score = 9;
        player.view_weapon.model = ModelRef::native(ResourceNamespace::Module(ModuleId(99)), 12);
        player.view_weapon.frame = 7;
        player.inventory.weapon = WeaponId(3);
        player.inventory.collectibles = 5;
        player.inventory.item_counts[1] = 6;
        for index in 0..32 {
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
        for index in 0..32 {
            assert_eq!(
                player.values.get(ValueId(index)).unwrap().as_integer(),
                700 + index as i32
            );
        }
    });
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
