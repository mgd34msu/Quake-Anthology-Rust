use qa_app::Runtime;
use qa_compat::services::{CallContext, CallError, ENGINE_CALLS, ServiceStorage};
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
fn modules_share_entity_lifetimes_cvars_command_buffer_and_byte_exact_output() {
    let mut runtime = Runtime::load(4, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 8), (ModuleId(2), 8)], 4).unwrap();
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
        (ENGINE_CALLS.print)(&mut services, None, PrintKind::Console, b"\x82native\n").unwrap();
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
    assert!(matches!(
        ServiceStorage::load(&[(ModuleId(1), 2), (ModuleId(1), 3)], 4),
        Err(CallError::ConfigString)
    ));
}

#[test]
fn module_file_handles_are_scoped_and_read_ranges_through_the_existing_vfs() {
    let directory = std::env::temp_dir().join(format!("qa-engine-services-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("module.txt"), b"native file bytes").unwrap();
    let mut runtime = Runtime::load(4, std::iter::empty()).unwrap();
    runtime.vfs.mount_directory(&directory, 0).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 1).unwrap();
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
