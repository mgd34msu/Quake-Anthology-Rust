use qa_compat::services::{CallError, ENGINE_CALLS, EngineServices, ServiceStorage};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars};
use qa_core::{
    configstrings::ConfigLayout,
    names::NameTable,
    primitives::{IndexSpan, ModuleId, RuleSetId},
};
use qa_world::collision::{CollisionStore, TraceScratch};

struct Fixture {
    server: qa_session::clients::Server,
    input: qa_input::Input,
    cvars: Cvars,
    commands: CommandBuffer,
    vfs: qa_content::vfs::Vfs,
    localization: qa_content::localization::Catalog,
    registry: qa_gameplay::registry::Registry,
    names: NameTable,
    storage: ServiceStorage,
    geometry: CollisionStore,
    scratch: TraceScratch,
}
impl Fixture {
    fn new(profiles: &[(ModuleId, RuleSetId)]) -> Self {
        let cvars = Cvars::new().unwrap();
        let storage = if profiles.is_empty() {
            ServiceStorage::load(&[(ModuleId(1), 12)], 0, &cvars).unwrap()
        } else {
            let configs = profiles
                .iter()
                .map(|&(module, rules)| (module, ConfigLayout::for_rules(rules).count, rules))
                .collect::<Vec<_>>();
            ServiceStorage::load_native(&configs, 0, &cvars).unwrap()
        };
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        Self {
            server: qa_session::clients::Server::load(1, 8, 1, 1, 1, 1).unwrap(),
            input: qa_input::Input::load(),
            cvars,
            commands: CommandBuffer::new(),
            vfs: qa_content::vfs::Vfs::default(),
            localization: qa_content::localization::Catalog::default(),
            registry,
            names,
            storage,
            geometry,
            scratch,
        }
    }
    fn services(&mut self) -> EngineServices<'_> {
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
            client_audio: None,
            renderer: None,
            scene: None,
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
}

#[test]
fn rerelease_models_skip_player_sentinel_in_create_find_and_repeat() {
    let module = ModuleId(1);
    let layout = ConfigLayout::QUAKE2_RERELEASE;
    let models = layout.resources[0];
    let mut fixture = Fixture::new(&[(module, layout.rules)]);
    let mut services = fixture.services();
    let register = ENGINE_CALLS.resource_register;
    let index = ENGINE_CALLS.resource_index;
    let find = ENGINE_CALLS.resource_find;
    assert_eq!(register(&mut services, module, models, b""), Ok((0, false)));
    assert_eq!(find(&mut services, module, models, b""), Ok(Some(0)));
    for ordinal in 1..=254 {
        let name = format!("models/{ordinal}.md2");
        assert_eq!(
            register(&mut services, module, models, name.as_bytes()),
            Ok((ordinal, true))
        );
    }
    let reserved = models.get(255).unwrap();
    let before = services.storage.configstring(module, reserved).unwrap().1;
    assert_eq!(
        register(&mut services, module, models, b"models/next.md2"),
        Ok((256, true))
    );
    assert_eq!(
        find(&mut services, module, models, b"models/next.md2"),
        Ok(Some(256))
    );
    assert_eq!(
        register(&mut services, module, models, b"models/next.md2"),
        Ok((256, false))
    );
    assert_eq!(
        index(&mut services, module, models, b"models/254.md2"),
        Ok(254)
    );
    assert_eq!(
        services.storage.configstring(module, reserved).unwrap(),
        (&b""[..], before)
    );

    // Native configstring writes remain legal, but that row never becomes a
    // filename match or allocation destination, even when it contains a name.
    (ENGINE_CALLS.configstring)(&mut services, module, reserved, b"models/player.md2").unwrap();
    let revision = services.storage.configstring(module, reserved).unwrap().1;
    assert_eq!(
        find(&mut services, module, models, b"models/player.md2"),
        Ok(None)
    );
    assert_eq!(
        register(&mut services, module, models, b"models/player.md2"),
        Ok((257, true))
    );
    assert_eq!(
        register(&mut services, module, models, b"models/player.md2"),
        Ok((257, false))
    );
    assert_eq!(
        find(&mut services, module, models, b"models/player.md2"),
        Ok(Some(257))
    );
    assert_eq!(
        services.storage.configstring(module, reserved).unwrap(),
        (&b"models/player.md2"[..], revision)
    );

    // A permitted earlier hole still wins before a later matching name.
    (ENGINE_CALLS.configstring)(&mut services, module, models.get(254).unwrap(), b"").unwrap();
    assert_eq!(
        find(&mut services, module, models, b"models/next.md2"),
        Ok(None)
    );
    assert_eq!(
        register(&mut services, module, models, b"models/next.md2"),
        Ok((254, true))
    );
}

#[test]
fn rerelease_model_capacity_excludes_exactly_null_and_player_ordinals() {
    let module = ModuleId(1);
    let layout = ConfigLayout::QUAKE2_RERELEASE;
    let models = layout.resources[0];
    let mut fixture = Fixture::new(&[(module, layout.rules)]);
    let mut services = fixture.services();
    let mut count = 0;
    for ordinal in 1..8192 {
        if ordinal == 255 {
            continue;
        }
        let name = format!("models/{ordinal}.md2");
        assert_eq!(
            (ENGINE_CALLS.resource_index)(&mut services, module, models, name.as_bytes()),
            Ok(ordinal)
        );
        count += 1;
    }
    assert_eq!(count, 8190);
    assert_eq!(
        (ENGINE_CALLS.resource_index)(&mut services, module, models, b"models/full.md2"),
        Err(CallError::Capacity)
    );
    assert_eq!(
        (ENGINE_CALLS.resource_find)(&mut services, module, models, b"models/full.md2"),
        Ok(None)
    );
    assert_eq!(
        (ENGINE_CALLS.resource_register)(&mut services, module, models, b"models/8191.md2"),
        Ok((8191, false))
    );
    for ordinal in [0, 255] {
        assert_eq!(
            services
                .storage
                .configstring(module, models.get(ordinal).unwrap())
                .unwrap(),
            (&b""[..], 0)
        );
    }
    assert_eq!(
        services
            .storage
            .configstring(module, layout.resources[1].get(0).unwrap())
            .unwrap(),
        (&b""[..], 0)
    );
    assert_eq!(
        (ENGINE_CALLS.resource_index)(
            &mut services,
            module,
            layout.resources[1],
            b"world/independent.wav"
        ),
        Ok(1)
    );
}

#[test]
fn other_native_resource_tables_still_admit_ordinal_255() {
    let module = ModuleId(1);
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let layout = ConfigLayout::for_rules(rules);
        let mut fixture = Fixture::new(&[(module, rules)]);
        let mut services = fixture.services();
        for (kind, range) in layout.resources.into_iter().enumerate() {
            if rules == RuleSetId::Quake2Rerelease && kind == 0 {
                continue;
            }
            if range.count == 0 {
                assert_eq!(
                    (ENGINE_CALLS.resource_index)(&mut services, module, range, b"absent"),
                    Err(CallError::ConfigString)
                );
                continue;
            }
            for ordinal in 1..=255 {
                let name = format!("resource/{kind}/{ordinal}");
                assert_eq!(
                    (ENGINE_CALLS.resource_index)(&mut services, module, range, name.as_bytes()),
                    Ok(ordinal),
                    "{rules:?}, kind {kind}"
                );
            }
        }
    }
}

#[test]
fn native_policy_is_module_scoped_and_wrong_spans_cannot_bypass_it() {
    let first = ModuleId(1);
    let second = ModuleId(2);
    let classic = ModuleId(3);
    let rr = ConfigLayout::QUAKE2_RERELEASE;
    let q2 = ConfigLayout::QUAKE2;
    let mut fixture = Fixture::new(&[(first, rr.rules), (second, rr.rules), (classic, q2.rules)]);
    let mut services = fixture.services();
    for module in [first, second] {
        for ordinal in 1..=254 {
            let name = format!("models/{ordinal}");
            assert_eq!(
                (ENGINE_CALLS.resource_index)(
                    &mut services,
                    module,
                    rr.resources[0],
                    name.as_bytes()
                ),
                Ok(ordinal)
            );
        }
        assert_eq!(
            (ENGINE_CALLS.resource_index)(&mut services, module, rr.resources[0], b"models/next"),
            Ok(256)
        );
    }
    assert_eq!(
        services
            .storage
            .configstring(classic, q2.resources[0].get(1).unwrap())
            .unwrap(),
        (&b""[..], 0)
    );
    for wrong in [
        q2.resources[0],
        IndexSpan {
            first: rr.resources[0].first + 1,
            count: rr.resources[0].count - 1,
        },
    ] {
        assert_eq!(
            (ENGINE_CALLS.resource_index)(&mut services, first, wrong, b"models/next"),
            Err(CallError::ConfigString)
        );
        assert_eq!(
            (ENGINE_CALLS.resource_find)(&mut services, first, wrong, b"models/next"),
            Err(CallError::ConfigString)
        );
    }
}

#[test]
fn unprofiled_storage_keeps_explicit_spans_and_exact_name_order() {
    let mut fixture = Fixture::new(&[]);
    let mut services = fixture.services();
    let module = ModuleId(1);
    let range = IndexSpan { first: 1, count: 5 };
    for (name, expected) in [(b"\x80Wall".as_slice(), 1), (b"\x80wall", 2)] {
        assert_eq!(
            (ENGINE_CALLS.resource_index)(&mut services, module, range, name),
            Ok(expected)
        );
    }
    assert_eq!(
        (ENGINE_CALLS.resource_register)(&mut services, module, range, b"\x80Wall"),
        Ok((1, false))
    );
    assert_eq!(
        (ENGINE_CALLS.resource_index)(&mut services, ModuleId(99), range, b"missing"),
        Err(CallError::ConfigString)
    );
}
