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
    area::{LinkFlags, LinkOrder},
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
            random_seed: 1,
        },
    )
    .unwrap()
}
fn client_vectors(mutate: impl FnOnce(&mut Image)) -> Vm {
    vm(|image| {
        image.names.reserve(2, 32).unwrap();
        let mut fields = image.fielddefs.to_vec();
        for (offset, name) in [(72, b"oldorigin".as_slice()), (75, b"movedir".as_slice())] {
            fields.push(qa_formats::program::quakec::Definition {
                value_type: 3,
                offset,
                name: image.names.intern(name).unwrap(),
            });
        }
        image.fielddefs = fields.into_boxed_slice();
        image.entityfields = 78;
        mutate(image);
    })
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
    fn entity_access(&self) -> qa_compat::quakec::EntityAccess<'_> {
        qa_compat::quakec::EntityAccess::native_only()
    }
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
    let mut vfs = qa_content::vfs::Vfs::default();
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
        vfs: &mut vfs,
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
        native_geometry: None,
        foreign_trace: None,
        visibility: None,
        portals: None,
        scratch: &mut scratch,
    };
    run(vm, &mut services, context, native, client);
}

#[test]
fn retained_client_body_keeps_native_bytes_and_area_order_until_exact_reconnect() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|_| {});
        with_native_client(&mut vm, source, |vm, services, context, native, client| {
            let identity = NativeEntity {
                module: context.module,
                slot: vm.entity_number(native).unwrap() as i32,
            };
            let old = services.server.clients[client.0 as usize].entity;
            let other_native = vm.stride as u32;
            let other = vm
                .ensure_client_body(services, context, other_native)
                .unwrap();
            number(vm, native, b"solid", 2.0);
            vector(vm, native, b"origin", Vec3([10.0, 0.0, 0.0]));
            vector(vm, native, b"mins", Vec3([-1.0; 3]));
            vector(vm, native, b"maxs", Vec3([1.0; 3]));
            (ENGINE_CALLS.link)(services, context, other, LinkFlags::SOLID).unwrap();
            vm.relink_entity(services, context, native).unwrap();
            let sibling = (ENGINE_CALLS.spawn)(services, context).unwrap();
            (ENGINE_CALLS.link)(services, context, sibling, LinkFlags::SOLID).unwrap();
            let query = Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            };
            let before = services
                .server
                .area
                .query(&services.server.entities, query, LinkFlags::SOLID)
                .map(|row| row.id)
                .collect::<Vec<_>>();
            let bounds = services.server.area.bounds(old).unwrap();
            let relinks = services.server.area.relinks;
            let bytes = vm
                .entities
                .read(u64::from(native), vm.stride)
                .unwrap()
                .to_vec();
            // Releasing a borrowed connection body leaves its binding intact.
            vm.release_client_body(services, context, native).unwrap();
            assert_eq!(
                vm.bound_entity(&services.server.entities, identity),
                Ok(Some(old))
            );
            let retained = vm
                .retain_client_body(services, context, native, old)
                .unwrap();
            assert_ne!(retained, old);
            assert_eq!(
                vm.bound_entity(&services.server.entities, identity),
                Ok(Some(retained))
            );
            assert_eq!(services.server.area.bounds(old), None);
            assert_eq!(services.server.area.bounds(retained), Some(bounds));
            assert_eq!(services.server.area.relinks, relinks);
            assert_eq!(
                services
                    .server
                    .area
                    .query(&services.server.entities, query, LinkFlags::SOLID)
                    .map(|row| row.id)
                    .collect::<Vec<_>>(),
                before
                    .into_iter()
                    .map(|id| if id == old { retained } else { id })
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                bytes
            );
            assert!(services.server.disconnect(client));
            assert!(services.server.entities.resolve(old).is_none());
            assert!(services.server.entities.resolve(retained).is_some());
            assert_eq!(services.server.area.bounds(retained), Some(bounds));
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                bytes
            );

            // A new connection can already carry this namespace before the
            // old ordinary body is reclaimed. Only that exact body retires.
            let reconnect = services
                .server
                .connect(Connection::Bot, context.module, Some(identity))
                .unwrap();
            vm.release_client_body(services, context, native).unwrap();
            assert!(services.server.entities.resolve(retained).is_none());
            assert!(services.server.entities.resolve(other).is_some());
            assert!(services.server.entities.resolve(sibling).is_some());
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                bytes
            );
            assert!(services.server.bind_client_native(reconnect, identity));
            let current = services.server.clients[reconnect.0 as usize].entity;
            vm.bind_client(&services.server.entities, identity, current)
                .unwrap();
            assert_eq!(
                vm.ensure_client_body(services, context, native),
                Ok(current)
            );
            let duplicate = services
                .server
                .connect(Connection::Bot, context.module, None)
                .unwrap();
            vm.release_client_body(services, context, native).unwrap();
            assert!(!services.server.bind_client_native(duplicate, identity));
            assert_eq!(
                vm.bound_entity(&services.server.entities, identity),
                Ok(Some(current))
            );
            assert!(services.server.entities.resolve(current).is_some());
        });
    }
}

#[test]
fn retaining_client_body_restores_borrowed_binding_on_projection_or_capacity_failure() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|_| {});
        with_native_client(&mut vm, source, |vm, services, context, native, client| {
            let identity = NativeEntity {
                module: context.module,
                slot: vm.entity_number(native).unwrap() as i32,
            };
            let old = services.server.clients[client.0 as usize].entity;
            number(vm, native, b"solid", 2.0);
            vector(vm, native, b"mins", Vec3([-1.0; 3]));
            vector(vm, native, b"maxs", Vec3([1.0; 3]));
            vm.relink_entity(services, context, native).unwrap();
            let bounds = services.server.area.bounds(old);
            let relinks = services.server.area.relinks;
            let before = services.server.entities.active().collect::<Vec<_>>();
            write(vm, native, b"classname", &[u32::MAX]);
            let bytes = vm
                .entities
                .read(u64::from(native), vm.stride)
                .unwrap()
                .to_vec();
            assert_eq!(
                vm.retain_client_body(services, context, native, old),
                Err(CallError::Memory)
            );
            assert_eq!(
                services.server.entities.active().collect::<Vec<_>>(),
                before
            );
            assert_eq!(
                vm.bound_entity(&services.server.entities, identity),
                Ok(Some(old))
            );
            assert_eq!(services.server.area.bounds(old), bounds);
            assert_eq!(services.server.area.relinks, relinks);
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                bytes
            );
            write(vm, native, b"classname", &[0]);
            while services
                .server
                .entities
                .allocate(context.clock, context.module, AllocationPolicy::EDICT)
                .is_some()
            {}
            let full = services.server.entities.active().collect::<Vec<_>>();
            let mut overwrite = context;
            overwrite.allocation.full = qa_world::entities::FullTable::OverwriteLastOwned;
            assert_eq!(
                vm.retain_client_body(services, overwrite, native, old),
                Err(CallError::Capacity)
            );
            assert_eq!(services.server.entities.active().collect::<Vec<_>>(), full);
            assert_eq!(
                vm.bound_entity(&services.server.entities, identity),
                Ok(Some(old))
            );
            assert_eq!(services.server.area.bounds(old), bounds);
            assert_eq!(services.server.area.relinks, relinks);
        });
    }
}

#[test]
fn native_client_spawn_and_fixangle_clear_publish_dirty_aliases_without_relinking() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|image| {
            for field in image.fielddefs.iter_mut() {
                if image.names.get(field.name) == Some(b"fixangle".as_slice()) {
                    field.offset = 0;
                }
            }
        });
        with_native_client(&mut vm, source, |vm, services, context, native, client| {
            let entity = services.server.clients[client.0 as usize].entity;
            vector(vm, native, b"origin", Vec3([9.0, 8.0, 7.0]));
            number(vm, native, b"frame", 13.0);
            vm.relink_entity(services, context, native).unwrap();
            vm.project_entities(services, context).unwrap();
            let bounds = services.server.area.bounds(entity);
            let relinks = services.server.area.relinks;
            vm.entities.write(u64::from(native), &[0xa5; 16]).unwrap();
            vm.prepare_client_spawn(services, context, native, b"", 0)
                .unwrap();
            assert_eq!(
                vm.entities.read(u64::from(native), 16).unwrap(),
                &[0xa5; 16]
            );
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3([9.0, 8.0, 7.0])
            );
            vm.project_dirty_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3::default()
            );
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                0
            );
            assert_eq!(services.server.area.bounds(entity), bounds);
            assert_eq!(services.server.area.relinks, relinks);

            vector(vm, native, b"origin", Vec3([77.0, 2.0, 3.0]));
            vm.project_entities(services, context).unwrap();
            vm.clear_client_set_angles(native).unwrap();
            assert_eq!(read_vector(vm, native, b"origin"), Vec3([0.0, 2.0, 3.0]));
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3([77.0, 2.0, 3.0])
            );
            vm.project_dirty_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3([0.0, 2.0, 3.0])
            );
            assert_eq!(services.server.area.bounds(entity), bounds);
            assert_eq!(services.server.area.relinks, relinks);
        });
    }
}

#[test]
fn native_client_command_retains_successful_dirty_prefix_when_later_button_field_is_absent() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|image| {
            for field in image.fielddefs.iter_mut() {
                match image.names.get(field.name).unwrap() {
                    b"v_angle" => field.offset = 0,
                    b"button0" => field.offset = 62,
                    b"button2" => field.value_type = 1,
                    _ => {}
                }
            }
        });
        with_native_client(&mut vm, source, |vm, services, context, native, client| {
            vm.project_entities(services, context).unwrap();
            let entity = services.server.clients[client.0 as usize].entity;
            let command = UserCmd {
                view_angles: Vec3([11.0, 22.0, 33.0]),
                buttons: buttons::ATTACK,
                ..UserCmd::default()
            };
            assert_eq!(
                vm.receive_client_command(context, native, &command),
                Err(CallError::Memory)
            );
            assert_eq!(read_vector(vm, native, b"origin"), command.view_angles);
            assert_eq!(read(vm, native, b"frame"), 1.0f32.to_bits());
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3::default()
            );
            vm.project_dirty_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                command.view_angles
            );
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                1
            );
            assert_eq!(services.server.area.bounds(entity), None);
        });
    }
}

#[test]
fn control_and_command_angles_publish_origin_aliases_for_admitted_native_modes() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for rejected in [false, true] {
            if source == RuleSetId::QuakeWorld && rejected {
                continue;
            }
            let mut vm = vm(|image| {
                for field in image.fielddefs.iter_mut() {
                    if image.names.get(field.name) == Some(b"angles".as_slice()) {
                        field.offset = 0;
                    }
                }
            });
            with_native_client(&mut vm, source, |vm, services, context, native, client| {
                let entity = services.server.clients[client.0 as usize].entity;
                number(vm, native, b"health", 100.0);
                number(vm, native, b"movetype", if rejected { 3.0 } else { 8.0 });
                vector(vm, native, b"v_angle", Vec3([12.0, 34.0, 56.0]));
                vm.project_entities(services, context).unwrap();
                if source == RuleSetId::Quake {
                    let result = vm.prepare_control(services, context, native, 0.1);
                    assert!(result.unwrap().is_some());
                } else {
                    vm.prepare_client_command(
                        services,
                        context,
                        native,
                        &UserCmd {
                            view_angles: Vec3([12.0, 34.0, 56.0]),
                            ..UserCmd::default()
                        },
                    )
                    .unwrap();
                }
                assert_eq!(read_vector(vm, native, b"origin"), Vec3([-4.0, 34.0, 0.0]));
                assert_eq!(
                    services.server.entities.columns.position[entity.slot as usize],
                    Vec3::default()
                );
                vm.project_dirty_entities(services, context).unwrap();
                assert_eq!(
                    services.server.entities.columns.position[entity.slot as usize],
                    Vec3([-4.0, 34.0, 0.0])
                );
                assert_eq!(services.server.area.bounds(entity), None);
            });
        }
    }
}

#[test]
fn native_motion_fields_and_frame_scalars_publish_nonphysical_aliases() {
    use qa_compat::quakec::MotionField;
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|image| {
            for field in image.fielddefs.iter_mut() {
                match image.names.get(field.name).unwrap() {
                    b"flags" => field.offset = 62,
                    b"waterlevel" => field.offset = 0,
                    b"watertype" => field.offset = 1,
                    b"groundentity" => field.offset = 2,
                    _ => {}
                }
            }
        });
        with_native_client(&mut vm, source, |vm, services, context, native, client| {
            let entity = services.server.clients[client.0 as usize].entity;
            vm.project_entities(services, context).unwrap();
            let world = services
                .server
                .entities
                .capture_reference(services.server.entities.id_at(0).unwrap())
                .unwrap();
            assert_eq!(world.reference(), None);
            for field in [
                MotionField::Flags(512),
                MotionField::WaterLevel(3.0),
                MotionField::WaterType(-3.0),
                MotionField::Ground(world),
            ] {
                vm.write_motion_field(&mut services.server.entities, context, native, field)
                    .unwrap();
            }
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize],
                Vec3([3.0, -3.0, 0.0])
            );
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                0
            );
            vm.project_dirty_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                512
            );
            assert_eq!(services.server.area.bounds(entity), None);
        });
    }
    let mut vm = vm(|image| {
        for field in image.fielddefs.iter_mut() {
            if image.names.get(field.name) == Some(b"origin".as_slice()) {
                field.offset = 62;
            }
        }
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, client| {
            let entity = services.server.clients[client.0 as usize].entity;
            vm.project_entities(services, context).unwrap();
            let mut player = PlayerState::default();
            player.body.position.0[0] = 42.0;
            vm.write_frame_scalar(
                &services.server.entities,
                context,
                native,
                entity,
                &mut player,
                MotionWrite::Origin(0),
            )
            .unwrap();
            assert_eq!(player.body.position.0[0], 42.0);
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                0
            );
            vm.project_dirty_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                42
            );
            assert_eq!(
                services.server.entities.columns.position[entity.slot as usize].0[0],
                42.0
            );
            assert_eq!(services.server.area.bounds(entity), None);
        },
    );
}

#[test]
fn native_client_getters_keep_raw_words_and_require_only_the_executed_typed_fields() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = client_vectors(|_| {});
        with_native_client(&mut vm, source, |vm, _, _, native, _| {
            vector(vm, native, b"v_angle", Vec3([12.0, 34.0, 56.0]));
            vector(vm, native, b"view_ofs", Vec3([99.0, 88.0, 22.25]));
            number(vm, native, b"waterlevel", 257.5);
            number(vm, native, b"teleport_time", 16_777_216.0);
            write(
                vm,
                native,
                b"oldorigin",
                &[0x7fc1_2345, (-0.0f32).to_bits(), 3.0f32.to_bits()],
            );
            vector(vm, native, b"movedir", Vec3([-50.0, 20.0, 0.0]));
            assert_eq!(vm.client_view_angles(native), Ok(Vec3([12.0, 34.0, 56.0])));
            assert_eq!(vm.client_view_height(native), Ok(22.25));
            assert_eq!(vm.client_water_level(native), Ok(257.5));
            assert_eq!(vm.client_teleport_time(native), Ok(16_777_216.0));
            assert_eq!(
                vm.client_old_origin(native, 0).unwrap().to_bits(),
                0x7fc1_2345
            );
            assert_eq!(
                vm.client_old_origin(native, 1).unwrap().to_bits(),
                (-0.0f32).to_bits()
            );
            assert_eq!(vm.client_move_direction(native, 0), Ok(-50.0));
            assert_eq!(vm.client_move_direction(native, 1), Ok(20.0));
            assert_eq!(vm.client_old_origin(native, 3), Err(CallError::Memory));
            assert_eq!(
                vm.client_move_direction(native, usize::MAX),
                Err(CallError::Memory)
            );
            assert_eq!(vm.client_water_level(0), Err(CallError::Entity));
            assert_eq!(vm.client_view_height(native + 1), Err(CallError::Entity));
            // A raw source definition edit does not change the load-cached ABI.
            for field in vm.image.fielddefs.iter_mut() {
                if vm.image.names.get(field.name) == Some(b"oldorigin".as_slice()) {
                    field.offset = 0;
                }
            }
            assert_eq!(vm.client_old_origin(native, 2), Ok(3.0));
        });
        let mut absent = self::vm(|_| {});
        with_native_client(&mut absent, source, |vm, _, _, native, _| {
            assert_eq!(vm.client_old_origin(native, 0), Err(CallError::Memory));
            assert_eq!(vm.client_move_direction(native, 0), Err(CallError::Memory));
            assert_eq!(vm.client_view_height(native), Ok(0.0));
            assert_eq!(vm.client_water_level(native), Ok(0.0));
        });
    }
}

#[test]
fn native_client_oldorigin_and_teleport_stores_refresh_bounds_aliases_without_full_projection() {
    use qa_compat::quakec::MotionField;
    let mut vm = client_vectors(|image| {
        for field in image.fielddefs.iter_mut() {
            match image.names.get(field.name).unwrap() {
                b"oldorigin" => field.offset = 9,
                b"teleport_time" => field.offset = 12,
                _ => {}
            }
        }
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, client| {
            let entity = services.server.clients[client.0 as usize].entity;
            vector(vm, native, b"mins", Vec3([-1.0; 3]));
            vector(vm, native, b"maxs", Vec3([1.0; 3]));
            vm.prepare_motion(&mut services.server.entities, context, native)
                .unwrap();
            write(vm, native, b"classname", &[u32::MAX]);
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::OldOrigin {
                    axis: 0,
                    value: -7.0,
                },
            )
            .unwrap();
            assert_eq!(vm.client_old_origin(native, 0), Ok(-7.0));
            assert_eq!(
                services.server.entities.columns.mins[entity.slot as usize],
                Vec3([-7.0, -1.0, -1.0])
            );
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::TeleportTime(9.0),
            )
            .unwrap();
            assert_eq!(vm.client_teleport_time(native), Ok(9.0));
            assert_eq!(
                services.server.entities.columns.maxs[entity.slot as usize],
                Vec3([9.0, 1.0, 1.0])
            );
            let before = vm
                .entities
                .read(u64::from(native), vm.stride)
                .unwrap()
                .to_vec();
            assert_eq!(
                vm.write_motion_field(
                    &mut services.server.entities,
                    context,
                    native,
                    MotionField::OldOrigin {
                        axis: 3,
                        value: 11.0
                    }
                ),
                Err(CallError::Memory)
            );
            assert_eq!(
                vm.entities.read(u64::from(native), vm.stride).unwrap(),
                before
            );
            assert_eq!(services.server.area.bounds(entity), None);
            write(vm, native, b"classname", &[0]);
            number(vm, native, b"frame", 29.0);
            vm.project_dirty_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                29
            );
        },
    );
}

#[test]
fn native_control_captures_ground_before_aliased_punch_stores() {
    let mut vm = vm(|image| {
        for field in image.fielddefs.iter_mut() {
            if image.names.get(field.name) == Some(b"flags".as_slice()) {
                field.offset = 42;
            }
        }
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            number(vm, native, b"movetype", 3.0);
            number(vm, native, b"health", 100.0);
            vector(vm, native, b"punchangle", Vec3([512.0, 0.0, 0.0]));
            let start = vm
                .prepare_control(services, context, native, 0.1)
                .unwrap()
                .unwrap();
            assert!(start.on_ground);
            assert_eq!(read(vm, native, b"flags"), 511.0f32.to_bits());
            assert_eq!(start.body_angles, read_vector(vm, native, b"angles"));
        },
    );
}

#[test]
fn native_frame_import_maps_foreign_mode_water_and_ground_without_changing_roles_or_bounds() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, client| {
            let entity = services.server.clients[client.0 as usize].entity;
            let ground = services
                .server
                .entities
                .active()
                .find(|id| {
                    services.server.entities.columns.native_entity[id.slot as usize]
                        == Some(NativeEntity {
                            module: context.module,
                            slot: 0,
                        })
                })
                .unwrap();
            vector(vm, native, b"origin", Vec3([100.0, 200.0, 300.0]));
            vector(vm, native, b"velocity", Vec3([7.0, 8.0, 9.0]));
            vector(vm, native, b"mins", Vec3([-80.0; 3]));
            number(vm, native, b"health", 100.0);
            number(vm, native, b"flags", 512.0);
            write(vm, native, b"groundentity", &[0]);
            number(vm, native, b"waterlevel", 257.5);
            number(vm, native, b"watertype", -4.0);
            number(vm, native, b"teleport_time", 10.25);
            let player = &mut services.server.clients[client.0 as usize].player;
            let bounds = (player.body.mins, player.body.maxs);
            player.movement.water_jump_seconds = 7.0;
            for rules in [
                RuleSetId::QuakeWorld,
                RuleSetId::Quake2,
                RuleSetId::Quake2Rerelease,
                RuleSetId::Quake3,
            ] {
                player.movement_rules = rules;
                for (mode, phase, expected) in [
                    (0.0, Motion::None, MovementMode::Frozen),
                    (3.0, Motion::Walk, MovementMode::Walk),
                    (4.0, Motion::Step, MovementMode::Walk),
                    (5.0, Motion::Fly, MovementMode::Fly),
                    (6.0, Motion::Toss, MovementMode::Walk),
                    (8.0, Motion::Noclip, MovementMode::Noclip),
                    (9.0, Motion::FlyMissile, MovementMode::Fly),
                    (10.0, Motion::Bounce, MovementMode::Walk),
                ] {
                    number(vm, native, b"movetype", mode);
                    vm.read_frame_client(
                        &services.server.entities,
                        context,
                        native,
                        entity,
                        phase,
                        player,
                    )
                    .unwrap();
                    assert_eq!(player.movement.mode, expected);
                    assert_eq!(player.body.position, Vec3([100.0, 200.0, 300.0]));
                    assert_eq!((player.body.mins, player.body.maxs), bounds);
                    assert_eq!(
                        (player.movement_rules, player.trace_rules),
                        (rules, RuleSetId::Quake2)
                    );
                    assert_eq!(player.flags, 512);
                    assert!(player.movement.grounded);
                    assert_eq!(player.movement.ground, Some(ground));
                    assert_eq!(player.movement.water_level, 1);
                    assert_eq!(
                        player.movement.water_contents,
                        Contents::from_q1(-4, Contents::EMPTY).0
                    );
                    assert_eq!(player.movement.teleport_hold_until, 10.25);
                    assert_eq!(player.movement.water_jump_seconds, 7.0);
                }
            }
            number(vm, native, b"movetype", 3.0);
            number(vm, native, b"health", -1.0);
            vm.read_frame_client(
                &services.server.entities,
                context,
                native,
                entity,
                Motion::Walk,
                player,
            )
            .unwrap();
            assert_eq!(player.movement.mode, MovementMode::Dead);
            number(vm, native, b"movetype", 11.0);
            assert_eq!(
                vm.read_frame_client(
                    &services.server.entities,
                    context,
                    native,
                    entity,
                    Motion::Unsupported,
                    player,
                ),
                Err(CallError::Unsupported)
            );
            assert_eq!(player.movement.mode, MovementMode::Dead);
        },
    );
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
        write(vm, native, b"solid", &[2.0f32.to_bits()]);
        write(vm, native, b"touch", &[6]);
        assert_eq!(vm.touch_callback(native, false), Ok(Some(CallbackId(0))));
        write(vm, native, b"touch", &[0]);
        assert_eq!(vm.touch_callback(native, false), Ok(None));
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
fn native_impact_saves_globals_before_clock_and_preserves_distinct_callback_time() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for alias_time in [false, true] {
            let time_at = if alias_time { 34 } else { 35 };
            let mut vm = vm(|image| {
                image
                    .globaldefs
                    .iter_mut()
                    .find(|global| image.names.get(global.name) == Some(b"time".as_slice()))
                    .unwrap()
                    .offset = time_at;
                image.statements = [
                    (Opcode::StoreF, [110, time_at as i16, 0]),
                    (Opcode::StoreEnt, [111, 36, 0]),
                    (Opcode::Return, [1, 0, 0]),
                    (Opcode::StoreF, [time_at as i16, 112, 0]),
                    (Opcode::Return, [1, 0, 0]),
                ]
                .map(|(opcode, operands)| Statement {
                    opcode,
                    source_opcode: opcode as u16,
                    operands,
                })
                .to_vec()
                .into_boxed_slice();
                image.functions[1].first_statement = 0;
                image.functions[2].first_statement = 3;
            });
            with_native_client(&mut vm, source, |vm, _, _, native, _| {
                vm.image.globals[34..37].copy_from_slice(&[777, 7.0f32.to_bits(), 888]);
                vm.image.globals[110] = 17.25f32.to_bits();
                vm.image.globals[111] = 999;
                let outer = vm.prepare_impact(ThinkTime::Seconds(12.5)).unwrap();
                assert_eq!(vm.image.globals[time_at], 12.5f32.to_bits());
                let first = vm.prepare_touch_call(0, native, None).unwrap();
                vm.call(&mut NoImports, 1, 64, false).unwrap();
                vm.finish_client_call(first).unwrap();
                let second = vm.prepare_touch_call(native, 0, None).unwrap();
                vm.call(&mut NoImports, 2, 64, false).unwrap();
                assert_eq!(
                    vm.image.globals[112],
                    if alias_time {
                        native
                    } else {
                        17.25f32.to_bits()
                    }
                );
                vm.finish_client_call(second).unwrap();
                vm.finish_client_call(outer).unwrap();
                assert_eq!(vm.image.globals[34], 777);
                assert_eq!(vm.image.globals[36], 888);
                assert_eq!(
                    vm.image.globals[35],
                    if alias_time {
                        7.0f32.to_bits()
                    } else {
                        17.25f32.to_bits()
                    }
                );
            });
        }
    }
}

#[test]
fn native_impact_missing_required_globals_preserves_all_words() {
    for missing in [b"self".as_slice(), b"other", b"time"] {
        let mut vm = vm(|image| {
            image.globaldefs = image
                .globaldefs
                .iter()
                .copied()
                .filter(|global| image.names.get(global.name) != Some(missing))
                .collect::<Vec<_>>()
                .into_boxed_slice();
        });
        with_native_client(&mut vm, RuleSetId::Quake, |vm, _, _, _, _| {
            vm.image.globals[34..37].copy_from_slice(&[777, 7.0f32.to_bits(), 888]);
            let before = vm.image.globals.to_vec();
            assert_eq!(
                vm.prepare_impact(ThinkTime::Seconds(12.5)),
                Err(CallError::Memory)
            );
            assert_eq!(vm.image.globals.as_ref(), before.as_slice());
        });
    }
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
                (3.75, Motion::Walk),
                (4.75, Motion::Step),
                (5.75, Motion::Fly),
                (6.75, Motion::Toss),
                (9.75, Motion::FlyMissile),
                (10.75, Motion::Bounce),
                (11.0, Motion::Unsupported),
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
            let maximum = vm.max_velocity(services.cvars).unwrap();
            assert_eq!(maximum, 4.0);
            vector(vm, native, b"origin", Vec3([f32::NAN, f32::INFINITY, -2.0]));
            vector(vm, native, b"velocity", Vec3([f32::INFINITY, 3.0, -99.0]));
            let classname = vm.insert_string(b"raw\x80_player").unwrap() as u32;
            write(vm, native, b"classname", &[classname]);
            assert_eq!(vm.classname(native).unwrap(), b"raw\x80_player");
            vm.prepare_velocity(&mut services.server.entities, context, native)
                .unwrap();
            let entity = services.server.clients[id.0 as usize].entity;
            let checked = qa_session::physics::check_velocity(
                &mut services.server.entities,
                entity,
                maximum,
                |entities, stage, _invalid| vm.write_motion(entities, context, native, stage),
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
                vm.prepare_velocity(&mut services.server.entities, context, native),
                Err(CallError::Entity)
            );
        },
    );
}

#[test]
fn native_nonclient_velocity_uses_shared_sanitizer_without_client_or_angular_fields() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
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
        with_native_client(&mut vm, source, |vm, services, context, _, _| {
            let native = vm.spawn_entity(services, context).unwrap();
            assert!(vm.entity_number(native).unwrap() > vm.client_count().unwrap() as u32);
            let entity = vm
                .resolve_entity(&services.server.entities, context.module, native)
                .unwrap()
                .unwrap();
            vector(vm, native, b"origin", Vec3([f32::NAN, 2.0, 3.0]));
            vector(vm, native, b"velocity", Vec3([9.0, f32::INFINITY, -9.0]));
            vm.prepare_velocity(&mut services.server.entities, context, native)
                .unwrap();
            let checked = qa_session::physics::check_velocity(
                &mut services.server.entities,
                entity,
                4.0,
                |entities, stage, _invalid| vm.write_motion(entities, context, native, stage),
            )
            .unwrap();
            assert_eq!((checked.invalid_velocity, checked.invalid_origin), (2, 1));
            assert_eq!(read_vector(vm, native, b"origin"), Vec3([0.0, 2.0, 3.0]));
            assert_eq!(read_vector(vm, native, b"velocity"), Vec3([4.0, 0.0, -4.0]));
            assert!(services.server.area.bounds(entity).is_none());
            assert!(services.server.free_entity(entity, context.clock));
            assert_eq!(
                vm.prepare_velocity(&mut services.server.entities, context, native),
                Err(CallError::Entity)
            );
        });
    }
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
            vm.prepare_velocity(&mut services.server.entities, context, native)
                .unwrap();
            let entity = services.server.clients[id.0 as usize].entity;
            let checked = qa_session::physics::check_velocity(
                &mut services.server.entities,
                entity,
                4.0,
                |entities, stage, _invalid| vm.write_motion(entities, context, native, stage),
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
            assert_eq!(options.body_angles, expected);
            assert!(!options.on_ground);
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
fn nq_control_preparation_leaves_mode_and_waterjump_admission_to_shared_physics() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            number(vm, native, b"health", 1.0);
            for (mode, flags) in [
                (8.0, 0.0),
                (8.5, 0.0),
                (8.0, 2048.0),
                (3.0, 0.0),
                (5.0, 0.0),
                (6.0, 0.0),
                (10.0, 0.0),
            ] {
                number(vm, native, b"movetype", mode);
                number(vm, native, b"flags", flags);
                assert!(
                    vm.prepare_control(services, context, native, 0.1)
                        .unwrap()
                        .is_some()
                );
            }
        },
    );
}

#[test]
fn negative_native_water_depth_imports_dry_without_changing_the_raw_word() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|_| {});
        with_native_client(&mut vm, source, |vm, services, context, native, id| {
            number(vm, native, b"movetype", 6.0);
            number(vm, native, b"waterlevel", -1.0);
            services.server.clients[id.0 as usize]
                .player
                .movement
                .water_level = 2;
            vm.project_entity(services, context, native).unwrap();
            assert_eq!(
                services.server.clients[id.0 as usize]
                    .player
                    .movement
                    .water_level,
                0
            );
            assert_eq!(read(vm, native, b"waterlevel"), (-1.0f32).to_bits());
            if source == RuleSetId::Quake {
                let entity = services.server.clients[id.0 as usize].entity;
                let player = &mut services.server.clients[id.0 as usize].player;
                vm.read_frame_client(
                    &services.server.entities,
                    context,
                    native,
                    entity,
                    Motion::Toss,
                    player,
                )
                .unwrap();
                assert_eq!(player.movement.water_level, 0);
                assert_eq!(read(vm, native, b"waterlevel"), (-1.0f32).to_bits());
            }
        });
    }
}

#[test]
fn nq_frame_finish_roundtrips_environment_and_view_without_changing_absolute_teleport() {
    let mut vm = vm(|_| {});
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            let entity = services.server.clients[id.0 as usize].entity;
            let ground = services
                .server
                .entities
                .find_native(
                    NativeEntity {
                        module: context.module,
                        slot: 0,
                    },
                    None,
                )
                .unwrap();
            number(vm, native, b"movetype", 3.0);
            number(vm, native, b"health", 100.0);
            number(vm, native, b"flags", 2561.0);
            number(vm, native, b"teleport_time", 13.75);
            write(vm, native, b"groundentity", &[native]);
            let player = &mut services.server.clients[id.0 as usize].player;
            player.movement.water_level = 2;
            player.movement.water_contents = Contents::SLIME.0;
            player.movement.grounded = false;
            player.movement.ground = None;
            player.movement.water_jump_seconds = 7.0;
            player.view_angles = Vec3([12.0, 34.0, 56.0]);
            vm.finish_frame_client(services, context, native).unwrap();
            assert_eq!(read(vm, native, b"waterlevel"), 2.0f32.to_bits());
            assert_eq!(read(vm, native, b"watertype"), (-4.0f32).to_bits());
            assert_eq!(read(vm, native, b"flags"), 2049.0f32.to_bits());
            assert_eq!(read(vm, native, b"groundentity"), native);
            assert_eq!(read(vm, native, b"teleport_time"), 13.75f32.to_bits());
            assert_eq!(
                read_vector(vm, native, b"v_angle"),
                Vec3([12.0, 34.0, 56.0])
            );
            assert_eq!(services.server.area.bounds(entity), None);
            let player = &mut services.server.clients[id.0 as usize].player;
            player.movement.grounded = true;
            player.movement.ground = Some(ground);
            player.view_angles = Vec3([7.0, 8.0, 9.0]);
            vm.finish_frame_client(services, context, native).unwrap();
            assert_eq!(read(vm, native, b"groundentity"), 0);
            assert_eq!(read(vm, native, b"flags"), 2561.0f32.to_bits());
            let player = &mut services.server.clients[id.0 as usize].player;
            player.movement.water_level = 0;
            player.movement.water_contents = 0;
            player.movement.grounded = false;
            player.movement.ground = None;
            player.view_angles = Vec3::default();
            vm.project_dirty_entities(services, context).unwrap();
            let player = &services.server.clients[id.0 as usize].player;
            assert_eq!(player.movement.water_level, 2);
            assert_eq!(player.movement.water_contents, Contents::SLIME.0);
            assert!(player.movement.grounded);
            assert_eq!(player.movement.ground, Some(ground));
            assert_eq!(player.view_angles, Vec3([7.0, 8.0, 9.0]));
            assert_eq!(read(vm, native, b"teleport_time"), 13.75f32.to_bits());
            assert_eq!(services.server.area.bounds(entity), None);
        },
    );
}

#[test]
fn packet_and_frame_environment_writes_keep_cached_flags_and_the_view_store_last() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|image| {
            for field in image.fielddefs.iter_mut() {
                field.offset = match image.names.get(field.name).unwrap() {
                    b"waterlevel" | b"flags" | b"teleport_time" => 62,
                    b"watertype" => 1,
                    b"groundentity" => 2,
                    b"v_angle" => 0,
                    _ => field.offset,
                };
            }
        });
        with_native_client(&mut vm, source, |vm, services, context, native, id| {
            let entity = services.server.clients[id.0 as usize].entity;
            number(vm, native, b"flags", 513.0);
            let player = &mut services.server.clients[id.0 as usize].player;
            player.movement.water_level = 3;
            player.movement.water_contents = Contents::SLIME.0;
            player.movement.grounded = false;
            player.movement.water_jump_seconds = 7.0;
            player.view_angles = Vec3([7.0, 8.0, 9.0]);
            if source == RuleSetId::Quake {
                vm.finish_frame_client(services, context, native).unwrap();
                assert_eq!(
                    services.server.entities.columns.appearance[entity.slot as usize].frame,
                    0
                );
                vm.project_dirty_entities(services, context).unwrap();
            } else {
                vm.write_client(services, context, native, Vec3::default())
                    .unwrap();
            }
            assert_eq!(read(vm, native, b"flags"), 1.0f32.to_bits());
            assert_eq!(read_vector(vm, native, b"origin"), Vec3([7.0, 8.0, 9.0]));
            assert_eq!(
                services.server.entities.columns.appearance[entity.slot as usize].frame,
                1
            );
        });
    }
}

#[test]
fn foreign_noclip_frame_import_preserves_missing_optional_environment_fields() {
    let mut vm = vm(|image| {
        image.fielddefs = image
            .fielddefs
            .iter()
            .copied()
            .filter(|field| {
                ![
                    b"flags".as_slice(),
                    b"groundentity",
                    b"waterlevel",
                    b"watertype",
                    b"health",
                ]
                .contains(&image.names.get(field.name).unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            let entity = services.server.clients[id.0 as usize].entity;
            let player = &mut services.server.clients[id.0 as usize].player;
            player.flags = 513;
            player.movement.grounded = true;
            player.movement.ground = Some(entity);
            player.movement.water_level = 2;
            player.movement.water_contents = Contents::SLIME.0;
            vm.read_frame_client(
                &services.server.entities,
                context,
                native,
                entity,
                Motion::Noclip,
                player,
            )
            .unwrap();
            assert_eq!(player.movement.mode, MovementMode::Noclip);
            assert_eq!(player.flags, 513);
            assert!(player.movement.grounded);
            assert_eq!(player.movement.ground, Some(entity));
            assert_eq!(player.movement.water_level, 2);
            assert_eq!(player.movement.water_contents, Contents::SLIME.0);
            vm.finish_frame_client(services, context, native).unwrap();
            let player = &services.server.clients[id.0 as usize].player;
            assert_eq!(player.flags, 513);
            assert_eq!(player.movement.water_level, 2);
            assert_eq!(player.movement.water_contents, Contents::SLIME.0);
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
            number(vm, native, b"movetype", 0.0);
            number(vm, native, b"teleport_time", 10.25);
            vector(vm, native, b"origin", Vec3([100.0, 200.0, 300.0]));
            vector(vm, native, b"velocity", Vec3([3.0, -2.0, 6.0]));
            vector(vm, native, b"mins", Vec3([-7.0, -8.0, -9.0]));
            let entity = services.server.clients[id.0 as usize].entity;
            let player = &mut services.server.clients[id.0 as usize].player;
            let bounds = (player.body.mins, player.body.maxs);
            player.movement.water_jump_seconds = 7.0;
            vm.read_frame_client(
                &services.server.entities,
                context,
                native,
                entity,
                Motion::Noclip,
                player,
            )
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
            number(vm, native, b"movetype", 8.0);
            vector(vm, native, b"origin", Vec3([10.0, 20.0, 30.0]));
            vector(vm, native, b"velocity", Vec3([2.0, 3.0, 4.0]));
            let entity = services.server.clients[id.0 as usize].entity;
            let player = &mut services.server.clients[id.0 as usize].player;
            player.movement_rules = RuleSetId::Quake;
            vm.read_frame_client(
                &services.server.entities,
                context,
                native,
                entity,
                Motion::Noclip,
                player,
            )
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
        assert_eq!(player.eye_offset(), Vec3([0.0, 0.0, 22.0]));
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
            assert_eq!(player.eye_offset(), Vec3([0.0, 0.0, 22.0]));
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

#[test]
fn native_nonclient_motion_decodes_loaded_phase_and_keeps_current_raw_mode() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|_| {});
        with_native_client(&mut vm, source, |vm, services, context, _, _| {
            let native = vm.spawn_entity(services, context).unwrap();
            let entity = vm
                .resolve_entity(&services.server.entities, context.module, native)
                .unwrap()
                .unwrap();
            for (raw, phase) in [
                (4.0, Motion::Step),
                (5.0, Motion::Fly),
                (6.0, Motion::Toss),
                (9.0, Motion::FlyMissile),
                (10.0, Motion::Bounce),
                (7.0, Motion::Push),
            ] {
                number(vm, native, b"movetype", raw);
                vm.project_entities(services, context).unwrap();
                let mut cursor = 0;
                let mut found = None;
                while let Some(row) = vm
                    .physics_entity(&services.server.entities, &mut cursor)
                    .unwrap()
                {
                    if row.entity == entity {
                        found = Some(row.phase);
                    }
                }
                assert_eq!(found, Some(phase));
            }
            number(vm, native, b"movetype", 5.75);
            vm.project_entities(services, context).unwrap();
            assert_eq!(
                services.server.entities.columns.motion[entity.slot as usize],
                if source == RuleSetId::QuakeWorld {
                    Motion::Fly
                } else {
                    Motion::Unsupported
                }
            );
            assert_eq!(vm.motion_state(context, native).unwrap().move_type, 5.75);
            number(vm, native, b"movetype", 6.0);
            vm.project_entities(services, context).unwrap();
            number(vm, native, b"movetype", 10.75);
            assert_eq!(
                services.server.entities.columns.motion[entity.slot as usize],
                Motion::Toss
            );
            assert_eq!(vm.motion_state(context, native).unwrap().move_type, 10.75);
        });
    }
}

#[test]
fn native_step_decoding_keeps_raw_mode_and_requires_no_angular_velocity() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = vm(|image| {
            image.fielddefs = image
                .fielddefs
                .iter()
                .copied()
                .filter(|field| image.names.get(field.name) != Some(b"avelocity".as_slice()))
                .collect::<Vec<_>>()
                .into_boxed_slice();
        });
        with_native_client(&mut vm, source, |vm, services, context, _, _| {
            let native = vm.spawn_entity(services, context).unwrap();
            let entity = vm
                .resolve_entity(&services.server.entities, context.module, native)
                .unwrap()
                .unwrap();
            for (raw, expected) in [
                (4.0, Motion::Step),
                (
                    4.75,
                    if source == RuleSetId::QuakeWorld {
                        Motion::Step
                    } else {
                        Motion::Unsupported
                    },
                ),
                (f32::from_bits(0x7fc1_2345), Motion::Unsupported),
            ] {
                number(vm, native, b"movetype", raw);
                vm.project_entities(services, context).unwrap();
                assert_eq!(
                    services.server.entities.columns.motion[entity.slot as usize],
                    expected
                );
                assert_eq!(
                    vm.motion_state(context, native)
                        .unwrap()
                        .move_type
                        .to_bits(),
                    raw.to_bits()
                );
                vm.prepare_motion(&mut services.server.entities, context, native)
                    .unwrap();
                assert_eq!(
                    services.server.entities.columns.angular_velocity[entity.slot as usize],
                    None
                );
            }
        });
    }
}

#[test]
fn native_motion_scalar_writes_refresh_shifted_physical_aliases() {
    use qa_compat::quakec::MotionField;
    let mut vm = vm(|image| {
        for (name, offset) in [
            (b"avelocity".as_slice(), 1),
            (b"groundentity", 4),
            (b"watertype", 2),
            (b"waterlevel", 3),
            (b"flags", 5),
        ] {
            image
                .fielddefs
                .iter_mut()
                .find(|field| image.names.get(field.name) == Some(name))
                .unwrap()
                .offset = offset;
        }
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, id| {
            vector(vm, native, b"origin", Vec3([1.0, 2.0, 3.0]));
            vector(vm, native, b"velocity", Vec3([4.0, 5.0, 6.0]));
            vector(vm, native, b"angles", Vec3([7.0, 8.0, 9.0]));
            vm.prepare_motion(&mut services.server.entities, context, native)
                .unwrap();
            let slot = services.server.clients[id.0 as usize].entity.slot as usize;
            services.server.entities.columns.angular_velocity[slot]
                .as_mut()
                .unwrap()
                .0[0] = -12.0;
            vm.write_motion(
                &mut services.server.entities,
                context,
                native,
                MotionWrite::AngularVelocity(0),
            )
            .unwrap();
            assert_eq!(read_vector(vm, native, b"origin").0[1], -12.0);
            assert_eq!(
                services.server.entities.columns.position[slot],
                read_vector(vm, native, b"origin")
            );
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::Flags(513),
            )
            .unwrap();
            assert_eq!(services.server.entities.columns.velocity[slot].0[2], 513.0);
            let weak = services
                .server
                .entities
                .capture_reference(
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
                )
                .unwrap();
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::Ground(weak),
            )
            .unwrap();
            assert_eq!(
                services.server.entities.columns.velocity[slot].0[1].to_bits(),
                0
            );
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::WaterType(-3.75),
            )
            .unwrap();
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::WaterLevel(1.25),
            )
            .unwrap();
            assert_eq!(services.server.entities.columns.position[slot].0[2], -3.75);
            assert_eq!(services.server.entities.columns.velocity[slot].0[0], 1.25);
            assert_eq!(vm.motion_water_type(context, native), Ok(-3.75));
            assert_eq!(read(vm, native, b"waterlevel"), 1.25f32.to_bits());
            for gravity in [0.0, -0.25, 1.0] {
                number(vm, native, b"gravity", gravity);
                assert_eq!(vm.motion_gravity(context, native), Ok(Some(gravity)));
            }
            let foreign_entity = services
                .server
                .entities
                .allocate(context.clock, ModuleId(55), context.allocation)
                .unwrap()
                .id;
            let foreign = services
                .server
                .entities
                .capture_reference(foreign_entity)
                .unwrap();
            assert_eq!(foreign.reference(), None);
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::Ground(foreign),
            )
            .unwrap();
            let ground = read(vm, native, b"groundentity");
            assert_ne!(ground, 0);
            assert_eq!(
                vm.resolve_entity(&services.server.entities, context.module, ground)
                    .unwrap(),
                Some(foreign_entity)
            );
            assert!(
                services
                    .server
                    .area
                    .bounds(services.server.clients[id.0 as usize].entity)
                    .is_none()
            );
        },
    );
}

#[test]
fn native_motion_optional_fields_reject_only_the_executed_boundary() {
    use qa_compat::quakec::MotionField;
    let mut vm = vm(|image| {
        image.fielddefs = image
            .fielddefs
            .iter()
            .copied()
            .filter(|field| {
                ![
                    b"gravity".as_slice(),
                    b"waterlevel",
                    b"groundentity",
                    b"solid",
                    b"flags",
                ]
                .contains(&image.names.get(field.name).unwrap())
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
    });
    with_native_client(
        &mut vm,
        RuleSetId::Quake,
        |vm, services, context, native, _| {
            let state = vm.motion_state(context, native).unwrap();
            assert!(state.solid.is_none());
            assert!(state.flags.is_none());
            assert_eq!(vm.motion_gravity(context, native), Ok(None));
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::WaterType(-3.0),
            )
            .unwrap();
            assert_eq!(vm.motion_water_type(context, native), Ok(-3.0));
            assert_eq!(
                vm.write_motion_field(
                    &mut services.server.entities,
                    context,
                    native,
                    MotionField::WaterLevel(1.0)
                ),
                Err(CallError::Memory)
            );
            assert_eq!(vm.motion_water_type(context, native), Ok(-3.0));
            assert_eq!(
                vm.write_motion_field(
                    &mut services.server.entities,
                    context,
                    native,
                    MotionField::Flags(512)
                ),
                Err(CallError::Memory)
            );
            assert_eq!(vm.touch_callback(native, true), Ok(None));
            write(vm, native, b"touch", &[6]);
            assert_eq!(vm.touch_callback(native, true), Err(CallError::Memory));
        },
    );
}

#[test]
fn native_weak_touch_other_preserves_freed_ordinal_without_world_fallback() {
    let mut vm = vm(|image| {
        image.names.reserve(3, 24).unwrap();
        let mut fields = image.fielddefs.to_vec();
        for (value_type, offset, name) in [
            (1, 70, b"model".as_slice()),
            (2, 71, b"takedamage".as_slice()),
            (2, 72, b"nextthink".as_slice()),
        ] {
            fields.push(qa_formats::program::quakec::Definition {
                value_type,
                offset,
                name: image.names.intern(name).unwrap(),
            });
        }
        image.fielddefs = fields.into_boxed_slice();
        image.entityfields = 73;
    });
    with_native_client(&mut vm, RuleSetId::Quake, |vm, services, context, _, _| {
        let native = vm.spawn_entity(services, context).unwrap();
        let reference = NativeEntityRef::entity(
            ResourceNamespace::Module(context.module),
            vm.entity_number(native).unwrap(),
        );
        vm.remove_entity(services, context, native).unwrap();
        assert_eq!(
            vm.resolve_entity(&services.server.entities, context.module, native),
            Ok(None)
        );
        assert_eq!(vm.weak_entity_offset(context.module, reference), Ok(native));
        let restore = vm.prepare_touch_call(0, native, None).unwrap();
        assert_eq!(vm.image.globals[36], native);
        vm.finish_client_call(restore).unwrap();
        assert_eq!(
            vm.weak_entity_offset(
                context.module,
                NativeEntityRef::entity(ResourceNamespace::Module(context.module), 0)
            ),
            Ok(0)
        );
        assert_eq!(
            vm.weak_entity_offset(
                context.module,
                NativeEntityRef::entity(ResourceNamespace::Client(ClientId(0)), reference.ordinal)
            ),
            Err(CallError::Entity)
        );
        assert_eq!(
            vm.weak_entity_offset(
                context.module,
                NativeEntityRef::entity(reference.namespace, u32::MAX)
            ),
            Err(CallError::Entity)
        );
    });
}
