use qa_compat::{
    abi::{Addresses, Invocation, Q3_SERVER, UnknownCalls},
    memory::ModuleMemory,
    services::{
        CallContext, CallError, ENGINE_CALLS, EngineServices, Q3_USERINFO_BYTES, ServiceStorage,
        userinfo_rules,
    },
};
use qa_console::{
    catalog::Scope,
    command_buffer::CommandBuffer,
    cvars::Cvars,
    views::{Context, Role},
};
use qa_core::{
    names::NameTable,
    primitives::{ClientId, ModuleId, NativeEntity, RuleSetId, ThinkTime},
    sys_events::EventTime,
};
use qa_session::clients::{Connection, Server};
use qa_world::{
    collision::{CollisionStore, TraceScratch},
    entities::AllocationPolicy,
};

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
    scratch: TraceScratch,
    id: ClientId,
}
impl Fixture {
    fn new() -> Self {
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let cvars = Cvars::new().unwrap();
        let mut server = Server::load(2, 8, 1, 1, 1, 0).unwrap();
        let native = NativeEntity {
            module: ModuleId(10),
            slot: 0,
        };
        let id = server
            .connect(Connection::Remote, native.module, Some(native))
            .unwrap();
        server.clients[id.0 as usize].player.movement_rules = RuleSetId::QuakeWorld;
        server.clients[id.0 as usize].player.trace_rules = RuleSetId::Quake2Rerelease;
        let mut storage = ServiceStorage::load_native(
            &[
                (native.module, 0, RuleSetId::Quake3),
                (ModuleId(20), 0, RuleSetId::Quake2Rerelease),
            ],
            0,
            &cvars,
        )
        .unwrap();
        storage.bind_client_capacity(native.module, 2).unwrap();
        storage.bind_client_capacity(ModuleId(20), 2).unwrap();
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        Self {
            server,
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
            id,
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
        memory: &mut ModuleMemory<'_>,
        addresses: Addresses,
        number: u32,
        arguments: &[u64],
        console_source: RuleSetId,
    ) -> Result<u64, CallError> {
        let mut unknown = UnknownCalls::load(1).unwrap();
        let result = Q3_SERVER.invoke(
            number,
            &mut Invocation {
                services: &mut self.engine(),
                memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: CallContext {
                    module: ModuleId(10),
                    clock: ThinkTime::Milliseconds(17),
                    server_frame: 3,
                    native_rules: RuleSetId::Quake3,
                    trace_rules: RuleSetId::Quake2,
                    sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake),
                    console: Context {
                        source: console_source,
                        ..Context::default()
                    },
                    allocation: AllocationPolicy::EDICT,
                    link_order: qa_gameplay::rules::link_order(RuleSetId::QuakeWorld),
                },
                platform_time: EventTime(17),
                command: &[],
                addresses,
                arguments,
            },
            &mut unknown,
        );
        assert_eq!(unknown.calls, 0, "imports must use real CallTable entries");
        result
    }
}
fn modes() -> [(Addresses, u64); 4] {
    [
        (Addresses::Qvm { mask: 8191 }, 0),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 4,
            },
            0x2000_0000,
        ),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
                pointer_bytes: 8,
            },
            1 << 40,
        ),
        (
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::Microsoft,
                pointer_bytes: 8,
            },
            1 << 40,
        ),
    ]
}

#[test]
fn q3_userinfo_imports_preserve_raw_bytes_truncation_and_native_pointer_widths() {
    for (mode, base) in modes() {
        let mut fixture = Fixture::new();
        let mut memory = ModuleMemory::load(base, 8192, &[]).unwrap();
        let raw = b"\\Name\\raw\xff\\name\\second;\"\x01";
        memory.write(base + 128, raw).unwrap();
        memory.write(base + 128 + raw.len() as u64, &[0]).unwrap();
        assert_eq!(
            fixture.call(&mut memory, mode, 21, &[0, base + 128], RuleSetId::Quake),
            Ok(0)
        );
        assert_eq!(fixture.server.userinfo(fixture.id), Some(raw.as_slice()));
        assert_eq!(
            fixture.call(
                &mut memory,
                mode,
                20,
                &[0, base + 512, 1024],
                RuleSetId::QuakeWorld
            ),
            Ok(0)
        );
        assert_eq!(memory.cstring(base + 512).unwrap(), raw);
        memory.write(base + 512, &[0x55; 8]).unwrap();
        fixture
            .call(
                &mut memory,
                mode,
                20,
                &[0, base + 512, 8],
                RuleSetId::Quake2,
            )
            .unwrap();
        assert_eq!(
            memory.read(base + 512, 8).unwrap(),
            [&raw[..7], &[0]].concat()
        );
        fixture
            .call(
                &mut memory,
                mode,
                20,
                &[0, base + 512, 1],
                RuleSetId::Quake2,
            )
            .unwrap();
        assert_eq!(memory.read(base + 512, 1).unwrap(), [0]);
        assert_eq!(
            fixture.call(&mut memory, mode, 20, &[0, base + 512, 0], RuleSetId::Quake),
            Err(CallError::Memory)
        );
        fixture
            .call(&mut memory, mode, 21, &[0, 0], RuleSetId::Quake)
            .unwrap();
        assert_eq!(fixture.server.userinfo(fixture.id), Some(b"".as_slice()));
    }
}

#[test]
fn native_setter_truncates_at_1023_and_keeps_larger_canonical_rerelease_capacity() {
    let mut fixture = Fixture::new();
    let mut memory = ModuleMemory::load(0, 8192, &[]).unwrap();
    let mode = Addresses::Qvm { mask: 8191 };
    let text = vec![0xff; 1500];
    memory.write(128, &text).unwrap();
    memory.write(1628, &[0]).unwrap();
    fixture
        .call(&mut memory, mode, 21, &[0, 128], RuleSetId::Quake2Rerelease)
        .unwrap();
    assert_eq!(
        fixture.server.userinfo(fixture.id).unwrap(),
        &text[..Q3_USERINFO_BYTES - 1]
    );
    let other_native = NativeEntity {
        module: ModuleId(20),
        slot: 2,
    };
    let other = fixture
        .server
        .connect(Connection::Local, other_native.module, Some(other_native))
        .unwrap();
    let full = vec![b'x'; 2047];
    (ENGINE_CALLS.set_userinfo)(&mut fixture.engine(), other_native, &full).unwrap();
    assert_eq!(fixture.server.userinfo(other), Some(full.as_slice()));
    assert_eq!(
        (ENGINE_CALLS.userinfo)(&fixture.engine(), other_native).unwrap(),
        full
    );
    assert_eq!(userinfo_rules(RuleSetId::Quake2Rerelease).bytes, 2048);
    assert_eq!(
        userinfo_rules(RuleSetId::QuakeWorld).bytes,
        qa_compat::quakec::QW_USERINFO_BYTES
    );
}

#[test]
fn frozen_native_limit_and_module_namespace_reject_stale_setters() {
    let mut fixture = Fixture::new();
    let maxclients = fixture
        .cvars
        .bind("sv_maxclients", Context::default())
        .unwrap();
    fixture.cvars.force_write(maxclients, "32").unwrap();
    assert_eq!(fixture.storage.client_capacity(ModuleId(10)), Some(2));
    assert_eq!(
        fixture.storage.bind_client_capacity(ModuleId(10), 32),
        Err(CallError::Capacity)
    );
    fixture
        .storage
        .bind_client_capacity(ModuleId(10), 2)
        .unwrap();
    let original = NativeEntity {
        module: ModuleId(10),
        slot: 0,
    };
    let invalid = NativeEntity {
        slot: 2,
        ..original
    };
    assert_eq!(
        (ENGINE_CALLS.userinfo)(&fixture.engine(), invalid),
        Err(CallError::Entity)
    );
    assert_eq!(
        (ENGINE_CALLS.userinfo)(
            &fixture.engine(),
            NativeEntity {
                slot: 1,
                ..original
            }
        ),
        Ok(b"".as_slice())
    );
    assert_eq!(
        (ENGINE_CALLS.set_userinfo)(
            &mut fixture.engine(),
            NativeEntity {
                slot: 1,
                ..original
            },
            b"unused"
        ),
        Err(CallError::Entity)
    );
    fixture
        .server
        .replace_userinfo(
            fixture.id,
            b"\\name\\old",
            userinfo_rules(RuleSetId::Quake3),
        )
        .unwrap();
    assert!(fixture.server.bind_client_native(
        fixture.id,
        NativeEntity {
            module: ModuleId(20),
            slot: 1
        }
    ));
    assert_eq!(
        (ENGINE_CALLS.set_userinfo)(&mut fixture.engine(), original, b"stale"),
        Err(CallError::Entity)
    );
    assert_eq!(
        fixture.server.userinfo(fixture.id),
        Some(b"\\name\\old".as_slice())
    );
    assert!(fixture.server.disconnect(fixture.id));
    assert_eq!(
        (ENGINE_CALLS.userinfo)(
            &fixture.engine(),
            NativeEntity {
                module: ModuleId(20),
                slot: 1
            }
        ),
        Ok(b"".as_slice())
    );
}

#[test]
fn q3_serverinfo_uses_native_server_scope_and_the_same_cvar_writer() {
    let mut fixture = Fixture::new();
    let server = Context {
        source: RuleSetId::Quake3,
        side: Scope::Server,
        role: Role::Game,
        ..Context::default()
    };
    let client = Context {
        source: RuleSetId::Quake3,
        ..Context::default()
    };
    for (context, value) in [(client, "client-secret"), (server, "server-secret")] {
        let view = fixture.cvars.bind("password", context).unwrap();
        fixture.cvars.full_set(view, value, 4).unwrap();
    }
    fixture
        .cvars
        .register("_qa_server_info", Some("from-module"), 4, server)
        .unwrap();
    let mut expected = [0; Q3_USERINFO_BYTES];
    let length = fixture.cvars.info_string(server, 4, &mut expected).unwrap();
    let value = qa_core::text::info_value(&expected[..length], b"password", false).unwrap();
    assert_eq!(&expected[value], b"server-secret");
    for (mode, base) in modes() {
        let mut memory = ModuleMemory::load(base, 8192, &[]).unwrap();
        for console_source in [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake3,
        ] {
            fixture
                .call(&mut memory, mode, 22, &[base + 256, 1024], console_source)
                .unwrap();
            assert_eq!(memory.cstring(base + 256).unwrap(), &expected[..length]);
            fixture
                .call(&mut memory, mode, 22, &[base + 256, 8], console_source)
                .unwrap();
            assert_eq!(
                memory.read(base + 256, 8).unwrap(),
                [&expected[..7], &[0]].concat()
            );
        }
        assert_eq!(
            fixture.call(&mut memory, mode, 22, &[base + 256, 0], RuleSetId::Quake3),
            Err(CallError::Memory)
        );
    }
}
