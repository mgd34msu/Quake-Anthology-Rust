#[path = "support/scalar_import.rs"]
mod scalar_import;
use qa_core::primitives::IndexSpan;
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod model_support;
#[path = "support/service_program.rs"]
mod service_program;
#[path = "support/visibility_world.rs"]
mod visibility_world;
use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, Invocation, Q3_CLIENT, Q3_SERVER, Q3_UI, QvmCalls, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::ThinkTime;
use qa_core::{
    events::FrameEvent,
    primitives::{ModuleId, RuleSetId},
    sys_events::EventTime,
};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

fn context() -> CallContext {
    CallContext {
        native_rules: qa_core::primitives::RuleSetId::Quake3,
        trace_rules: qa_core::primitives::RuleSetId::Quake3,
        sound_policy: qa_gameplay::rules::sound_policy(qa_core::primitives::RuleSetId::Quake3),
        server_frame: 0,
        module: ModuleId(1),
        clock: ThinkTime::Milliseconds(17),
        console: Context::default(),
        allocation: AllocationPolicy::EDICT,
        link_order: LinkOrder::Head,
    }
}

#[test]
fn cgame_world_and_model_registration_keep_distinct_native_ordinals() {
    for addresses in [
        Addresses::Qvm { mask: 127 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let mut storage =
            ServiceStorage::load_native(&[(ModuleId(1), 0, RuleSetId::Quake3)], 0, &console.cvars)
                .unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut memory = ModuleMemory::load(0, 128, &[0; 128]).unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut invocation = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses,
            arguments: &[0],
        };
        assert_eq!(
            Q3_CLIENT
                .invoke(37, &mut invocation, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(
            Q3_CLIENT
                .invoke(36, &mut invocation, &mut unknown)
                .map(scalar_import::word),
            Err(CallError::Memory)
        );
    }
}

#[test]
fn model_tag_calls_keep_ui_void_result_and_ignore_names_without_tag_storage() {
    use qa_app::model_assets::{ModelAssets, ModelRequest};
    use qa_compat::services::{ModelLoader, ModelLookup};
    let fixture = model_support::Fixture::surfaces();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut builder = ModelAssets::new(
        qa_render::Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
        .unwrap();
    let expected = builder
        .catalog()
        .lerp_tag(handle, 0, 1, 0.25, b"tag_weapon")
        .unwrap();
    builder.end_initial_registration();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.model_assets = Some(builder);
    for addresses in [
        Addresses::Qvm { mask: 255 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        let mut console = Console::new(Context::default()).unwrap();
        let mut storage =
            ServiceStorage::load_native(&[(ModuleId(1), 0, RuleSetId::Quake3)], 0, &console.cvars)
                .unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut memory = ModuleMemory::load(0, 256, &[0xff; 256]).unwrap();
        memory.write(16, b"tag_weapon\0").unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let orientation = |memory: &ModuleMemory<'_>| qa_core::primitives::Orientation {
            origin: memory.read_vec3(64).unwrap(),
            axes: std::array::from_fn(|axis| memory.read_vec3(76 + axis as u64 * 12).unwrap()),
        };
        let mut invocation = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses,
            arguments: &[64, handle as u64, 0, 1, 0.25f32.to_bits() as u64, 16],
        };
        assert_eq!(
            Q3_CLIENT
                .invoke(48, &mut invocation, &mut unknown)
                .map(scalar_import::word),
            Ok(1)
        );
        assert_eq!(orientation(invocation.memory), expected);
        assert_eq!(
            Q3_UI
                .invoke(29, &mut invocation, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(orientation(invocation.memory), expected);
        // No tags: original R_LerpTag never consumes name, frames or fraction.
        invocation.arguments = &[64, 0, 0, 0, 0, 250];
        assert_eq!(
            Q3_CLIENT
                .invoke(48, &mut invocation, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(orientation(invocation.memory), Default::default());
    }
}

#[test]
fn q3_visibility_imports_use_compiled_pvs_and_native_portal_policy() {
    use qa_core::primitives::Vec3;
    use qa_world::visibility::PvsRows;
    for addresses in [
        Addresses::Qvm { mask: 127 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        let base = if matches!(addresses, Addresses::Qvm { .. }) {
            0
        } else {
            1 << 40
        };
        let mut runtime = Runtime::load(1, []).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let ctx = Context {
            source: RuleSetId::Quake3,
            ..Context::default()
        };
        let no_areas = console.cvars.bind("cm_noAreas", ctx).unwrap();
        let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
        memory.write_vec3(base + 16, Vec3([1.0, 0.0, 0.0])).unwrap();
        memory
            .write_vec3(base + 32, Vec3([-1.0, 0.0, 0.0]))
            .unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        for (selectors, row_bytes, visible) in [
            ([Some(0), Some(1)], vec![3, 3], true),
            ([Some(0), Some(1)], vec![1, 2], false),
            ([None, Some(1)], vec![3, 3], true),
        ] {
            runtime.collision = Some(visibility_world::collision(
                &runtime.geometry,
                selectors,
                PvsRows::load(vec![Some(0), Some(1)], row_bytes, None).unwrap(),
            ));
            for (open, ignore, same_leaf, expected) in [
                (false, "0", false, false),
                (true, "0", false, visible),
                (false, "1", false, visible),
                (false, "0.5", false, false),
                (false, "1.5", false, visible),
                (false, "0", true, true),
            ] {
                runtime
                    .collision
                    .as_mut()
                    .unwrap()
                    .portals
                    .as_mut()
                    .unwrap()
                    .set(0, open)
                    .unwrap();
                console.cvars.force_write(no_areas, ignore).unwrap();
                let wrap = if base == 0 { 128 } else { 0 };
                let second = if same_leaf { 16 } else { 32 };
                let arguments = [base + 16 + wrap, base + second + wrap];
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    native_pmove: None,
                    native_player: None,
                    context: context(),
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(
                    Q3_SERVER
                        .invoke(26, &mut call, &mut unknown)
                        .map(scalar_import::word),
                    Ok(u64::from(expected))
                );
                assert_eq!(
                    Q3_SERVER
                        .invoke(27, &mut call, &mut unknown)
                        .map(scalar_import::word),
                    Ok(u64::from(same_leaf || visible))
                );
            }
            if matches!(addresses, Addresses::Qvm { .. }) {
                use qa_formats::program::qvm::Opcode::*;
                for ordinal in [26, 27] {
                    let mut vm = service_program::program(
                        &[
                            (Enter, 64),
                            (Const, 16),
                            (Arg, 8),
                            (Const, 32),
                            (Arg, 12),
                            (Const, -ordinal - 1),
                            (Call, 0),
                            (Leave, 64),
                        ],
                        memory.read(base, 48).unwrap(),
                    );
                    let mut services =
                        runtime.engine_services(&mut console, &mut storage, &mut scratch);
                    assert_eq!(
                        vm.call(
                            &mut QvmCalls {
                                services: &mut services,
                                table: &Q3_SERVER,
                                context: context(),
                                platform_time: EventTime(0),
                                command: Default::default(),
                                argument_selection:
                                    &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                                unknown: &mut unknown,
                            },
                            [0; 10],
                            128,
                            false
                        ),
                        Ok(if ordinal == 26 { 0 } else { i32::from(visible) })
                    );
                }
            }
        }
        assert_eq!(unknown.calls, 0);
        assert!(runtime.server.events.is_empty());
    }
}

#[test]
fn q3_area_connection_keeps_native_bounds_and_integer_override() {
    use qa_formats::program::qvm::Opcode::*;
    use qa_world::visibility::PvsRows;
    for addresses in [
        Addresses::Qvm { mask: 127 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.collision = Some(visibility_world::collision(
            &runtime.geometry,
            [Some(0), Some(1)],
            PvsRows::load(vec![Some(0); 2], vec![3], None).unwrap(),
        ));
        let mut console = Console::new(Context::default()).unwrap();
        let no_areas = console
            .cvars
            .bind(
                "cm_noAreas",
                Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
            )
            .unwrap();
        let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut memory = ModuleMemory::load(0, 128, &[]).unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        for (open, ignore, arguments, expected) in [
            (false, "0", [0, 0], Ok(1)),
            (false, "0", [0, 1], Ok(0)),
            (true, "0", [0, 1], Ok(1)),
            (true, "0", [1, 0], Ok(1)),
            (false, "0", [1, 1], Ok(1)),
            (false, "0", [u32::MAX as u64, 1], Ok(0)),
            (false, "0", [0, u32::MAX as u64], Ok(0)),
            (false, "0", [2, 1], Err(CallError::Geometry)),
            (false, "0", [0, 2], Err(CallError::Geometry)),
            (false, "1", [2, u32::MAX as u64], Ok(1)),
            (false, "0.5", [0, 1], Ok(0)),
            (false, "1.5", [3, 4], Ok(1)),
            (false, "0", [1 << 40, 1], Ok(0)),
        ] {
            runtime
                .collision
                .as_mut()
                .unwrap()
                .portals
                .as_mut()
                .unwrap()
                .set(0, open)
                .unwrap();
            console.cvars.force_write(no_areas, ignore).unwrap();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: context(),
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &arguments,
            };
            assert_eq!(
                Q3_SERVER
                    .invoke(29, &mut call, &mut unknown)
                    .map(scalar_import::word),
                expected
            );
            if matches!(addresses, Addresses::Qvm { .. }) {
                let mut vm = service_program::program(
                    &[
                        (Enter, 64),
                        (Const, arguments[0] as u32 as i32),
                        (Arg, 8),
                        (Const, arguments[1] as u32 as i32),
                        (Arg, 12),
                        (Const, -30),
                        (Call, 0),
                        (Leave, 64),
                    ],
                    &[],
                );
                let mut calls = QvmCalls {
                    services: &mut services,
                    table: &Q3_SERVER,
                    context: context(),
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    unknown: &mut unknown,
                };
                let result = vm.call(&mut calls, [0; 10], 128, false);
                match expected {
                    Ok(value) => assert_eq!(result.unwrap(), value as i32),
                    Err(_) => assert!(result.is_err()),
                }
            }
        }
        assert_eq!(unknown.calls, 0);
        assert!(runtime.server.events.is_empty());
    }
}

#[test]
fn q3_pointcontents_uses_native_pass_identity_over_shared_world_geometry() {
    use qa_app::WorldCollision;
    use qa_core::primitives::{
        Body, Bounds, ClipNode, CollisionShape, NativeEntity, Plane, SurfaceFlags, Vec3,
    };
    use qa_world::{
        area::{LinkFlags, LinkIntent},
        collision::{
            Contents,
            brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
            hulls::HullModel,
            surfaces::SurfaceTable,
        },
    };
    for addresses in [
        Addresses::Qvm { mask: 127 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        for hull in [true, false] {
            let mut runtime = Runtime::load(1, []).unwrap();
            let bounds = Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            };
            let geometry = if hull {
                let plane = Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0);
                let node = ClipNode {
                    plane: 0,
                    children: [-3, -1],
                };
                runtime
                    .geometry
                    .load_hulls(
                        vec![plane],
                        vec![node],
                        vec![node],
                        vec![HullModel {
                            roots: [0; 3],
                            brushes: Default::default(),
                        }],
                        vec![bounds],
                        None,
                        Default::default(),
                    )
                    .unwrap()
            } else {
                runtime
                    .geometry
                    .load_brushes(
                        vec![Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 0.0)],
                        vec![Brush {
                            sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                            contents: Contents::WATER,
                        }],
                        Vec::new(),
                        SurfaceTable::flags(vec![SurfaceFlags::default()]),
                        BrushTree {
                            planes: vec![],
                            nodes: vec![],
                            leaves: vec![CollisionLeaf {
                                stored_contents: Some(Contents::SOLID),
                                brushes: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                                patches: Default::default(),
                            }],
                            leaf_brushes: vec![0],
                            models: vec![ModelRoot::Leaf(0)],
                            leaf_patches: Vec::new(),
                        },
                        vec![bounds],
                    )
                    .unwrap()
            };
            runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
            let bodies = [ModuleId(7), ModuleId(8)].map(|module| {
                let entity = runtime
                    .server
                    .entities
                    .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::EDICT)
                    .unwrap()
                    .id;
                let slot = entity.slot as usize;
                runtime.server.entities.columns.set_body(
                    slot,
                    Body {
                        position: Vec3([1.0, 0.0, 0.0]),
                        mins: Vec3([-1.0; 3]),
                        maxs: Vec3([1.0; 3]),
                        ..Body::default()
                    },
                );
                runtime.server.entities.columns.collision_shape[slot] = CollisionShape::Box;
                runtime.server.entities.columns.native_entity[slot] =
                    Some(NativeEntity { module, slot: 0 });
                assert!(runtime.server.area.link(
                    &runtime.server.entities,
                    entity,
                    LinkFlags::SOLID,
                    LinkOrder::Head,
                    LinkIntent::Explicit
                ));
                entity
            });
            assert_ne!(bodies[0].slot, 0);
            let mut console = Console::new(Context::default()).unwrap();
            let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let base = if matches!(addresses, Addresses::Qvm { .. }) {
                0
            } else {
                1 << 40
            };
            let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
            memory.write_vec3(base + 16, Vec3([1.0, 0.0, 0.0])).unwrap();
            let mut unknown = UnknownCalls::load(1).unwrap();
            // Another module with the same native number remains included.
            for (foreign_linked, pass, expected) in [
                (true, 0i32, 32 | Contents::BODY.0 as u32),
                (false, 0, 32),
                (false, -1, 32 | Contents::BODY.0 as u32),
                (false, 1, 32 | Contents::BODY.0 as u32),
            ] {
                if !foreign_linked {
                    runtime.server.area.unlink(bodies[1]);
                }
                let arguments = [base + 16 + if base == 0 { 128 } else { 0 }, pass as u64];
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                assert_eq!(
                    Q3_SERVER
                        .invoke(
                            25,
                            &mut Invocation {
                                services: &mut services,
                                memory: &mut memory,
                                native_cvars: None,
                                native_resources: None,
                                native_entities: None,
                                native_surfaces: None,
                                native_command: None,
                                native_configs: None,
                                native_pmove: None,
                                native_player: None,
                                context: CallContext {
                                    module: ModuleId(7),
                                    ..context()
                                },
                                platform_time: EventTime(0),
                                command: Default::default(),
                                argument_selection:
                                    &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                                addresses,
                                arguments: &arguments,
                            },
                            &mut unknown
                        )
                        .map(scalar_import::word),
                    Ok(u64::from(expected))
                );
            }
            assert_eq!(unknown.calls, 0);
            assert!(runtime.server.events.is_empty());
            assert_eq!(
                runtime.server.entities.columns.position[bodies[0].slot as usize],
                Vec3([1.0, 0.0, 0.0])
            );
        }
    }
}

#[test]
fn q3_usercmd_reads_native_clients_and_projects_exact_abi_fields() {
    use qa_core::primitives::{NativeEntity, UserCmd, Vec3, WeaponId, buttons};
    use qa_session::clients::Connection;
    for addresses in [
        Addresses::Qvm { mask: 127 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        let mut runtime = Runtime::load(3, []).unwrap();
        let clients = [
            NativeEntity {
                module: ModuleId(8),
                slot: 0,
            },
            NativeEntity {
                module: ModuleId(7),
                slot: 0,
            },
        ]
        .map(|native| {
            runtime
                .server
                .connect(Connection::Bot, native.module, Some(native))
                .unwrap()
        });
        assert_eq!(clients[1].0, 1);
        let target = clients[1];
        let own = runtime
            .catalog
            .registry
            .native_weapon(RuleSetId::Quake3, 7)
            .unwrap();
        let foreign = runtime
            .catalog
            .registry
            .native_weapon(RuleSetId::Quake2, 7)
            .unwrap();
        assert_ne!(own, WeaponId(7));
        let command = UserCmd {
            server_time_ms: i32::MIN,
            view_angles: Vec3([90.0, -90.0, 360.0]),
            movement: [f32::MAX, -f32::MAX, 0.0],
            units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
            buttons: buttons::ATTACK | buttons::TALK | buttons::ANY | buttons::JUMP,
            weapon: Some(own),
            ..UserCmd::default()
        };
        runtime.server.clients[clients[0].0 as usize]
            .command
            .server_time_ms = 999;
        let mut console = Console::new(Context::default()).unwrap();
        let maximum = console
            .cvars
            .bind("sv_maxclients", Context::default())
            .unwrap();
        console.cvars.force_write(maximum, "4").unwrap();
        let mut storage =
            ServiceStorage::load_native(&[(ModuleId(7), 0, RuleSetId::Quake3)], 0, &console.cvars)
                .unwrap();
        storage.bind_client_capacity(ModuleId(7), 4).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let base = if matches!(addresses, Addresses::Qvm { .. }) {
            0
        } else {
            1 << 40
        };
        let mut memory = ModuleMemory::load(base, 128, &[0xa5; 128]).unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        for (weapon, native_weapon) in [
            (Some(own), 7u8),
            (Some(foreign), 0),
            (None, 0),
            (Some(WeaponId(u32::MAX)), 0),
        ] {
            runtime.server.clients[target.0 as usize].command = UserCmd { weapon, ..command };
            let arguments = [0, base + 32 + if base == 0 { 128 } else { 0 }];
            {
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                assert_eq!(
                    Q3_SERVER
                        .invoke(
                            36,
                            &mut Invocation {
                                services: &mut services,
                                memory: &mut memory,
                                native_cvars: None,
                                native_resources: None,
                                native_entities: None,
                                native_surfaces: None,
                                native_command: None,
                                native_configs: None,
                                native_pmove: None,
                                native_player: None,
                                context: CallContext {
                                    module: ModuleId(7),
                                    ..context()
                                },
                                platform_time: EventTime(0),
                                command: Default::default(),
                                argument_selection:
                                    &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                                addresses,
                                arguments: &arguments,
                            },
                            &mut unknown
                        )
                        .map(scalar_import::word),
                    Ok(0)
                );
            }
            let mut expected = Vec::new();
            for word in [i32::MIN, 16384, 49152, 0, 2051] {
                expected.extend(word.to_le_bytes());
            }
            expected.extend([native_weapon, 127, 128, 127]);
            assert_eq!(memory.read(base + 32, 24).unwrap(), expected);
            assert_eq!(memory.read(base + 31, 1).unwrap(), &[0xa5]);
            assert_eq!(memory.read(base + 56, 1).unwrap(), &[0xa5]);
        }
        for (slot, address, expected) in [
            (3, base + 32, Ok(0)), // native-valid unused client, despite smaller common capacity
            (4, base + 32, Err(CallError::Entity)),
            (u64::MAX, base + 32, Err(CallError::Entity)),
            (0, base + 120, Err(CallError::Memory)),
        ] {
            memory.write(base, &[0xa5; 128]).unwrap();
            let arguments = [slot, address];
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            assert_eq!(
                Q3_SERVER
                    .invoke(
                        36,
                        &mut Invocation {
                            services: &mut services,
                            memory: &mut memory,
                            native_cvars: None,
                            native_resources: None,
                            native_entities: None,
                            native_surfaces: None,
                            native_command: None,
                            native_configs: None,
                            native_pmove: None,
                            native_player: None,
                            context: CallContext {
                                module: ModuleId(7),
                                ..context()
                            },
                            platform_time: EventTime(0),
                            command: Default::default(),
                            argument_selection:
                                &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                            addresses,
                            arguments: &arguments,
                        },
                        &mut unknown
                    )
                    .map(scalar_import::word),
                expected
            );
            if expected.is_ok() {
                assert_eq!(memory.read(base + 32, 24).unwrap(), &[0; 24]);
            } else {
                assert_eq!(memory.read(base, 128).unwrap(), &[0xa5; 128]);
            }
        }
        if matches!(addresses, Addresses::Qvm { .. }) {
            use qa_formats::program::qvm::Opcode::*;
            let mut vm = service_program::program(
                &[
                    (Enter, 64),
                    (Const, 0),
                    (Arg, 8),
                    (Const, 32),
                    (Arg, 12),
                    (Const, -37),
                    (Call, 0),
                    (Pop, 0),
                    (Const, 32),
                    (Load4, 0),
                    (Leave, 64),
                ],
                &[],
            );
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            assert_eq!(
                vm.call(
                    &mut QvmCalls {
                        services: &mut services,
                        table: &Q3_SERVER,
                        context: CallContext {
                            module: ModuleId(7),
                            ..context()
                        },
                        platform_time: EventTime(0),
                        command: Default::default(),
                        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                        unknown: &mut unknown,
                    },
                    [0; 10],
                    128,
                    false
                ),
                Ok(i32::MIN)
            );
        }
        assert_eq!(unknown.calls, 0);
        assert!(runtime.server.events.is_empty());
    }
}

#[test]
fn module_console_prints_do_not_wait_for_client_consumers() {
    use qa_core::{
        events::{OutputSubmission, OutputTarget},
        primitives::{ClientId, PrintKind},
    };
    for addresses in [
        Addresses::Qvm { mask: 127 },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::Microsoft,
        },
    ] {
        for (table, ordinal, native_only, null_client) in [
            (&Q3_SERVER, 0, false, false),
            (&Q3_CLIENT, 0, false, false),
            (&Q3_UI, 1, false, false),
            (&qa_compat::abi::Q2_RERELEASE, 1, true, false),
            (&qa_compat::abi::Q2_RERELEASE, 2, true, true),
        ] {
            if native_only && matches!(addresses, Addresses::Qvm { .. }) {
                continue;
            }
            let mut runtime = Runtime::load(1, []).unwrap();
            let remote = runtime
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let base = if matches!(addresses, Addresses::Qvm { .. }) {
                0
            } else {
                1 << 40
            };
            let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
            memory
                .write_string(base + 16, 32, b"console\x82\n")
                .unwrap();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let arguments = if null_client {
                [0, 2, base + 16]
            } else {
                [base + 16, 0, 0]
            };
            {
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                assert_eq!(
                    table
                        .invoke(
                            ordinal,
                            &mut Invocation {
                                services: &mut services,
                                memory: &mut memory,
                                native_cvars: None,
                                native_resources: None,
                                native_entities: None,
                                native_surfaces: None,
                                native_command: None,
                                native_configs: None,
                                native_pmove: None,
                                native_player: None,
                                context: context(),
                                platform_time: EventTime(0),
                                command: Default::default(),
                                argument_selection:
                                    &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                                addresses,
                                arguments: &arguments[..if null_client { 3 } else { 1 }],
                            },
                            &mut unknown
                        )
                        .map(scalar_import::word),
                    Ok(0)
                );
            }
            let events = &mut runtime.server.events;
            let mut client = events.batch(remote).unwrap();
            assert!(events.next(&mut client).is_none());
            let presentation = runtime.server.presentation;
            let mut local = events.batch(presentation).unwrap();
            let record = events.next(&mut local).unwrap();
            let FrameEvent::Print(print) = record.event else {
                panic!()
            };
            assert_eq!(print.kind, PrintKind::Console);
            assert_eq!(events.texts.get(print.text).unwrap(), b"console\x82\n");
            assert!(events.submit(presentation, record.sequence, OutputSubmission::BestEffort));
            assert!(events.is_empty());
        }
    }
}

#[test]
fn local_native_and_received_priority_prints_have_identical_hud_effects() {
    use qa_compat::abi::Q2_RERELEASE;
    use qa_core::{
        events::{OutputRecord, OutputSubmission, TextStore},
        primitives::{HudState, PrintKind},
    };
    use qa_network::{commands::packet::Protocol, outputs};
    for abi in [
        qa_platform::native::NativeAbi::SystemV,
        qa_platform::native::NativeAbi::Microsoft,
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let base = 1 << 40;
        let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
        memory
            .write_string(base + 16, 32, b"priority\x82\n")
            .unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        for level in 0u8..=255 {
            {
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                assert_eq!(
                    Q2_RERELEASE
                        .invoke(
                            0,
                            &mut Invocation {
                                services: &mut services,
                                memory: &mut memory,
                                native_cvars: None,
                                native_resources: None,
                                native_entities: None,
                                native_surfaces: None,
                                native_command: None,
                                native_configs: None,
                                native_pmove: None,
                                native_player: None,
                                context: context(),
                                platform_time: EventTime(0),
                                command: Default::default(),
                                argument_selection:
                                    &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                                addresses: Addresses::Native {
                                    pointer_bytes: 8,
                                    abi
                                },
                                arguments: &[u64::from(level), base + 16],
                            },
                            &mut unknown
                        )
                        .map(scalar_import::word),
                    Ok(0)
                );
            }
            let consumer = runtime.server.presentation;
            let mut seen = 0;
            qa_core::events::dispatch_consumer(
                &mut runtime.server.events,
                consumer,
                &mut |record: OutputRecord, texts: &mut TextStore| {
                    let FrameEvent::Print(print) = record.event else {
                        panic!("print record")
                    };
                    seen += 1;
                    let mut local = HudState::default();
                    assert!(qa_ui::hud::print(&mut local, texts, print, 1.0, 2.0, 3.0));
                    for protocol in [
                        Protocol::QuakeWorld28,
                        Protocol::Quake2_34,
                        Protocol::Quake2Repro1038,
                    ] {
                        let mut wire = [0; 64];
                        let len = outputs::print(
                            protocol,
                            print.kind,
                            print.level,
                            texts.get(print.text).unwrap(),
                            None,
                            &mut wire,
                        )
                        .unwrap();
                        let received = outputs::Prints::new(protocol, &wire[..len])
                            .next()
                            .unwrap()
                            .unwrap();
                        assert_eq!(print.kind, received.kind, "native level {level}");
                        assert_eq!(
                            received.level,
                            if print.kind == PrintKind::Center
                                && protocol != Protocol::Quake2Repro1038
                            {
                                None
                            } else {
                                Some(level)
                            }
                        );
                        assert_eq!(received.text, b"priority\x82\n");
                        let mut remote = HudState::default();
                        assert!(qa_ui::hud::print(
                            &mut remote,
                            texts,
                            qa_core::primitives::PrintEvent {
                                kind: received.kind,
                                ..print
                            },
                            1.0,
                            2.0,
                            3.0
                        ));
                        let expected_notify = level <= 1 || level == 3;
                        assert_eq!(local.notify[3].is_some(), expected_notify);
                        assert_eq!(remote.notify[3].is_some(), expected_notify);
                        assert_eq!(
                            local.notify[3].as_ref().map(|line| line.until),
                            remote.notify[3].as_ref().map(|line| line.until)
                        );
                        remote.clear_messages(texts);
                    }
                    assert_eq!(print.kind == PrintKind::Console, level == 2 || level > 5);
                    local.clear_messages(texts);
                    OutputSubmission::BestEffort
                },
            );
            assert_eq!(seen, 1);
            assert!(runtime.server.events.is_empty());
        }
        assert_eq!(unknown.calls, 0);
    }
}

#[test]
fn q3_game_and_ui_commands_insert_and_append_in_the_one_buffer() {
    use qa_core::text::FixedText;
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 1023 }),
        (
            1 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
        (
            1 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::Microsoft,
            },
        ),
    ] {
        for (table, ordinal) in [(&Q3_SERVER, 14), (&Q3_UI, 12)] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let mut ctx = context();
            ctx.console.source = RuleSetId::Quake3;
            services
                .commands
                .append("sensitivity 1\n", ctx.console)
                .unwrap();
            let mut memory = ModuleMemory::load(base, 1024, &[]).unwrap();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let args = [2, base + 16];
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: ctx,
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &args,
            };
            call.memory
                .write_string(base + 16, 64, b"sensitivity 3\n")
                .unwrap();
            assert_eq!(
                table
                    .invoke(ordinal, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(0)
            );
            let insert = [0xffff_ffff_0000_0001, base + 16];
            call.arguments = &insert;
            call.memory
                .write_string(base + 16, 64, b"sensitivity 2")
                .unwrap();
            assert_eq!(
                table
                    .invoke(ordinal, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(0)
            );
            let now = [0, base + 16];
            call.arguments = &now;
            let qa_compat::abi::ImportStep::Host {
                request:
                    qa_compat::abi::HostRequest::ExecuteNow {
                        context: found,
                        text,
                    },
                continuation,
            } = table.invoke(ordinal, &mut call, &mut unknown).unwrap()
            else {
                panic!("EXEC_NOW host request");
            };
            assert_eq!(found, ctx.console);
            assert_eq!(text, Some(base + 16));
            assert!(matches!(
                continuation.resume(&mut call, 9).unwrap(),
                qa_compat::abi::ImportStep::Return(0)
            ));
            let invalid = [3, base + 16];
            call.arguments = &invalid;
            assert_eq!(
                table
                    .invoke(ordinal, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Err(CallError::Text)
            );
            let mut line = FixedText::<64>::default();
            for expected in ["sensitivity 2", "sensitivity 1", "sensitivity 3"] {
                let (found, result) = call.services.commands.next_line(&mut line).unwrap();
                assert_eq!(found, ctx.console);
                assert_eq!(result, Ok(()));
                assert_eq!(line.as_str(), expected);
            }
            assert!(call.services.commands.is_empty());
            call.services
                .commands
                .append(
                    &"x".repeat(qa_console::command_buffer::CAPACITY - 20),
                    ctx.console,
                )
                .unwrap();
            call.memory
                .write_string(base + 16, 64, b"insert text that cannot fit now")
                .unwrap();
            call.arguments = &insert;
            assert_eq!(
                table
                    .invoke(ordinal, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Err(CallError::Text)
            );
            let mut full = FixedText::<{ qa_console::command_buffer::CAPACITY }>::default();
            assert_eq!(
                call.services.commands.next_line(&mut full).unwrap().1,
                Ok(())
            );
            assert_eq!(
                full.as_bytes().len(),
                qa_console::command_buffer::CAPACITY - 20
            );
            assert!(full.as_bytes().iter().all(|&b| b == b'x'));
            assert!(call.services.commands.is_empty());
            assert_eq!(unknown.calls, 0);
        }
    }
}

#[test]
fn q3_spawn_tokens_have_independent_module_cursors_and_bounded_raw_output() {
    use qa_formats::entities::EntitySyntax;
    use std::sync::Arc;
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let mut storage = ServiceStorage::load(
            &[(ModuleId(1), 0), (ModuleId(2), 0), (ModuleId(3), 0)],
            0,
            &console.cvars,
        )
        .unwrap();
        let source: Arc<[u8]> = b"/* comment */ { \"classname\" \"worldspawn\" \"empty\" \"\" \"raw\" \"\x82\\n\" }\0ignored".as_slice().into();
        for module in [ModuleId(1), ModuleId(2)] {
            storage
                .bind_entity_text(module, source.clone(), EntitySyntax::Quake3)
                .unwrap();
        }
        assert_eq!(Arc::strong_count(&source), 3);
        let mut scratch = runtime.geometry.scratch();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let arguments = [base + 16, 64];
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses,
            arguments: &arguments,
        };
        for expected in [
            b"{".as_slice(),
            b"classname",
            b"worldspawn",
            b"empty",
            b"",
            b"raw",
            b"\x82\\n",
            b"}",
        ] {
            for module in [ModuleId(1), ModuleId(2)] {
                call.context.module = module;
                assert_eq!(
                    Q3_SERVER
                        .invoke(37, &mut call, &mut unknown)
                        .map(scalar_import::word),
                    Ok(1)
                );
                assert_eq!(call.memory.cstring(base + 16).unwrap(), expected);
            }
        }
        for module in [ModuleId(1), ModuleId(2), ModuleId(3)] {
            call.context.module = module;
            assert_eq!(
                Q3_SERVER
                    .invoke(37, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(0)
            );
            assert_eq!(call.memory.cstring(base + 16).unwrap(), b"");
        }
        call.services
            .storage
            .bind_entity_text(ModuleId(1), source.clone(), EntitySyntax::Quake3)
            .unwrap();
        call.context.module = ModuleId(1);
        let short = [base + 16, 1];
        call.arguments = &short;
        assert_eq!(
            Q3_SERVER
                .invoke(37, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(1)
        );
        assert_eq!(call.memory.cstring(base + 16).unwrap(), b"");
        let invalid = [base + 510, 64];
        call.arguments = &invalid;
        assert_eq!(
            Q3_SERVER
                .invoke(37, &mut call, &mut unknown)
                .map(scalar_import::word),
            Err(CallError::Memory)
        );
        let zero = [base + 16, 0];
        call.arguments = &zero;
        assert_eq!(
            Q3_SERVER
                .invoke(37, &mut call, &mut unknown)
                .map(scalar_import::word),
            Err(CallError::Text)
        );
        call.arguments = &arguments;
        assert_eq!(
            Q3_SERVER
                .invoke(37, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(1)
        );
        assert_eq!(call.memory.cstring(base + 16).unwrap(), b"classname");
        assert_eq!(unknown.calls, 0);
    }
}

#[test]
fn numeric_cvar_imports_share_views_and_keep_signed_native_results() {
    use qa_formats::program::qvm::Opcode::*;
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 255 }),
        (
            1 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
        (
            1 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::Microsoft,
            },
        ),
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let view = console
            .cvars
            .register("_qa_module_number", Some("1"), 0, context().console)
            .unwrap();
        let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut memory = ModuleMemory::load(base, 256, &[]).unwrap();
        memory
            .write_string(base + 16, 64, b"_qa_module_number")
            .unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        for (text, integer, float) in [
            ("3.75", 3i32, 3.75f32),
            ("-7.5", -7, -7.5),
            ("1e3", 1, 1000.0),
            ("no digits", 0, 0.0),
            (" \t-42tail", -42, -42.0),
            ("2147483647", i32::MAX, 2147483648.0),
            ("-2147483648", i32::MIN, -2147483648.0),
        ] {
            console.cvars.force_write(view, text).unwrap();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let arguments = [base + 16];
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: context(),
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &arguments,
            };
            assert_eq!(
                Q3_SERVER
                    .invoke(6, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(integer as i64 as u64)
            );
            assert_eq!(
                Q3_UI
                    .invoke(4, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(u64::from(float.to_bits()))
            );
            if base == 0 {
                let mut vm = service_program::program(
                    &[
                        (Enter, 64),
                        (Const, 16),
                        (Arg, 8),
                        (Const, -7),
                        (Call, 0),
                        (Leave, 64),
                    ],
                    memory.read(base, 80).unwrap(),
                );
                let mut calls = QvmCalls {
                    services: &mut services,
                    table: &Q3_SERVER,
                    context: context(),
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    unknown: &mut unknown,
                };
                assert_eq!(vm.call(&mut calls, [0; 10], 128, false), Ok(integer));
            }
        }
        memory
            .write_string(base + 16, 64, b"_qa_missing_number")
            .unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let arguments = [base + 16];
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses,
            arguments: &arguments,
        };
        assert_eq!(
            Q3_SERVER
                .invoke(6, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert!(
            services
                .cvars
                .bind("_qa_missing_number", context().console)
                .is_none()
        );
        assert_eq!(unknown.calls, 0);
        assert!(runtime.server.events.is_empty());
    }
}

#[test]
fn q3_module_setters_create_cvars_and_force_readonly_and_latched_values() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        for (table, ordinal) in [(&Q3_SERVER, 5), (&Q3_CLIENT, 5), (&Q3_UI, 3)] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut call_context = context();
            call_context.console.source = RuleSetId::Quake3;
            let ctx = call_context.console;
            let foreign_context = Context {
                source: RuleSetId::Quake2,
                ..ctx
            };
            let foreign = console
                .cvars
                .register("bad;name", Some("q2"), 0, foreign_context)
                .unwrap();
            let gamma = console.cvars.bind("gamma", ctx).unwrap();
            console.cvars.set_text(gamma.canonical(), "0").unwrap();
            assert!(console.cvars.read(gamma).is_err());
            let readonly = console
                .cvars
                .register("_qa_module_rom", Some("1"), 64, ctx)
                .unwrap();
            let latched = console
                .cvars
                .register("_qa_module_latch", Some("1"), 32, ctx)
                .unwrap();
            assert!(console.cvars.write(readonly, "2").is_err());
            console.cvars.set_latch_active(latched.canonical(), true);
            console.cvars.write(latched, "2").unwrap();
            assert_eq!(
                console.cvars.latched(latched).unwrap().unwrap().as_str(),
                "2"
            );
            let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            for (name, value, lookup) in [
                ("_qa_module_new", "17", "_qa_module_new"),
                ("_qa_module_rom", "4", "_qa_module_rom"),
                ("_qa_module_latch", "3", "_qa_module_latch"),
                ("_qa_module_new", "18", "_qa_module_new"),
                ("gamma", "1.000000", "gamma"),
                ("bad;name", "3", "BADNAME"),
                ("bad\\name", "4", "BADNAME"),
                ("bad\"name", "5", "BADNAME"),
            ] {
                memory.write_string(base + 16, 64, name.as_bytes()).unwrap();
                memory
                    .write_string(base + 96, 64, value.as_bytes())
                    .unwrap();
                let arguments = [base + 16, base + 96];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    native_pmove: None,
                    native_player: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(
                    table
                        .invoke(ordinal, &mut call, &mut unknown)
                        .map(scalar_import::word),
                    Ok(0)
                );
                let view = services.cvars.bind(lookup, ctx).unwrap();
                assert_eq!(services.cvars.read(view).unwrap().as_str(), value);
            }
            assert_eq!(services.cvars.read(foreign).unwrap().as_str(), "q2");
            assert!(!services.server.events.is_empty());
            assert!(services.cvars.latched(latched).unwrap().is_none());
            assert_eq!(services.cvars.flags(readonly) & 64, 64);
            assert_eq!(unknown.calls, 0);
        }
    }
}

#[test]
fn q3_null_setters_reset_existing_views_and_leave_missing_names_absent() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        for (table, ordinal) in [(&Q3_SERVER, 5), (&Q3_CLIENT, 5), (&Q3_UI, 3)] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut call_context = context();
            call_context.console.source = RuleSetId::Quake3;
            let ctx = call_context.console;
            let readonly = console
                .cvars
                .register("_qa_reset_rom", Some("1"), 64, ctx)
                .unwrap();
            let latched = console
                .cvars
                .register("_qa_reset_latch", Some("1"), 32, ctx)
                .unwrap();
            console.cvars.force_write(readonly, "2").unwrap();
            console.cvars.force_write(latched, "2").unwrap();
            console.cvars.set_latch_active(latched.canonical(), true);
            console.cvars.write(latched, "3").unwrap();
            let sensitivity = console.cvars.bind("sensitivity", ctx).unwrap();
            console.cvars.force_write(sensitivity, "9").unwrap();
            let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
            // Zero in a QVM is NULL even when its data at offset zero is not empty.
            memory.write(base, b"7\0").unwrap();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let before = services.cvars.entries().count();
            for (name, expected) in [
                ("_qa_reset_missing", None),
                ("_qa_reset_rom", Some("1")),
                ("_qa_reset_latch", Some("1")),
                ("sensitivity", Some("5")),
            ] {
                memory.write_string(base + 32, 64, name.as_bytes()).unwrap();
                let arguments = [base + 32, 0];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    native_pmove: None,
                    native_player: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(
                    table
                        .invoke(ordinal, &mut call, &mut unknown)
                        .map(scalar_import::word),
                    Ok(0)
                );
                let actual = services
                    .cvars
                    .bind(name, ctx)
                    .map(|view| services.cvars.read(view).unwrap().as_str().to_owned());
                assert_eq!(actual.as_deref(), expected);
            }
            assert_eq!(services.cvars.entries().count(), before);
            assert!(services.cvars.latched(latched).unwrap().is_none());
            assert_eq!(services.cvars.flags(readonly) & 64, 64);
            assert_eq!(unknown.calls, 0);
            // The same native value leaves an outstanding latch untouched.
            services.cvars.write(latched, "2").unwrap();
            memory
                .write_string(base + 32, 64, b"_qa_reset_latch")
                .unwrap();
            memory.write_string(base + 96, 64, b"1").unwrap();
            let arguments = [base + 32, base + 96];
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: call_context,
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &arguments,
            };
            assert_eq!(
                table
                    .invoke(ordinal, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(0)
            );
            assert_eq!(
                services.cvars.latched(latched).unwrap().unwrap().as_str(),
                "2"
            );
            if base == 0 {
                // A nonzero QVM pointer reads the text instead of resetting.
                memory.write_string(32, 64, b"sensitivity").unwrap();
                let arguments = [32, 512];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    native_pmove: None,
                    native_player: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: Default::default(),
                    argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(
                    table
                        .invoke(ordinal, &mut call, &mut unknown)
                        .map(scalar_import::word),
                    Ok(0)
                );
                assert_eq!(services.cvars.read(sensitivity).unwrap().as_str(), "7");
            }
        }
    }
}

#[test]
fn q3_ui_create_and_reset_preserve_native_flags_protection_and_latches() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let ctx = context();
        let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut invoke = |services: &mut qa_compat::services::EngineServices<'_>,
                          ordinal,
                          name: &str,
                          value: Option<&str>,
                          flags: u32| {
            memory
                .write_string(base + 32, 128, name.as_bytes())
                .unwrap();
            let value_pointer = if let Some(value) = value {
                memory
                    .write_string(base + 192, 128, value.as_bytes())
                    .unwrap();
                base + 192
            } else {
                0
            };
            let arguments = [base + 32, value_pointer, u64::from(flags)];
            let mut call = Invocation {
                services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: ctx,
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &arguments,
            };
            assert_eq!(
                Q3_UI
                    .invoke(ordinal, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(0)
            );
        };
        let before = services.cvars.entries().count();
        invoke(&mut services, 7, "_qa_ui_missing", None, 0);
        assert_eq!(services.cvars.entries().count(), before);
        for (name, flags) in [
            ("_qa_ui_rom", 64),
            ("_qa_ui_init", 16),
            ("_qa_ui_cheat", 512),
        ] {
            invoke(&mut services, 8, name, Some("1"), flags);
            invoke(&mut services, 3, name, Some("2"), 0);
            let view = services.cvars.bind(name, ctx.console).unwrap();
            services
                .cvars
                .set_runtime_policy(qa_console::cvars::RuntimePolicy {
                    initialized: true,
                    ..services.cvars.runtime_policy()
                });
            let cheats = services.cvars.bind("sv_cheats", ctx.console).unwrap();
            services.cvars.force_write(cheats, "0").unwrap();
            invoke(&mut services, 7, name, None, 0);
            assert_eq!(services.cvars.read(view).unwrap().as_str(), "2");
            assert_eq!(services.cvars.flags(view) & flags, flags);
            invoke(&mut services, 3, name, None, 0);
            assert_eq!(services.cvars.read(view).unwrap().as_str(), "1");
        }
        invoke(&mut services, 8, "_qa_ui_latch", Some("1"), 32);
        invoke(&mut services, 3, "_qa_ui_latch", Some("2"), 0);
        let view = services.cvars.bind("_qa_ui_latch", ctx.console).unwrap();
        services.cvars.set_latch_active(view.canonical(), true);
        invoke(&mut services, 7, "_qa_ui_latch", None, 0);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "2");
        assert_eq!(services.cvars.latched(view).unwrap().unwrap().as_str(), "1");
        invoke(&mut services, 3, "_qa_ui_latch", None, 0);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "1");
        assert!(services.cvars.latched(view).unwrap().is_none());
        invoke(&mut services, 8, "_qa_ui_existing", Some("5"), 0);
        invoke(&mut services, 3, "_qa_ui_existing", Some("8"), 0);
        let view = services.cvars.bind("_qa_ui_existing", ctx.console).unwrap();
        let entries = services.cvars.entries().count();
        invoke(&mut services, 8, "_QA_UI_EXISTING", Some("99"), 64);
        assert_eq!(services.cvars.entries().count(), entries);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "8");
        assert_eq!(services.cvars.flags(view) & 64, 64);
        invoke(&mut services, 3, "_qa_ui_existing", None, 0);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "5");
        assert_eq!(unknown.calls, 0);
        assert_eq!(services.server.events.len(), 3);
        let events = &services.server.events;
        let mut batch = events.batch(services.server.presentation).unwrap();
        for expected in [
            b"_qa_ui_rom is read only.\n".as_slice(),
            b"_qa_ui_init is write protected.\n".as_slice(),
            b"_qa_ui_cheat is cheat protected.\n".as_slice(),
        ] {
            let FrameEvent::Print(print) = events.next(&mut batch).unwrap().event else {
                panic!("print");
            };
            assert_eq!(events.texts.get(print.text).unwrap(), expected);
        }
    }
}

#[test]
fn q3_ui_numeric_setter_preserves_native_formatting_and_forced_writes() {
    // Cvar_SetValue's %i/%f branches and val[32], checked against libc snprintf.
    let values: [(u32, &str); 15] = [
        (0x00000000, "0"),
        (0x80000000, "0"),
        (0x3f800000, "1"),
        (0xc0000000, "-2"),
        (0x3f000000, "0.500000"),
        (0xbfa00000, "-1.250000"),
        (0x3f9e0651, "1.234568"),
        (0x4effffff, "2147483520"),
        (0x4f000000, "2147483648.000000"),
        (0xcf000000, "-2147483648"),
        (0x7149f2ca, "1000000015047466219876688855040"),
        (0x7f800000, "inf"),
        (0xff800000, "-inf"),
        (0x7fc00000, "nan"),
        (0xffc00000, "-nan"),
    ];
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let ctx = context();
        let rom = console
            .cvars
            .register("_qa_ui_number_rom", Some("1"), 64, ctx.console)
            .unwrap();
        let latch = console
            .cvars
            .register("_qa_ui_number_latch", Some("1"), 32, ctx.console)
            .unwrap();
        console.cvars.set_latch_active(latch.canonical(), true);
        console.cvars.write(latch, "3").unwrap();
        let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        for (name, bits, expected) in values
            .iter()
            .map(|&(bits, text)| ("_qa_ui_number", bits, text))
            .chain([
                ("_qa_ui_number_rom", 0x40000000, "2"),
                ("_qa_ui_number_latch", 0x40000000, "2"),
            ])
        {
            memory
                .write_string(base + 32, 128, name.as_bytes())
                .unwrap();
            let arguments = [base + 32, 0xfedcba9800000000 | u64::from(bits)];
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: ctx,
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &arguments,
            };
            assert_eq!(
                Q3_UI
                    .invoke(6, &mut call, &mut unknown)
                    .map(scalar_import::word),
                Ok(0)
            );
            let view = services.cvars.bind(name, ctx.console).unwrap();
            assert_eq!(services.cvars.read(view).unwrap().as_str(), expected);
        }
        assert_eq!(services.cvars.flags(rom) & 64, 64);
        assert!(services.cvars.latched(latch).unwrap().is_none());
        assert_eq!(services.server.events.len(), 1);
        assert_eq!(unknown.calls, 0);
        let events = &services.server.events;
        let mut batch = events.batch(services.server.presentation).unwrap();
        let FrameEvent::Print(print) = events.next(&mut batch).unwrap().event else {
            panic!("print");
        };
        assert_eq!(
            events.texts.get(print.text).unwrap(),
            b"Com_sprintf: overflow of 38 in 32\n"
        );
    }
}

#[test]
fn native_syscall_memory_width_stays_separate_from_child_runtime_imports() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let base = 1u64 << 40;
    let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
    memory.write(base + 16, b"\xffa\0").unwrap();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let mut call = Invocation {
        services: &mut services,
        memory: &mut memory,
        native_cvars: None,
        native_resources: None,
        native_entities: None,
        native_surfaces: None,
        native_command: None,
        native_configs: None,
        native_pmove: None,
        native_player: None,
        context: context(),
        platform_time: EventTime(0),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        addresses: Addresses::Native {
            pointer_bytes: 8,
            abi: qa_platform::native::NativeAbi::SystemV,
        },
        arguments: &[base + 32, base + 16, (1u64 << 32) + 1],
    };
    assert_eq!(
        Q3_SERVER
            .invoke(101, &mut call, &mut unknown)
            .map(scalar_import::word),
        Ok(0)
    );
    assert_eq!(call.memory.read(base + 32, 1).unwrap(), b"\xff");
    assert_eq!(
        Q3_SERVER
            .invoke(qa_platform::native::runtime::FIRST, &mut call, &mut unknown)
            .map(scalar_import::word),
        Ok(0)
    );
    assert_eq!(unknown.calls, 1);
}

#[test]
fn original_float_syscalls_keep_their_declared_bits() {
    use qa_compat::abi::QUAKEC;
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut memory = ModuleMemory::load(0, 64, &[]).unwrap();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let mut invoke = |table: &qa_compat::abi::CallTable, number, addresses, arguments: &[u64]| {
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses,
            arguments,
        };
        table
            .invoke(number, &mut call, &mut unknown)
            .map(scalar_import::word)
    };
    let functions: [(u32, &qa_compat::abi::CallTable, u32, fn(f64, f64) -> f64); 7] = [
        (6, &Q3_SERVER, 103, |x, _| x.sin()),
        (7, &Q3_SERVER, 104, |x, _| x.cos()),
        (8, &Q3_SERVER, 105, f64::atan2),
        (9, &Q3_SERVER, 106, |x, _| x.sqrt()),
        (10, &Q3_SERVER, 110, |x, _| x.floor()),
        (11, &Q3_SERVER, 111, |x, _| x.ceil()),
        (12, &Q3_CLIENT, 111, |x, _| x.acos()),
    ];
    for x in [
        -0.0,
        0.0,
        -0.5,
        0.5,
        1.0,
        -12345.6789,
        1e30,
        f64::INFINITY,
        f64::NAN,
    ] {
        for &(_, table, original, operation) in &functions {
            let x = x as f32;
            let args = [
                0xaabbccdd00000000 | u64::from(x.to_bits()),
                u64::from((-2.25f32).to_bits()),
            ];

            let actual =
                invoke(table, original, Addresses::Qvm { mask: 63 }, &args).unwrap() as u32;
            let expected = operation(f64::from(x), -2.25) as f32;
            if expected.is_nan() {
                assert!(f32::from_bits(actual).is_nan());
            } else {
                assert_eq!(actual, expected.to_bits(), "original math {original}({x})");
            }
        }
    }
    for bits in [0x80000000u32, 0xbf000000, 0xffc12345] {
        let args = [u64::from(bits)];

        assert_eq!(
            invoke(
                &QUAKEC,
                43,
                Addresses::Native {
                    pointer_bytes: 8,
                    abi: qa_platform::native::NativeAbi::SystemV
                },
                &args
            ),
            Ok(u64::from(bits & 0x7fffffff))
        );
    }
    assert_eq!(unknown.calls, 0);
}

#[test]
fn native_addresses_role_ordinals_cvar_conversion_and_byte_strings_use_existing_services() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 8)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let base = 1u64 << 40;
    let mut memory = ModuleMemory::load(base, 512, b"\0").unwrap();
    memory.write(base + 16, b"cg_fov\0").unwrap();
    memory.write(base + 32, b"1e3\0").unwrap();
    memory.write(base + 48, b"\x82native\n\0").unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime((u64::from(u32::MAX) + 8) * 1_000_000),
            command: qa_console::command_text::Arguments::from_command(
                qa_core::primitives::CommandArgs {
                    argv: &[b"native", b"arg"],
                    raw: &[],
                },
            ),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses: Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
            arguments: &[base + 16, base + 32],
        };
        assert_eq!(
            Q3_SERVER
                .invoke(5, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        let args1 = [base + 16];
        call.arguments = &args1;
        // atoi is not a float-to-integer cast: the same text has value 1000.
        assert_eq!(
            Q3_SERVER
                .invoke(6, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(1)
        );
        assert_eq!(
            Q3_UI
                .invoke(4, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(1000.0f32.to_bits() as u64)
        );
        assert_eq!(
            Q3_SERVER
                .invoke(2, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(7)
        );
        let args2 = [3, base + 48];
        call.arguments = &args2;
        assert_eq!(
            Q3_SERVER
                .invoke(18, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        let args3 = [3, base + 80, 4];
        call.arguments = &args3;
        assert_eq!(
            Q3_SERVER
                .invoke(19, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(call.memory.read(base + 80, 4).unwrap(), b"\x82na\0");
        let args4 = [u32::MAX as u64, base + 80, 4];
        call.arguments = &args4;
        assert_eq!(
            Q3_UI
                .invoke(11, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(call.memory.read(base + 80, 1).unwrap(), b"\0");
        let args5 = [base + 48];
        call.arguments = &args5;
        assert_eq!(
            Q3_UI
                .invoke(1, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(
            Q3_UI
                .invoke(0, &mut call, &mut unknown)
                .map(scalar_import::word),
            Err(CallError::Aborted)
        );
        assert_eq!(
            Q3_SERVER
                .invoke(1234, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(
            Q3_SERVER
                .invoke(1234, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!(
            Q3_SERVER
                .invoke(1235, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        assert_eq!((unknown.calls, unknown.capacity_drops), (3, 1));
        assert_eq!(unknown.last, Some(1235));
        let mut batch = services
            .server
            .events
            .batch(services.server.presentation)
            .unwrap();
        let mut prints = Vec::new();
        while let Some(record) = services.server.events.next(&mut batch) {
            if let FrameEvent::Print(p) = record.event {
                prints.push(services.server.events.texts.get(p.text).unwrap().to_vec());
            }
        }
        assert_eq!(
            prints,
            vec![
                b"\x82native\n".to_vec(),
                b"\x82native\n".to_vec(),
                b"module 1: unknown system call 1234\n".to_vec()
            ]
        );
    }
}

#[test]
fn real_qvm_calls_write_cvars_print_and_append_into_the_existing_console() {
    let mut vm = service_program::qvm();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(4).unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut calls = QvmCalls {
            services: &mut services,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(90_000_000),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [0; 10], 1000, false), Ok(105));
        assert_eq!(unknown.calls, 0);
    }
    console.execute_frame(&mut runtime);
    assert_eq!(
        console
            .cvars
            .numeric(
                console
                    .cvars
                    .bind("sensitivity", Context::default())
                    .unwrap()
            )
            .unwrap(),
        7.0
    );
    assert_eq!(
        console
            .cvars
            .value_in(console.cvars.find("cg_fov").unwrap(), RuleSetId::Quake3),
        105.0
    );
}

#[test]
fn server_and_client_console_imports_keep_their_different_arguments() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(4).unwrap();
    let mut memory = ModuleMemory::load(0, 128, b"\0sensitivity 6\n\0").unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            native_pmove: None,
            native_player: None,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            addresses: Addresses::Qvm { mask: 127 },
            arguments: &[1],
        };
        assert_eq!(
            Q3_CLIENT
                .invoke(14, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        let args6 = [2, 1];
        call.arguments = &args6;
        assert_eq!(
            Q3_SERVER
                .invoke(14, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(0)
        );
        let args7 = [0, 1];
        call.arguments = &args7;
        let qa_compat::abi::ImportStep::Host {
            request:
                qa_compat::abi::HostRequest::ExecuteNow {
                    context: found,
                    text,
                },
            continuation,
        } = Q3_SERVER.invoke(14, &mut call, &mut unknown).unwrap()
        else {
            panic!("EXEC_NOW host request");
        };
        assert_eq!(found, call.context.console);
        assert_eq!(text, Some(1));
        assert_eq!(
            call.memory.cstring(text.unwrap()).unwrap(),
            b"sensitivity 6\n"
        );
        assert!(matches!(
            continuation.resume(&mut call, u64::MAX).unwrap(),
            qa_compat::abi::ImportStep::Return(0)
        ));
        assert_eq!(call.memory.cstring(1).unwrap(), b"sensitivity 6\n");
        let mut queued = qa_core::text::FixedText::<64>::default();
        for _ in 0..2 {
            let (context, result) = call.services.commands.next_line(&mut queued).unwrap();
            assert_eq!(context, found);
            assert_eq!(result, Ok(()));
            assert_eq!(queued.as_bytes(), b"sensitivity 6");
        }
        assert!(call.services.commands.is_empty());
        let args8 = [1.25f32.to_bits() as u64];
        call.arguments = &args8;
        assert_eq!(
            Q3_SERVER
                .invoke(110, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(1.0f32.to_bits() as u64)
        );
        assert_eq!(
            Q3_CLIENT
                .invoke(107, &mut call, &mut unknown)
                .map(scalar_import::word),
            Ok(1.0f32.to_bits() as u64)
        );
    }
}

#[test]
fn cached_native_sound_ordinal_rebinds_on_config_and_resource_changes() {
    use qa_compat::services::ENGINE_CALLS;
    use qa_core::primitives::{Pcm, PcmChannels};
    use std::num::NonZeroU32;
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = qa_audio::Bank::load(
        rate,
        ["sound/world/amb.wav", "sound/world/new.wav"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![1024],
                        loop_start: None,
                        loop_end: None,
                        loop_invalid: false,
                    },
                )
            })
            .collect(),
    )
    .unwrap();
    let ambient = bank.find("sound/world/amb.wav").unwrap();
    let new = bank.find("sound/world/new.wav").unwrap();
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let module = ModuleId(1);
    let other = ModuleId(2);
    let mut storage = ServiceStorage::load(&[(module, 8), (other, 8)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    services.sound_bank = Some(&bank);
    let range = IndexSpan { first: 2, count: 4 };
    assert!(!range.contains(1));
    assert!(range.contains(2));
    assert!(range.contains(5));
    assert!(!range.contains(6));
    assert_eq!(range.get(0), Some(2));
    assert_eq!(range.get(3), Some(5));
    assert_eq!(range.get(4), None);
    assert_eq!(
        IndexSpan {
            first: u32::MAX,
            count: 2
        }
        .get(usize::MAX),
        None
    );
    for bad in [
        IndexSpan { first: 2, count: 0 },
        IndexSpan { first: 6, count: 3 },
        IndexSpan {
            first: u32::MAX,
            count: 2,
        },
    ] {
        assert!(matches!(
            services.storage.bind_sound_bank(module, bad, &bank),
            Err(qa_compat::services::CallError::ConfigString)
        ));
    }
    services.configstring(module, 3, b"world/amb.wav").unwrap();
    services
        .storage
        .bind_sound_bank(module, range, &bank)
        .unwrap();
    services.configstring(other, 3, b"world/amb.wav").unwrap();
    services
        .storage
        .bind_sound_bank(other, range, &bank)
        .unwrap();
    let read = |services: &mut qa_compat::services::EngineServices<'_>, index| {
        (ENGINE_CALLS.sound_id)(services, module, range, index)
    };
    assert_eq!(read(&mut services, 1), Ok(Some(ambient)));
    let revision = services.storage.configstring(module, 3).unwrap().1;
    services.configstring(module, 3, b"world/amb.wav").unwrap();
    assert_eq!(read(&mut services, 1), Ok(Some(ambient)));
    assert_eq!(
        services.storage.configstring(module, 3).unwrap().1,
        revision
    );
    services.configstring(module, 3, b"world/new.wav").unwrap();
    assert_eq!(read(&mut services, 1), Ok(Some(new)));
    assert_eq!(
        (ENGINE_CALLS.sound_id)(&mut services, other, range, 1),
        Ok(Some(ambient))
    );
    services
        .configstring(module, 3, b"#sound/world/amb.wav")
        .unwrap();
    assert_eq!(read(&mut services, 1), Ok(Some(ambient)));
    for text in [b"missing.wav".as_slice(), b"*death.wav", b""] {
        services.configstring(module, 3, text).unwrap();
        assert_eq!(read(&mut services, 1), Ok(None));
    }
    let ordinal =
        (ENGINE_CALLS.resource_index)(&mut services, module, range, b"world/new.wav").unwrap();
    assert_eq!(ordinal, 1);
    assert_eq!(read(&mut services, ordinal as i32), Ok(Some(new)));
    assert_eq!(
        (ENGINE_CALLS.resource_index)(&mut services, module, range, b"world/amb.wav"),
        Ok(2)
    );
    assert_eq!(read(&mut services, 2), Ok(Some(ambient)));
    // Ordinary configstrings must keep their raw bytes and never enter path lookup.
    services.configstring(module, 0, &[0xff, 0xfe]).unwrap();
    assert_eq!(
        services.storage.configstring(module, 0).unwrap().0,
        &[0xff, 0xfe]
    );
    services.configstring(module, 3, b"").unwrap();
    assert_eq!(read(&mut services, 1), Ok(None));
    assert_eq!(read(&mut services, 0), Ok(None));
    assert!(read(&mut services, -1).is_err());
    assert!(read(&mut services, 4).is_err());
}
