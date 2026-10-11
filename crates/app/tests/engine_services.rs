use qa_app::Runtime;
use qa_compat::services::{CallContext, CallError, ENGINE_CALLS, ResourceRange, ServiceStorage};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::ThinkTime;
use qa_core::{
    events::FrameEvent,
    primitives::{ModuleId, PrintKind, RuleSetId},
};
use qa_world::{
    area::{LinkFlags, LinkOrder},
    entities::AllocationPolicy,
};

fn context(module: u16, rules: RuleSetId, order: LinkOrder) -> CallContext {
    CallContext {
        module: ModuleId(module),
        clock: ThinkTime::Seconds(2.0),
        console: Context {
            source: rules,
            ..Context::default()
        },
        allocation: AllocationPolicy::EDICT,
        link_order: order,
    }
}

#[test]
fn module_messages_are_independent_and_rejected_appends_preserve_the_prefix() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage =
        ServiceStorage::load(&[(ModuleId(1), 0), (ModuleId(2), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    (ENGINE_CALLS.message_bits)(&mut services, ModuleId(1), 0x1234, 16).unwrap();
    (ENGINE_CALLS.message_string)(&mut services, ModuleId(2), b"\xff\x80").unwrap();
    let oversized = vec![0xaa; 0x8000];
    assert_eq!(
        (ENGINE_CALLS.message_string)(&mut services, ModuleId(1), &oversized),
        Err(CallError::Capacity)
    );
    assert_eq!(
        services.storage.message(ModuleId(1)).unwrap(),
        &[0x34, 0x12]
    );
    assert_eq!(
        services.storage.message(ModuleId(2)).unwrap(),
        &[0xff, 0x80, 0]
    );
    (ENGINE_CALLS.message_string)(&mut services, ModuleId(1), &oversized[..0x8000 - 3]).unwrap();
    let full = services.storage.message(ModuleId(1)).unwrap().to_vec();
    assert_eq!(full.len(), 0x8000);
    assert_eq!(
        (ENGINE_CALLS.message_bits)(&mut services, ModuleId(1), 1, 8),
        Err(CallError::Capacity)
    );
    assert_eq!(services.storage.message(ModuleId(1)).unwrap(), full);
    assert_eq!(
        (ENGINE_CALLS.message_bits)(&mut services, ModuleId(3), 1, 8),
        Err(CallError::Memory)
    );
}

#[test]
fn modules_share_entity_lifetimes_cvars_command_buffer_and_byte_exact_output() {
    let mut runtime = Runtime::load(4, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage =
        ServiceStorage::load(&[(ModuleId(1), 8), (ModuleId(2), 8)], 4, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let q1 = context(1, RuleSetId::Quake, LinkOrder::Tail);
    let q3 = context(2, RuleSetId::Quake3, LinkOrder::Head);
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let first = (ENGINE_CALLS.spawn)(&mut services, q1).unwrap();
        let second = (ENGINE_CALLS.spawn)(&mut services, q3).unwrap();
        assert_eq!(
            services.server.entities.columns.owner[first.slot as usize],
            ModuleId(1)
        );
        assert_eq!(
            services.server.entities.columns.owner[second.slot as usize],
            ModuleId(2)
        );
        assert!((ENGINE_CALLS.link)(&mut services, q1, first, LinkFlags::SOLID).unwrap());
        assert!((ENGINE_CALLS.link)(&mut services, q3, second, LinkFlags::SOLID).unwrap());
        // Explicit relinking is never turned into an unchanged-body commit.
        assert!((ENGINE_CALLS.link)(&mut services, q1, first, LinkFlags::SOLID).unwrap());
        (ENGINE_CALLS.free)(&mut services, q1, first).unwrap();
        assert_eq!(
            (ENGINE_CALLS.link)(&mut services, q1, first, LinkFlags::SOLID),
            Err(CallError::Entity)
        );
        let view = services.cvars.bind("fov", q1.console).unwrap();
        (ENGINE_CALLS.cvar_set)(&mut services, view, "103").unwrap();
        let alias = services.cvars.bind("cg_fov", q1.console).unwrap();
        assert_eq!(services.cvars.numeric(alias).unwrap(), 103.0);
        let registered = (ENGINE_CALLS.cvar_register)(
            &mut services,
            q1.console,
            "_qa_shared_module",
            Some("12"),
            0,
        )
        .unwrap();
        let other = (ENGINE_CALLS.cvar_register)(
            &mut services,
            q3.console,
            "_QA_SHARED_MODULE",
            Some("19"),
            0,
        )
        .unwrap();
        assert_eq!(registered.canonical(), other.canonical());
        (ENGINE_CALLS.cvar_set)(&mut services, registered, "27").unwrap();
        assert_eq!(services.cvars.numeric(other).unwrap(), 27.0);
        assert_eq!(
            (ENGINE_CALLS.cvar_register)(&mut services, q3.console, "", Some("0"), 0).unwrap_err(),
            CallError::Cvar
        );
        (ENGINE_CALLS.command)(&mut services, q3.console, "sensitivity 5 extra\n").unwrap();
        (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 3, b"\x80native").unwrap();
        (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 3, b"\x80native").unwrap();
        assert_eq!(
            services.storage.configstring(ModuleId(1), 3).unwrap(),
            (&b"\x80native"[..], 1)
        );
        assert_eq!(
            services.storage.configstring(ModuleId(2), 3).unwrap(),
            (&b""[..], 0)
        );
        assert_eq!(
            (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), 8, b"bad"),
            Err(CallError::ConfigString)
        );
        (ENGINE_CALLS.print)(
            &mut services,
            None,
            PrintKind::Console,
            PrintKind::Console.default_level(),
            b"\x82native\n",
        )
        .unwrap();
        let mut batch = services
            .server
            .events
            .batch(services.server.presentation)
            .unwrap();
        let record = services.server.events.next(&mut batch).unwrap();
        let FrameEvent::Print(print) = record.event else {
            panic!("print");
        };
        assert_eq!(
            services.server.events.texts.get(print.text),
            Some(&b"\x82native\n"[..])
        );
    }
    assert!(!console.idle());
    console.execute_frame(&mut runtime);
    let view = console.cvars.bind("sensitivity", q3.console).unwrap();
    assert_eq!(console.cvars.numeric(view).unwrap(), 5.0);
}

#[test]
fn duplicate_module_config_ranges_are_rejected_at_load() {
    let console = Console::<Runtime>::new(Context::default()).unwrap();
    assert!(matches!(
        ServiceStorage::load(&[(ModuleId(1), 2), (ModuleId(1), 3)], 4, &console.cvars),
        Err(CallError::ConfigString)
    ));
}

#[test]
fn module_leaf_queries_share_the_immutable_world_and_reject_missing_geometry() {
    use qa_core::primitives::{Bounds, Vec3};
    use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let bounds = Bounds {
        mins: Vec3([-8.0; 3]),
        maxs: Vec3([8.0; 3]),
    };
    let mut output = [u32::MAX; 1];
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        assert_eq!(
            (ENGINE_CALLS.box_leaves)(&mut services, bounds, &mut output),
            Err(CallError::Geometry)
        );
    }
    let shared = std::sync::Arc::new(
        VisibilityWorld::load(
            vec![],
            vec![],
            vec![VisLeaf {
                selector: Some(0),
                area: Some(7),
                solid: false,
                bounds,
                surfaces: SurfaceSpan::default(),
            }],
            vec![],
            0,
            -1,
            PvsRows::all_visible(1),
        )
        .unwrap(),
    );
    runtime.collision = Some(
        qa_app::WorldCollision::new(
            &runtime.geometry,
            qa_core::primitives::GeometryId {
                slot: 0,
                generation: 1,
            },
            0,
        )
        .with_visibility(shared.clone()),
    );
    assert!(std::sync::Arc::ptr_eq(
        &shared,
        runtime
            .collision
            .as_ref()
            .unwrap()
            .visibility
            .as_ref()
            .unwrap()
    ));
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let result = (ENGINE_CALLS.box_leaves)(&mut services, bounds, &mut output).unwrap();
    assert_eq!((result.count, result.top_node, output[0]), (1, -1, 0));
    assert_eq!(
        services
            .visibility
            .as_ref()
            .unwrap()
            .0
            .leaf(output[0])
            .unwrap()
            .area,
        Some(7)
    );
}

#[test]
fn module_visibility_selects_compiled_rows_and_caller_portal_policy() {
    use qa_core::primitives::{Bounds, GeometryId, Plane, Vec3};
    use qa_world::visibility::{
        PvsRows, SurfaceSpan, VisLeaf, VisNode,
        VisibilityChannel::{Hearing, Sight},
        VisibilityWorld,
    };
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let a = Vec3([1.0, 0.0, 0.0]);
    let b = Vec3([-1.0, 0.0, 0.0]);
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, b, Sight, false, true, true),
        Err(CallError::Geometry)
    );
    drop(services);
    let bounds = Bounds {
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([10.0; 3]),
    };
    let world = VisibilityWorld::load(
        vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds,
            surfaces: SurfaceSpan::default(),
        }],
        (0..2)
            .map(|i| VisLeaf {
                selector: Some(i),
                area: Some(i),
                solid: false,
                bounds,
                surfaces: SurfaceSpan::default(),
            })
            .collect(),
        vec![],
        0,
        0,
        PvsRows::load(vec![Some(0), Some(1)], vec![1, 2, 3])
            .unwrap()
            .with_hearing(vec![Some(2); 2])
            .unwrap(),
    )
    .unwrap();
    runtime.collision = Some(
        qa_app::WorldCollision::new(
            &runtime.geometry,
            GeometryId {
                slot: 0,
                generation: 1,
            },
            0,
        )
        .with_visibility(std::sync::Arc::new(world))
        .with_portals(Some(
            qa_world::portals::AreaPortals::load(
                2,
                1,
                vec![qa_world::portals::Portal {
                    number: 0,
                    first: 0,
                    second: 1,
                }],
            )
            .unwrap(),
        )),
    );
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, a, Sight, false, true, true),
        Ok(true)
    );
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, b, Sight, false, false, true),
        Ok(false)
    );
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, b, Hearing, false, false, false),
        Ok(true)
    );
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, b, Hearing, false, true, true),
        Ok(false)
    );
    (ENGINE_CALLS.portal_set)(&mut services, 0, true, true).unwrap();
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, b, Hearing, false, true, true),
        Ok(true)
    );
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(&mut services, a, b, Hearing, false, true, false),
        Ok(false)
    );
    assert_eq!(
        (ENGINE_CALLS.in_visibility)(
            &mut services,
            Vec3([f32::NAN; 3]),
            b,
            Hearing,
            false,
            false,
            false
        ),
        Err(CallError::Geometry)
    );
}

#[test]
fn native_resource_indexes_share_configstrings_and_keep_exact_first_gap_order() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage =
        ServiceStorage::load(&[(ModuleId(1), 12), (ModuleId(2), 12)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let models = ResourceRange { first: 1, count: 5 };
    let sounds = ResourceRange { first: 6, count: 5 };
    let index = ENGINE_CALLS.resource_index;
    let set = ENGINE_CALLS.configstring;
    assert_eq!(index(&mut services, ModuleId(1), models, b"").unwrap(), 0);
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"\x80Wall").unwrap(),
        1
    );
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"\x80Wall").unwrap(),
        1
    );
    assert_eq!(
        services.storage.configstring(ModuleId(1), 2).unwrap(),
        (&b"\x80Wall"[..], 1)
    );
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"\x80wall").unwrap(),
        2
    );
    assert_eq!(
        index(&mut services, ModuleId(1), sounds, b"\x80Wall").unwrap(),
        1
    );
    assert_eq!(
        index(&mut services, ModuleId(2), models, b"\x80Wall").unwrap(),
        1
    );
    assert_eq!(
        services.storage.configstring(ModuleId(1), 1).unwrap(),
        (&b""[..], 0)
    );
    // A direct native configstring update changes the cached numeric binding.
    set(&mut services, ModuleId(1), 2, b"replaced").unwrap();
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"replaced").unwrap(),
        1
    );
    // SV_FindIndex fills the first hole even when a later slot has that name.
    set(&mut services, ModuleId(1), 3, b"").unwrap();
    set(&mut services, ModuleId(1), 4, b"beyond-gap").unwrap();
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"beyond-gap").unwrap(),
        2
    );
    assert_eq!(
        services.storage.configstring(ModuleId(1), 3).unwrap(),
        (&b"beyond-gap"[..], 3)
    );
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"last").unwrap(),
        4
    );
    assert_eq!(
        index(&mut services, ModuleId(1), models, b"full"),
        Err(CallError::Capacity)
    );
    assert_eq!(
        index(&mut services, ModuleId(3), models, b"bad"),
        Err(CallError::ConfigString)
    );
    assert_eq!(
        index(
            &mut services,
            ModuleId(1),
            ResourceRange {
                first: 11,
                count: 2
            },
            b"bad"
        ),
        Err(CallError::ConfigString)
    );
    assert_eq!(
        index(
            &mut services,
            ModuleId(1),
            ResourceRange {
                first: usize::MAX,
                count: 2
            },
            b"bad"
        ),
        Err(CallError::ConfigString)
    );
    assert_eq!(
        index(
            &mut services,
            ModuleId(1),
            ResourceRange { first: 0, count: 1 },
            b"bad"
        ),
        Err(CallError::ConfigString)
    );
}

#[test]
fn module_file_handles_are_scoped_and_read_ranges_through_the_existing_vfs() {
    let directory = std::env::temp_dir().join(format!("qa-engine-services-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("module.txt"), b"native file bytes").unwrap();
    let mut runtime = Runtime::load(4, std::iter::empty()).unwrap();
    runtime.vfs.mount_directory(&directory, 0).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 1, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let (handle, length) =
            (ENGINE_CALLS.file_open)(&mut services, ModuleId(1), b"module.txt").unwrap();
        assert_eq!(length, 17);
        let mut out = [0; 6];
        assert_eq!(
            (ENGINE_CALLS.file_read)(&mut services, ModuleId(2), handle, &mut out),
            Err(CallError::File)
        );
        assert_eq!(
            (ENGINE_CALLS.file_read)(&mut services, ModuleId(1), handle, &mut out).unwrap(),
            6
        );
        assert_eq!(&out, b"native");
        assert_eq!(
            (ENGINE_CALLS.file_read)(&mut services, ModuleId(1), handle, &mut out).unwrap(),
            6
        );
        assert_eq!(&out, b" file ");
        assert_eq!(
            (ENGINE_CALLS.file_read)(&mut services, ModuleId(1), handle, &mut out).unwrap(),
            5
        );
        assert_eq!(&out[..5], b"bytes");
        assert_eq!(
            (ENGINE_CALLS.file_read)(&mut services, ModuleId(1), handle, &mut out).unwrap(),
            0
        );
        assert_eq!(
            (ENGINE_CALLS.file_close)(&mut services, ModuleId(2), handle),
            Err(CallError::File)
        );
        (ENGINE_CALLS.file_close)(&mut services, ModuleId(1), handle).unwrap();
        assert_eq!(
            (ENGINE_CALLS.file_read)(&mut services, ModuleId(1), handle, &mut out),
            Err(CallError::File)
        );
    }
    std::fs::remove_file(directory.join("module.txt")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
