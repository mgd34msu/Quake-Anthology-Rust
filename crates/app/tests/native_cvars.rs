use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, Invocation, Q2_CLASSIC, Q2_RERELEASE, UnknownCalls},
    cvars::NativeCvars,
    memory::ModuleMemory,
    services::{CallContext, ServiceStorage},
};
use qa_console::{
    commands::Console,
    views::{Context, Role},
};
use qa_core::{
    primitives::{ModuleId, RuleSetId, ThinkTime},
    sys_events::EventTime,
};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

fn pointer(memory: &ModuleMemory<'_>, address: u64) -> u64 {
    u64::from_le_bytes(memory.read(address, 8).unwrap().try_into().unwrap())
}

#[test]
fn q2_cvar_layouts_preserve_cached_views_latches_and_native_modification_fields() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let rr = rules == RuleSetId::Quake2Rerelease;
        let table = if rr { &Q2_RERELEASE } else { &Q2_CLASSIC };
        let get = if rr { 39 } else { 36 };
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let console_context = Context {
            source: rules,
            role: Role::Game,
            ..Context::default()
        };
        let mut console = Console::new(console_context).unwrap();
        let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let context = CallContext {
            module: ModuleId(1),
            clock: ThinkTime::Milliseconds(0),
            console: console_context,
            allocation: AllocationPolicy::EDICT,
            link_order: LinkOrder::Head,
        };
        let base = 1u64 << 40;
        let mut memory =
            ModuleMemory::load(base, 4096 + NativeCvars::byte_length(4).unwrap(), &[]).unwrap();
        memory.write(base + 16, b"_qa_native_test\0").unwrap();
        memory.write(base + 64, b"1e3\0").unwrap();
        let mut native = NativeCvars::load(base + 4096, 4, rr);
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let extra = if rr { 32 } else { 0 };
        let mut invoke = |native: &mut NativeCvars,
                          memory: &mut ModuleMemory<'_>,
                          services: &mut qa_compat::services::EngineServices<'_>,
                          number,
                          arguments: &[u64]| {
            table
                .invoke(
                    number,
                    &mut Invocation {
                        services,
                        memory,
                        native_cvars: Some(native),
                        native_resources: None,
                        native_entities: None,
                        native_surfaces: None,
                        context,
                        platform_time: EventTime(0),
                        command: &[],
                        addresses: Addresses::NativeFunction,
                        arguments,
                    },
                    &mut unknown,
                )
                .unwrap()
        };
        let cvar = invoke(
            &mut native,
            &mut memory,
            &mut services,
            get,
            &[base + 16, base + 64, 20 | extra],
        );
        assert_eq!(
            memory.cstring(pointer(&memory, cvar)).unwrap(),
            b"_qa_native_test"
        );
        let value_pointer = pointer(&memory, cvar + 8);
        assert_eq!(memory.cstring(value_pointer).unwrap(), b"1e3");
        assert_eq!(memory.read_word(cvar + 24).unwrap(), (20 | extra) as i32);
        assert_eq!(memory.read_word(cvar + 28).unwrap(), 1);
        assert_eq!(
            memory.read_word(cvar + 32).unwrap() as u32,
            1000f32.to_bits()
        );
        if rr {
            assert_eq!(memory.read_word(cvar + 48).unwrap(), 1);
        }
        let view = services
            .cvars
            .bind("_qa_native_test", console_context)
            .unwrap();
        memory.write(base + 16, b"_QA_NATIVE_TEST\0").unwrap();
        memory.write(base + 64, b"99\0").unwrap();
        assert_eq!(
            invoke(
                &mut native,
                &mut memory,
                &mut services,
                get,
                &[base + 16, base + 64, 0]
            ),
            cvar
        );
        assert_eq!(native.updates, 1);
        services.cvars.server_active = true;
        let generation = services.cvars.view_generation(view);
        memory
            .write_word(cvar + 28, if rr { 1 } else { 0 })
            .unwrap();
        memory.write(base + 64, b"42\0").unwrap();
        assert_eq!(
            invoke(
                &mut native,
                &mut memory,
                &mut services,
                get + 1,
                &[base + 16, base + 64]
            ),
            cvar
        );
        assert_eq!(services.cvars.view_generation(view), generation);
        assert_eq!(memory.cstring(value_pointer).unwrap(), b"1e3");
        assert_eq!(memory.cstring(pointer(&memory, cvar + 16)).unwrap(), b"42");
        assert_eq!(memory.read_word(cvar + 28).unwrap(), if rr { 1 } else { 0 });
        let updates = native.updates;
        native.refresh(services.cvars, &mut memory).unwrap();
        assert_eq!(native.updates, updates);
        memory.write(base + 64, b"78\0").unwrap();
        assert_eq!(
            invoke(
                &mut native,
                &mut memory,
                &mut services,
                get + 2,
                &[base + 16, base + 64]
            ),
            cvar
        );
        assert_eq!(memory.cstring(value_pointer).unwrap(), b"78");
        assert_eq!(pointer(&memory, cvar + 16), 0);
        assert_eq!(memory.read_word(cvar + 24).unwrap(), (20 | extra) as i32);
        assert_eq!(memory.read_word(cvar + 28).unwrap(), if rr { 2 } else { 1 });
        services.cvars.apply_latches().unwrap();
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "78");
        // A second binding stays byte-exact when only the first changes.
        memory.write(base + 16, b"_qa_other\0").unwrap();
        let other = invoke(
            &mut native,
            &mut memory,
            &mut services,
            get,
            &[base + 16, base + 64, 0],
        );
        assert_eq!(pointer(&memory, other + 40), cvar);
        let saved = memory.read(other, 56).unwrap().to_vec();
        let updates = native.updates;
        services.cvars.force_write(view, "17.25").unwrap();
        native.refresh(services.cvars, &mut memory).unwrap();
        assert_eq!(memory.read(other, 56).unwrap(), saved);
        assert_eq!(native.updates, updates + 1);
        assert_eq!(memory.cstring(value_pointer).unwrap(), b"17.25");
        if rr {
            memory.write_word(cvar + 28, -1).unwrap();
            services.cvars.force_write(view, "19").unwrap();
            native.refresh(services.cvars, &mut memory).unwrap();
            assert_eq!(memory.read_word(cvar + 28).unwrap(), 1);
        }
        // NOSET rejects regular writes even during startup; force preserves it.
        memory.write(base + 16, b"_qa_protected\0").unwrap();
        let protected = invoke(
            &mut native,
            &mut memory,
            &mut services,
            get,
            &[base + 16, base + 64, 8],
        );
        memory.write(base + 64, b"22\0").unwrap();
        assert_eq!(
            invoke(
                &mut native,
                &mut memory,
                &mut services,
                get + 1,
                &[base + 16, base + 64]
            ),
            protected
        );
        assert_eq!(
            memory.cstring(pointer(&memory, protected + 8)).unwrap(),
            b"78"
        );
        assert_eq!(
            invoke(
                &mut native,
                &mut memory,
                &mut services,
                get + 2,
                &[base + 16, base + 64]
            ),
            protected
        );
        assert_eq!(
            memory.cstring(pointer(&memory, protected + 8)).unwrap(),
            b"22"
        );
        assert_eq!(memory.read_word(protected + 24).unwrap(), 8);
        memory.write(base + 16, b"_qa_absent\0").unwrap();
        let count = services.cvars.entries().count();
        assert_eq!(
            invoke(
                &mut native,
                &mut memory,
                &mut services,
                get,
                &[base + 16, 0, 0]
            ),
            0
        );
        assert_eq!(services.cvars.entries().count(), count);
        // The catalog's rerelease maxentities default is unresolved. Native
        // Cvar_Get supplies the module's value in the same existing row.
        memory.write(base + 16, b"maxentities\0").unwrap();
        memory.write(base + 64, b"8192\0").unwrap();
        let query = invoke(
            &mut native,
            &mut memory,
            &mut services,
            get,
            &[base + 16, 0, 16],
        );
        let edicts = invoke(
            &mut native,
            &mut memory,
            &mut services,
            get,
            &[base + 16, base + 64, 16],
        );
        assert_eq!(query, edicts);
        assert_eq!(
            memory.cstring(pointer(&memory, edicts + 8)).unwrap(),
            if rr { &b"8192"[..] } else { &b"1024"[..] }
        );
        assert_eq!(
            memory.read_word(edicts + 32).unwrap() as u32,
            if rr {
                8192f32.to_bits()
            } else {
                1024f32.to_bits()
            }
        );
        if rr {
            assert_eq!(memory.read_word(edicts + 48).unwrap(), 8192);
        }
        assert_eq!(unknown.calls, 0);
    }
}

#[test]
fn native_bindings_keep_alias_projections_distinct_in_the_one_registry() {
    let context = Context {
        source: RuleSetId::Quake2,
        role: Role::Game,
        ..Context::default()
    };
    let mut cvars = qa_console::cvars::Cvars::with_context(context).unwrap();
    let first = cvars.bind("s_khz", context).unwrap();
    let second = cvars.bind("s_outputRate", context).unwrap();
    assert_eq!(first.canonical(), second.canonical());
    let base = 4096;
    let mut memory = ModuleMemory::load(base, NativeCvars::byte_length(2).unwrap(), &[]).unwrap();
    let mut native = NativeCvars::load(base, 2, false);
    let khz = native.publish(&cvars, &mut memory, first, 0).unwrap();
    let hz = native.publish(&cvars, &mut memory, second, 0).unwrap();
    assert_ne!(khz, hz);
    cvars.force_write(first, "22").unwrap();
    native.refresh(&cvars, &mut memory).unwrap();
    assert_eq!(memory.read_word(khz + 32).unwrap() as u32, 22f32.to_bits());
    assert_eq!(
        memory.read_word(hz + 32).unwrap() as u32,
        22050f32.to_bits()
    );
    assert!(
        native
            .publish(&cvars, &mut memory, cvars.bind("fov", context).unwrap(), 0)
            .is_err()
    );
    assert_eq!(native.capacity_drops, 1);
    cvars.register("s_khz", Some("22"), 32, context).unwrap();
    cvars.server_active = true;
    cvars.write(first, "44").unwrap();
    assert_eq!(cvars.latched(first).unwrap().unwrap().as_str(), "44");
    assert_eq!(
        cvars.latched(second).unwrap().unwrap().as_str(),
        "44100.000000"
    );
    native.refresh(&cvars, &mut memory).unwrap();
    assert_eq!(memory.read_word(khz + 32).unwrap() as u32, 22f32.to_bits());
    assert_eq!(memory.cstring(pointer(&memory, khz + 16)).unwrap(), b"44");
    assert_eq!(
        memory.cstring(pointer(&memory, hz + 16)).unwrap(),
        b"44100.000000"
    );
}
