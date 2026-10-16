use qa_compat::{
    abi::{Addresses, Invocation, Q3_CLIENT, Q3_UI, UnknownCalls},
    memory::ModuleMemory,
    services::{
        CallContext, CallError, ClientAudio, ENGINE_CALLS, EngineServices, ServiceStorage,
        SoundRegistrar,
    },
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    events::{EventRing, FrameEvent, OutputConsumerId, OutputSubmission, OutputTarget},
    names::NameTable,
    primitives::{
        ClientId, EntityId, Listener, LoopLifetime, ModuleId, Pcm, PcmChannels, ResourceNamespace,
        RuleSetId, SoundAction, SoundBus, SoundEmitter, SoundEvent, ThinkTime, Vec3,
    },
    sys_events::EventTime,
};
use qa_world::{collision::CollisionStore, entities::AllocationPolicy};
use std::num::NonZeroU32;

#[path = "support/shaders.rs"]
mod shaders;
use shaders::ShaderLoader;

struct Registrar {
    bank: qa_audio::Bank,
    calls: Vec<(Vec<u8>, RuleSetId)>,
}

impl SoundRegistrar for Registrar {
    fn register(
        &mut self,
        _: &qa_content::vfs::Vfs,
        sample: &[u8],
        source: RuleSetId,
    ) -> Result<i32, CallError> {
        self.calls.push((sample.to_vec(), source));
        let Some(path) = qa_audio::sound_path(sample, source).map_err(|_| CallError::Text)? else {
            return Ok(0);
        };
        self.bank
            .attempt_registration(sample, Some(path.as_bytes()))
            .map_err(|_| CallError::Capacity)?;
        self.bank
            .find_bytes(path.as_bytes())
            .ok_or(CallError::File)?;
        self.bank.registered_handle(sample).ok_or(CallError::File)
    }
    fn precache(
        &mut self,
        _: &qa_content::vfs::Vfs,
        sample: &[u8],
        source: RuleSetId,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError> {
        self.calls.push((sample.to_vec(), source));
        let Some(path) = qa_audio::sound_path(sample, source).map_err(|_| CallError::Text)? else {
            return Ok(None);
        };
        self.bank
            .attempt(path.as_bytes())
            .map_err(|_| CallError::Capacity)?;
        self.bank
            .find_bytes(path.as_bytes())
            .map(Some)
            .ok_or(CallError::File)
    }
    fn bank(&self) -> &qa_audio::Bank {
        &self.bank
    }
}

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
    scratch: qa_world::collision::TraceScratch,
    registrar: Registrar,
    output: OutputConsumerId,
    native_output: OutputConsumerId,
}
impl Fixture {
    fn new() -> Self {
        Self::native(RuleSetId::Quake3)
    }
    fn native(rules: RuleSetId) -> Self {
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let cvars = Cvars::new().unwrap();
        let mut server = qa_session::clients::Server::load(1, 8, 1, 1, 1, 1).unwrap();
        let output = server.presentation;
        let native_output = server
            .events
            .bind(OutputTarget::Module(ModuleId(2)))
            .unwrap();
        let storage = ServiceStorage::load_native(
            &[(
                ModuleId(1),
                qa_core::configstrings::ConfigLayout::for_rules(rules).count,
                rules,
            )],
            0,
            &cvars,
        )
        .unwrap();
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        let rate = NonZeroU32::new(22050).unwrap();
        let mut bank = qa_audio::Bank::with_capacity(rate, 8, 512).unwrap();
        for path in [
            b"sound/feedback/hit.wav".as_slice(),
            b"sound/a.wav",
            b"sound/b.wav",
        ] {
            let sound = bank.attempt(path).unwrap();
            bank.admit(sound, pcm(rate)).unwrap();
            bank.attempt_registration(path, Some(path)).unwrap();
        }
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
            registrar: Registrar {
                bank,
                calls: vec![],
            },
            output,
            native_output,
        }
    }
    fn engine(&mut self, cold: bool) -> EngineServices<'_> {
        let (sound_loader, sound_bank): (Option<&mut dyn SoundRegistrar>, Option<&qa_audio::Bank>) =
            if cold {
                (Some(&mut self.registrar), None)
            } else {
                (None, Some(&self.registrar.bank))
            };
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
            renderer: None,
            scene: None,
            client_audio: None,
            vfs: &self.vfs,
            model_loader: None,
            model_lookup: None,
            sound_loader,
            sound_bank,
            localization: &self.localization,
            registry: &self.registry,
            names: &mut self.names,
            storage: &mut self.storage,
            geometry: &self.geometry,
            world: None,
            native_geometry: None,
            foreign_trace: None,
            visibility: None,
            portals: None,
            scratch: &mut self.scratch,
        }
    }
}

fn pcm(rate: NonZeroU32) -> Pcm {
    Pcm {
        rate,
        channels: PcmChannels::Mono,
        samples: vec![1000, -1000],
        loop_start: None,
        loop_end: None,
        loop_invalid: false,
    }
}

fn context() -> CallContext {
    CallContext {
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::QuakeWorld,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
        console: Context {
            source: RuleSetId::Quake,
            ..Context::default()
        },
        module: ModuleId(1),
        clock: ThinkTime::Milliseconds(17),
        server_frame: 9,
        allocation: AllocationPolicy::EDICT,
        link_order: qa_gameplay::rules::link_order(RuleSetId::QuakeWorld),
    }
}
fn modes() -> [Addresses; 4] {
    [
        Addresses::Qvm { mask: 511 },
        Addresses::Native {
            abi: qa_platform::native::NativeAbi::SystemV,
            pointer_bytes: 4,
        },
        Addresses::Native {
            abi: qa_platform::native::NativeAbi::SystemV,
            pointer_bytes: 8,
        },
        Addresses::Native {
            abi: qa_platform::native::NativeAbi::Microsoft,
            pointer_bytes: 8,
        },
    ]
}
fn base(addresses: Addresses) -> u64 {
    match addresses {
        Addresses::Qvm { .. } => 0,
        Addresses::Native {
            pointer_bytes: 4, ..
        } => 0x2000_0000,
        _ => 1 << 40,
    }
}
fn invoke(
    services: &mut EngineServices<'_>,
    memory: &mut ModuleMemory<'_>,
    addresses: Addresses,
    ui: bool,
    pointer: u64,
) -> Result<u64, CallError> {
    let arguments = [pointer, u64::MAX];
    invoke_call(
        services,
        memory,
        addresses,
        if ui { &Q3_UI } else { &Q3_CLIENT },
        if ui { 31 } else { 34 },
        &arguments,
    )
}
fn invoke_call(
    services: &mut EngineServices<'_>,
    memory: &mut ModuleMemory<'_>,
    addresses: Addresses,
    table: &qa_compat::abi::CallTable,
    number: u32,
    arguments: &[u64],
) -> Result<u64, CallError> {
    let mut call = Invocation {
        services,
        memory,
        native_cvars: None,
        native_resources: None,
        native_entities: None,
        native_surfaces: None,
        native_command: None,
        native_configs: None,
        native_pmove: None,
        native_player: None,
        context: context(),
        platform_time: EventTime(123_456_789),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        addresses,
        arguments,
    };
    let mut unknown = UnknownCalls::load(1).unwrap();
    let result = table.invoke(number, &mut call, &mut unknown);
    assert_eq!(unknown.calls, 0);
    result.map(|step| match step {
        qa_compat::abi::ImportStep::Return(value) => value,
        qa_compat::abi::ImportStep::Host { .. } => panic!("unexpected host request"),
    })
}

#[test]
fn cgame_shader_registration_shares_checked_native_and_qvm_dispatch_and_frozen_lookup() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let mut shader = ShaderLoader::default();
        let base = base(addresses);
        let mut bytes = [0; 512];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        memory.write(base + 16, b"ui/pic\0").unwrap();
        memory.write(base + 48, b"ui/missing\0").unwrap();
        memory.write(base + 96, &[b'x'; 64]).unwrap();
        memory.write(base + 160, &[0]).unwrap();
        memory.write(base + 176, &[0]).unwrap();
        {
            let mut engine = fixture.engine(true);
            engine.model_loader = Some(&mut shader);
            for ordinal in [39, 57] {
                assert_eq!(
                    invoke_call(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        ordinal,
                        &[base + 16]
                    ),
                    Ok(42)
                );
                assert_eq!(
                    invoke_call(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        ordinal,
                        &[0]
                    ),
                    Err(CallError::Memory)
                );
                assert_eq!(
                    invoke_call(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        ordinal,
                        &[base + 176]
                    ),
                    Ok(0)
                );
                assert_eq!(
                    invoke_call(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        ordinal,
                        &[base + 96]
                    ),
                    Ok(0)
                );
                assert_eq!(
                    invoke_call(
                        &mut engine,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        ordinal,
                        &[base + 48]
                    ),
                    Ok(0)
                );
            }
            if matches!(addresses, Addresses::Qvm { .. }) {
                memory.write(0, b"ui/pic\0").unwrap();
                assert_eq!(
                    invoke_call(&mut engine, &mut memory, addresses, &Q3_CLIENT, 39, &[512]),
                    Ok(42)
                );
            }
        }
        assert_eq!(
            &shader.calls[..4],
            &[
                (b"ui/pic".to_vec(), RuleSetId::Quake3, true),
                (b"ui/missing".to_vec(), RuleSetId::Quake3, true),
                (b"ui/pic".to_vec(), RuleSetId::Quake3, false),
                (b"ui/missing".to_vec(), RuleSetId::Quake3, false),
            ]
        );
        let before = shader.calls.len();
        {
            let mut engine = fixture.engine(false);
            engine.model_lookup = Some(&shader);
            assert_eq!(
                invoke_call(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    57,
                    &[base + 16]
                ),
                Ok(42)
            );
            assert_eq!(
                invoke_call(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    39,
                    &[base + 48]
                ),
                Ok(0)
            );
            memory.write(base + 48, b"ui/unknown\0").unwrap();
            assert_eq!(
                invoke_call(
                    &mut engine,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    39,
                    &[base + 48]
                ),
                Ok(0)
            );
        }
        assert_eq!(shader.calls.len(), before);
    }
}

#[test]
fn executed_qvm_shader_imports_share_the_same_cold_service() {
    use Opcode::*;
    use qa_formats::program::qvm::{Image, Opcode};
    let operations: [(Opcode, i32); 11] = [
        (Enter, 32),
        (Const, 16),
        (Arg, 8),
        (Const, -40),
        (Call, 0),
        (Pop, 0),
        (Const, 16),
        (Arg, 8),
        (Const, -58),
        (Call, 0),
        (Leave, 32),
    ];
    let mut code = Vec::new();
    for (opcode, argument) in operations {
        code.push(opcode as u8);
        match opcode.operand_bytes() {
            1 => code.push(argument as u8),
            4 => code.extend(argument.to_le_bytes()),
            _ => {}
        }
    }
    code.resize((code.len() + 3) & !3, 0);
    let mut file = Vec::new();
    for value in [
        0x12721444i32,
        operations.len() as i32,
        32,
        code.len() as i32,
        32 + code.len() as i32,
        64,
        0,
        65536 - 64,
    ] {
        file.extend(value.to_le_bytes());
    }
    file.extend(code);
    let data = file.len();
    file.resize(data + 64, 0);
    file[data + 16..data + 23].copy_from_slice(b"ui/pic\0");
    let mut vm = qa_compat::qvm::Vm::load(Image::parse(&file).unwrap()).unwrap();
    let mut fixture = Fixture::new();
    let mut shader = ShaderLoader::default();
    let mut unknown = UnknownCalls::load(1).unwrap();
    {
        let mut engine = fixture.engine(true);
        engine.model_loader = Some(&mut shader);
        let mut calls = qa_compat::abi::QvmCalls {
            services: &mut engine,
            table: &Q3_CLIENT,
            context: context(),
            platform_time: EventTime(19),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [0; 10], 100, false), Ok(42));
    }
    assert_eq!(
        shader.calls,
        vec![
            (b"ui/pic".to_vec(), RuleSetId::Quake3, true),
            (b"ui/pic".to_vec(), RuleSetId::Quake3, false)
        ]
    );
    assert_eq!(unknown.calls, 0);
}

#[test]
fn cgame_and_ui_use_one_bank_with_valid_engine_handle_zero_in_each_address_mode() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let default = fixture.registrar.bank.registered_sound(0).unwrap();
        let positive = fixture
            .registrar
            .bank
            .registered_handle(b"sound/a.wav")
            .unwrap();
        assert!(positive > 0);
        let expected = fixture.registrar.bank.registered_sound(positive).unwrap();
        let base = base(addresses);
        let mut bytes = [0u8; 512];
        let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
        memory
            .write(base + 16, b"sound/feedback/hit.wav\0")
            .unwrap();
        memory.write(base + 64, b"SOUND/A.WAV\0").unwrap();
        let wrap = if base == 0 { 512 } else { 0 };
        for ui in [false, true] {
            let mut services = fixture.engine(true);
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, base + 16 + wrap),
                Ok(0)
            );
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, base + 64 + wrap),
                Ok(positive as u64)
            );
            assert_eq!(
                (ENGINE_CALLS.registered_sound)(&services, 0),
                Ok(Some(default))
            );
            assert_eq!(
                (ENGINE_CALLS.registered_sound)(&services, positive),
                Ok(Some(expected))
            );
            assert_eq!((ENGINE_CALLS.registered_sound)(&services, -1), Ok(None));
            assert_eq!(
                (ENGINE_CALLS.registered_sound)(&services, i32::MAX),
                Ok(None)
            );
            let sounds =
                qa_core::configstrings::ConfigLayout::for_rules(RuleSetId::Quake3).resources[1];
            assert_eq!(
                (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, 0),
                Ok(None)
            );
        }
        assert_eq!(fixture.registrar.calls.len(), 4);
        assert!(
            fixture
                .registrar
                .calls
                .iter()
                .all(|(_, rules)| *rules == RuleSetId::Quake3)
        );
        let mut services = fixture.engine(false);
        assert_eq!(
            invoke(
                &mut services,
                &mut memory,
                addresses,
                false,
                base + 64 + wrap
            ),
            Ok(positive as u64)
        );
        drop(services);
        assert_eq!(fixture.registrar.calls.len(), 4);
    }
}

#[test]
fn invalid_native_sound_names_are_scoped_and_failures_publish_only_console_diagnostics() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let base = base(addresses);
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        memory.write(base + 16, b"sound/missing.wav\0").unwrap();
        memory.write(base + 64, &[b'x'; 64]).unwrap();
        memory.write(base + 128, &[0]).unwrap();
        memory.write(base + 511, b"x").unwrap();
        let wrap = if base == 0 { 512 } else { 0 };
        for ui in [false, true] {
            let mut services = fixture.engine(true);
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, 0),
                Err(CallError::Memory)
            );
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, base + 128 + wrap),
                Err(CallError::Text)
            );
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, base + 16 + wrap),
                Ok(0)
            );
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, base + 64 + wrap),
                Ok(0)
            );
            assert_eq!(
                invoke(&mut services, &mut memory, addresses, ui, base + 511 + wrap),
                Err(CallError::Memory)
            );
        }
        assert_eq!(fixture.registrar.calls.len(), 2);
        assert_eq!(fixture.server.events.rejected_publications(), 0);
        let mut batch = fixture.server.events.batch(fixture.output).unwrap();
        let mut prints = 0;
        while let Some(record) = fixture.server.events.next(&mut batch) {
            assert!(matches!(record.event, FrameEvent::Print(_)));
            prints += 1;
        }
        assert_eq!(prints, 4);
        let mut services = fixture.engine(false);
        assert_eq!(
            invoke(
                &mut services,
                &mut memory,
                addresses,
                false,
                base + 16 + wrap
            ),
            Ok(0)
        );
        drop(services);
        assert_eq!(fixture.registrar.calls.len(), 2);
    }
}

#[test]
fn nonzero_qvm_address_wrapped_to_zero_registers_data_while_raw_null_rejects() {
    let mut fixture = Fixture::new();
    let mut memory = ModuleMemory::load(0, 512, b"sound/feedback/hit.wav\0").unwrap();
    let mut services = fixture.engine(true);
    for ui in [false, true] {
        assert_eq!(
            invoke(
                &mut services,
                &mut memory,
                Addresses::Qvm { mask: 511 },
                ui,
                512
            ),
            Ok(0)
        );
        assert_eq!(
            invoke(
                &mut services,
                &mut memory,
                Addresses::Qvm { mask: 511 },
                ui,
                0
            ),
            Err(CallError::Memory)
        );
    }
}

#[test]
fn failed_attempts_keep_their_shared_engine_handle_order_after_freezing() {
    let mut fixture = Fixture::new();
    let before = fixture.registrar.bank.len();
    {
        let mut services = fixture.engine(true);
        for _ in 0..2 {
            assert_eq!(
                (ENGINE_CALLS.register_sound)(
                    &mut services,
                    b"sound/missing.wav",
                    RuleSetId::Quake3
                ),
                Ok(0)
            );
        }
    }
    assert_eq!(fixture.registrar.bank.len(), before + 1);
    assert_eq!(fixture.registrar.bank.registered_sound(3), None);
    let late = fixture.registrar.bank.attempt(b"sound/late.wav").unwrap();
    fixture
        .registrar
        .bank
        .admit(late, pcm(fixture.registrar.bank.rate))
        .unwrap();
    fixture
        .registrar
        .bank
        .attempt_registration(b"sound/late.wav", Some(b"sound/late.wav"))
        .unwrap();
    assert_eq!(
        fixture.registrar.bank.registered_handle(b"sound/late.wav"),
        Some(4)
    );
    let mut services = fixture.engine(false);
    assert_eq!(
        (ENGINE_CALLS.register_sound)(&mut services, b"sound/missing.wav", RuleSetId::Quake3),
        Ok(0)
    );
    assert_eq!((ENGINE_CALLS.registered_sound)(&services, 3), Ok(None));
    assert_eq!(
        (ENGINE_CALLS.registered_sound)(&services, 4),
        Ok(Some(late))
    );
}

#[test]
fn native_separator_aliases_keep_distinct_engine_handles_while_sharing_pcm() {
    let mut fixture = Fixture::new();
    let original = fixture
        .registrar
        .bank
        .registered_handle(b"sound/a.wav")
        .unwrap();
    let expected = fixture.registrar.bank.registered_sound(original).unwrap();
    let alias;
    {
        let mut services = fixture.engine(true);
        alias = (ENGINE_CALLS.register_sound)(&mut services, b"sound\\a.wav", RuleSetId::Quake3)
            .unwrap();
        assert_ne!(alias, original);
        assert_eq!(
            (ENGINE_CALLS.registered_sound)(&services, alias),
            Ok(Some(expected))
        );
    }
    let mut services = fixture.engine(false);
    assert_eq!(
        (ENGINE_CALLS.register_sound)(&mut services, b"SOUND/A.WAV", RuleSetId::Quake3),
        Ok(original)
    );
    assert_eq!(
        (ENGINE_CALLS.register_sound)(&mut services, b"SOUND\\A.WAV", RuleSetId::Quake3),
        Ok(alias)
    );
}

#[test]
fn cold_native_config_sound_uses_the_existing_numeric_cache_then_rebinds_frozen_rewrites() {
    use std::sync::Arc;
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut fixture = Fixture::native(rules);
        let sounds = qa_core::configstrings::ConfigLayout::for_rules(rules).resources[1];
        let first = fixture.registrar.bank.find_bytes(b"sound/a.wav").unwrap();
        let second = fixture.registrar.bank.find_bytes(b"sound/b.wav").unwrap();
        let raw = if rules == RuleSetId::Quake3 {
            b"sound/a.wav".as_slice()
        } else {
            b"a.wav"
        };
        let ordinal;
        {
            let mut services = fixture.engine(true);
            ordinal =
                (ENGINE_CALLS.resource_index)(&mut services, ModuleId(1), sounds, raw).unwrap();
            assert_eq!(ordinal, 1);
            assert_eq!(
                (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, 0),
                Ok(None)
            );
            assert_eq!(
                (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, ordinal as i32),
                Ok(Some(first))
            );
            assert_eq!(
                (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, ordinal as i32),
                Ok(Some(first))
            );
            (ENGINE_CALLS.sound)(
                &mut services,
                qa_core::primitives::NativeDelivery {
                    module: ModuleId(1),
                    source: rules,
                    target: qa_core::primitives::NativeTarget::All,
                    origin: qa_core::primitives::Vec3::default(),
                    reliable: false,
                    dupe_key: 0,
                    component: 0,
                    emission: None,
                    check_portals: false,
                },
                qa_core::primitives::SoundEvent {
                    sound: first,
                    source_ordinal: Some(ordinal),
                    volume: 1.0,
                    ..qa_core::primitives::SoundEvent::default()
                },
            )
            .unwrap();
        }
        assert_eq!(fixture.registrar.calls, vec![(raw.to_vec(), rules)]);
        let mut batch = fixture.server.events.batch(fixture.native_output).unwrap();
        let record = fixture.server.events.next(&mut batch).unwrap();
        let FrameEvent::Sound(event) = record.event else {
            panic!("cold sound was not published")
        };
        assert_eq!(event.sound, first);
        assert_eq!(event.source_ordinal, Some(ordinal));
        assert!(fixture.server.events.next(&mut batch).is_none());
        let bank = Arc::new(fixture.registrar.bank);
        fixture
            .storage
            .bind_sound_bank(ModuleId(1), sounds, Arc::clone(&bank))
            .unwrap();
        // Frozen native rewrites use the same per-row binder, with no registrar.
        let new_name = if rules == RuleSetId::Quake3 {
            b"sound/b.wav".as_slice()
        } else {
            b"b.wav"
        };
        let mut services = EngineServices {
            server: &mut fixture.server,
            callbacks: None,
            client_disconnect: None,
            input: &mut fixture.input,
            editor: None,
            cvars: &mut fixture.cvars,
            commands: &mut fixture.commands,
            client_connection: None,
            client_collision: None,
            renderer: None,
            scene: None,
            client_audio: None,
            vfs: &fixture.vfs,
            model_loader: None,
            model_lookup: None,
            sound_loader: None,
            sound_bank: Some(&bank),
            localization: &fixture.localization,
            registry: &fixture.registry,
            names: &mut fixture.names,
            storage: &mut fixture.storage,
            geometry: &fixture.geometry,
            world: None,
            native_geometry: None,
            foreign_trace: None,
            visibility: None,
            portals: None,
            scratch: &mut fixture.scratch,
        };
        let row = sounds.get(ordinal as usize).unwrap();
        (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), row, new_name).unwrap();
        assert_eq!(
            (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, ordinal as i32),
            Ok(Some(second))
        );
        (ENGINE_CALLS.configstring)(&mut services, ModuleId(1), row, b"").unwrap();
        assert_eq!(
            (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, ordinal as i32),
            Ok(None)
        );
    }
}

#[test]
fn cold_staged_native_sound_binds_before_group_publication_without_an_arc_bank() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut fixture = Fixture::native(rules);
        let expected = fixture.registrar.bank.find_bytes(b"sound/a.wav").unwrap();
        let sounds = qa_core::configstrings::ConfigLayout::for_rules(rules).resources[1];
        {
            let mut services = fixture.engine(true);
            assert_eq!(
                (ENGINE_CALLS.resource_index)(&mut services, ModuleId(1), sounds, b"a.wav"),
                Ok(1)
            );
            assert_eq!(services.storage.bound_sound(ModuleId(1), 1), None);
            // Sound followed by a print retains source order. Index zero does
            // not resolve the valid engine default handle into a config sound.
            for byte in [9, 0, 1, 9, 0, 0, 10, 2, b'p', 0] {
                (ENGINE_CALLS.message_bits)(&mut services, ModuleId(1), byte.into(), 8).unwrap();
            }
            let delivery = qa_core::primitives::NativeDelivery {
                module: ModuleId(1),
                source: rules,
                target: qa_core::primitives::NativeTarget::All,
                origin: qa_core::primitives::Vec3::default(),
                reliable: false,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            };
            assert!(
                (ENGINE_CALLS.native_message)(
                    &mut services,
                    delivery,
                    qa_gameplay::rules::sound_policy(RuleSetId::Quake3)
                )
                .unwrap()
                .is_some()
            );
            assert_eq!(services.storage.bound_sound(ModuleId(1), 1), Some(expected));
            assert_eq!(services.storage.bound_sound(ModuleId(1), 0), None);
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    &mut services,
                    delivery,
                    qa_gameplay::rules::sound_policy(RuleSetId::Quake3)
                ),
                Ok(None)
            );
        }
        assert_eq!(fixture.registrar.calls, vec![(b"a.wav".to_vec(), rules)]);
        assert_eq!(fixture.server.events.rejected_publications(), 1);
        let mut batch = fixture.server.events.batch(fixture.native_output).unwrap();
        let sound = fixture.server.events.next(&mut batch).unwrap();
        assert!(
            matches!(sound.event, FrameEvent::Sound(event) if event.sound == expected && event.source_ordinal == Some(1))
        );
        assert!(matches!(
            fixture.server.events.next(&mut batch).unwrap().event,
            FrameEvent::Print(_)
        ));
        assert!(fixture.server.events.next(&mut batch).is_none());
    }
}

#[test]
fn native_config_precache_cannot_claim_a_cgame_handle_for_a_different_file() {
    let mut fixture = Fixture::native(RuleSetId::Quake2);
    let native_sample = fixture.registrar.bank.find_bytes(b"sound/a.wav").unwrap();
    let root_sample = fixture.registrar.bank.attempt(b"a.wav").unwrap();
    fixture
        .registrar
        .bank
        .admit(root_sample, pcm(fixture.registrar.bank.rate))
        .unwrap();
    assert_ne!(native_sample, root_sample);
    assert_eq!(fixture.registrar.bank.registered_handle(b"a.wav"), None);
    let sounds = qa_core::configstrings::ConfigLayout::for_rules(RuleSetId::Quake2).resources[1];
    {
        let mut services = fixture.engine(true);
        assert_eq!(
            (ENGINE_CALLS.resource_index)(&mut services, ModuleId(1), sounds, b"a.wav"),
            Ok(1)
        );
        assert_eq!(
            (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, 1),
            Ok(Some(native_sample))
        );
    }
    assert_eq!(fixture.registrar.bank.registered_handle(b"a.wav"), None);
    let mut memory = ModuleMemory::load(0, 512, &[]).unwrap();
    memory.write(16, b"a.wav\0").unwrap();
    let handle;
    {
        let mut services = fixture.engine(true);
        handle = invoke(
            &mut services,
            &mut memory,
            Addresses::Qvm { mask: 511 },
            false,
            16,
        )
        .unwrap() as i32;
        assert_eq!(handle, 3);
        assert_eq!(
            (ENGINE_CALLS.registered_sound)(&services, handle),
            Ok(Some(root_sample))
        );
        assert_eq!(
            (ENGINE_CALLS.sound_id)(&mut services, ModuleId(1), sounds, 1),
            Ok(Some(native_sample))
        );
    }
    let mut services = fixture.engine(false);
    assert_eq!(
        invoke(
            &mut services,
            &mut memory,
            Addresses::Qvm { mask: 511 },
            true,
            16
        ),
        Ok(handle as u64)
    );
    assert_eq!(
        (ENGINE_CALLS.registered_sound)(&services, handle),
        Ok(Some(root_sample))
    );
}

struct AudioBoundary {
    client: ClientId,
    lifetime: EntityId,
    producer: ModuleId,
    listener: i32,
}
impl AudioBoundary {
    fn load(fixture: &mut Fixture) -> Self {
        let client = fixture
            .server
            .connect(qa_session::clients::Connection::Local, ModuleId(1), None)
            .unwrap();
        Self {
            client,
            lifetime: fixture.server.clients[client.0 as usize].entity,
            producer: ModuleId(1),
            listener: 0,
        }
    }
    fn emitter(&self, slot: i32) -> SoundEmitter {
        SoundEmitter::Native {
            namespace: ResourceNamespace::Client(self.client),
            slot,
        }
    }
}
impl ClientAudio for AudioBoundary {
    fn publish(
        &mut self,
        events: &mut EventRing,
        mut event: SoundEvent,
        native_entity: Option<i32>,
        _doppler_enabled: bool,
    ) -> Result<(), CallError> {
        if event.listener_owner != Some(self.producer) {
            return Err(CallError::Entity);
        }
        event.client = Some(self.client);
        event.client_lifetime = Some(self.lifetime);
        event.entity = native_entity
            .or_else(|| (event.bus == SoundBus::Local).then_some(self.listener))
            .map(|native| self.emitter(native));
        events
            .push_to(OutputTarget::Presentation, FrameEvent::Sound(event))
            .map(|_| ())
            .map_err(|_| CallError::Capacity)
    }
    fn respatialize(
        &mut self,
        events: &mut EventRing,
        producer: ModuleId,
        queued_at: EventTime,
        native_entity: u32,
        origin: Vec3,
        right: Vec3,
    ) -> Result<(), CallError> {
        let event = SoundEvent {
            action: SoundAction::Respatialize,
            queued_at,
            listener_owner: Some(producer),
            listener: Some(Listener {
                client: self.client,
                entity: Some(self.emitter(native_entity as i32)),
                origin,
                right,
                negate_dot: true,
            }),
            ..SoundEvent::default()
        };
        self.publish(events, event, Some(native_entity as i32), false)?;
        self.listener = native_entity as i32;
        Ok(())
    }
}
fn take_sound(fixture: &mut Fixture) -> SoundEvent {
    let mut batch = fixture.server.events.batch(fixture.output).unwrap();
    let record = fixture.server.events.next(&mut batch).unwrap();
    assert!(fixture.server.events.submit(
        fixture.output,
        record.sequence,
        OutputSubmission::BestEffort
    ));
    let FrameEvent::Sound(sound) = record.event else {
        panic!("expected CLIENT sound record")
    };
    sound
}

#[test]
fn native_client_sound_controls_use_one_ring_and_exact_widths_in_all_address_modes() {
    assert!(Q3_CLIENT.produces_client_audio());
    assert!(!Q3_UI.produces_client_audio());
    assert!(!qa_compat::abi::Q3_SERVER.produces_client_audio());
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let mut audio = AudioBoundary::load(&mut fixture);
        let client = audio.client;
        let lifetime = audio.lifetime;
        let default = fixture.registrar.bank.registered_sound(0).unwrap();
        let handle = fixture
            .registrar
            .bank
            .registered_handle(b"sound/a.wav")
            .unwrap();
        let sample = fixture.registrar.bank.registered_sound(handle).unwrap();
        let base = base(addresses);
        let wrap = if base == 0 { 512 } else { 0 };
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        memory
            .write_vec3(base + 16, Vec3([8.0, -3.0, 9.0]))
            .unwrap();
        memory.write_vec3(base + 32, Vec3([1.0, 2.0, 3.0])).unwrap();
        for (index, axis) in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .into_iter()
            .enumerate()
        {
            memory
                .write_vec3(base + 64 + index as u64 * 12, Vec3(axis))
                .unwrap();
        }
        let origin = base + 16 + wrap;
        let velocity = base + 32 + wrap;
        let axes = base + 64 + wrap;
        {
            let mut services = fixture.engine(false);
            services.client_audio = Some(&mut audio);
            for (number, arguments) in [
                (28, [0, 0, (-3i32) as u32 as u64, 0]),
                (
                    28,
                    [
                        origin,
                        (-12i32) as u32 as u64,
                        0x1234_0000_0007,
                        handle as u64,
                    ],
                ),
                (29, [handle as u64, (-9i32) as u32 as u64, 0, 0]),
                (30, [0, 0, 0, 0]),
                (31, [0, origin, velocity, handle as u64]),
                (32, [1023, origin, 0, 0]),
                (33, [2, origin, axes, u64::MAX]),
                (80, [1023, origin, velocity, handle as u64]),
                (81, [0, 0, 0, 0]),
                (30, [0x8000_0000, 0, 0, 0]),
            ] {
                assert_eq!(
                    invoke_call(
                        &mut services,
                        &mut memory,
                        addresses,
                        &Q3_CLIENT,
                        number,
                        &arguments
                    ),
                    Ok(0)
                );
            }
        }
        let dynamic = take_sound(&mut fixture);
        assert_eq!(dynamic.sound, default);
        assert_eq!(dynamic.source_ordinal, Some(0));
        assert!(!dynamic.positioned);
        assert_eq!(dynamic.channel, -3);
        assert_eq!(
            dynamic.entity,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: 0
            })
        );
        assert_eq!(dynamic.client_lifetime, Some(lifetime));
        assert_eq!(dynamic.listener_owner, Some(ModuleId(1)));
        assert_eq!(dynamic.queued_at, EventTime(123_456_789));
        assert_eq!(dynamic.spatial, context().sound_policy);
        assert_eq!(
            dynamic.playback,
            qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3)
        );
        let positioned = take_sound(&mut fixture);
        assert_eq!(positioned.sound, sample);
        assert_eq!(positioned.source_ordinal, Some(handle as u32));
        assert!(positioned.positioned);
        assert_eq!(positioned.position, Vec3([8.0, -3.0, 9.0]));
        assert_eq!(positioned.channel, 7);
        assert_eq!(
            positioned.entity,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: -12
            })
        );
        let local = take_sound(&mut fixture);
        assert_eq!(local.bus, SoundBus::Local);
        assert_eq!(local.channel, -9);
        assert_eq!(local.entity, dynamic.entity);
        assert_eq!(
            take_sound(&mut fixture).action,
            SoundAction::ClearTemporaryLoops
        );
        let temporary = take_sound(&mut fixture);
        assert_eq!(temporary.action, SoundAction::StartLoop);
        assert_eq!(temporary.looping.unwrap().lifetime, LoopLifetime::Temporary);
        assert!(temporary.looping.unwrap().latched);
        assert_eq!(temporary.velocity, Vec3([1., 2., 3.]));
        let position = take_sound(&mut fixture);
        assert_eq!(position.action, SoundAction::UpdateEmitterPosition);
        let spatialize = take_sound(&mut fixture);
        assert_eq!(spatialize.action, SoundAction::Respatialize);
        let listener = spatialize.listener.unwrap();
        assert_eq!(listener.client, client);
        assert_eq!(listener.origin, Vec3([8.0, -3.0, 9.0]));
        assert_eq!(listener.right, Vec3([0.0, 1.0, 0.0]));
        assert!(listener.negate_dot);
        let real = take_sound(&mut fixture);
        assert_eq!(real.looping.unwrap().lifetime, LoopLifetime::Persistent);
        assert!(real.looping.unwrap().latched);
        assert_eq!(real.velocity, Vec3([1., 2., 3.]));
        assert_eq!(take_sound(&mut fixture).action, SoundAction::StopLoop);
        assert_eq!(
            take_sound(&mut fixture).action,
            SoundAction::ClearClientLoops
        );
        assert!(fixture.registrar.calls.is_empty());
    }
}

#[test]
fn ui_local_sound_publishes_without_a_fabricated_client_or_listener() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let default = fixture.registrar.bank.registered_sound(0).unwrap();
        let mut memory = ModuleMemory::load(base(addresses), 512, &[]).unwrap();
        let mut services = fixture.engine(false);
        assert!(
            services
                .server
                .clients
                .iter()
                .all(|client| client.connection.is_none())
        );
        assert_eq!(
            invoke_call(
                &mut services,
                &mut memory,
                addresses,
                &Q3_UI,
                32,
                &[0, u64::MAX]
            ),
            Ok(0)
        );
        drop(services);
        let event = take_sound(&mut fixture);
        assert_eq!(event.sound, default);
        assert_eq!(event.channel, -1);
        assert_eq!(event.bus, SoundBus::Local);
        assert_eq!(event.client, None);
        assert_eq!(event.client_lifetime, None);
        assert_eq!(event.entity, None);
        assert_eq!(event.listener, None);
        assert_eq!(
            event.playback,
            qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3)
        );
    }
}

#[test]
fn client_audio_rejects_executed_bad_rows_pointers_and_stale_owner_without_partial_events() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let mut audio = AudioBoundary::load(&mut fixture);
        let base = base(addresses);
        let wrap = if base == 0 { 512 } else { 0 };
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        memory.write_vec3(base, Vec3([4.0, 5.0, 6.0])).unwrap();
        let mut services = fixture.engine(false);
        assert_eq!(
            invoke_call(
                &mut services,
                &mut memory,
                addresses,
                &Q3_CLIENT,
                29,
                &[0, 0]
            ),
            Err(CallError::Entity)
        );
        services.client_audio = Some(&mut audio);
        for (number, arguments, expected) in [
            (28, [0, u64::MAX, 0, 0], CallError::Entity),
            (28, [0, 1024, 0, 0], CallError::Entity),
            (31, [0, 0, base + 32 + wrap, 0], CallError::Memory),
            (
                80,
                [1024, base + 16 + wrap, base + 32 + wrap, 0],
                CallError::Entity,
            ),
            (32, [0, 0, 0, 0], CallError::Memory),
            (81, [u64::MAX, 0, 0, 0], CallError::Entity),
            (
                33,
                [0, base + 16 + wrap, base + 480 + wrap, 0],
                CallError::Memory,
            ),
        ] {
            assert_eq!(
                invoke_call(
                    &mut services,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    number,
                    &arguments
                ),
                Err(expected)
            );
        }
        drop(services);
        assert!(
            fixture
                .server
                .events
                .next(&mut fixture.server.events.batch(fixture.output).unwrap())
                .is_none()
        );
        audio.producer = ModuleId(2);
        let mut services = fixture.engine(false);
        services.client_audio = Some(&mut audio);
        assert_eq!(
            invoke_call(&mut services, &mut memory, addresses, &Q3_CLIENT, 30, &[0]),
            Err(CallError::Entity)
        );
        assert_eq!(
            invoke_call(
                &mut services,
                &mut memory,
                addresses,
                &Q3_CLIENT,
                33,
                &[0, base + 16 + wrap, base + 64 + wrap, 0]
            ),
            Err(CallError::Entity)
        );
        drop(services);
        assert!(
            fixture
                .server
                .events
                .next(&mut fixture.server.events.batch(fixture.output).unwrap())
                .is_none()
        );
    }
}

#[test]
fn unavailable_engine_handles_publish_diagnostics_only_and_qvm_wrapped_zero_is_real_origin() {
    let mut fixture = Fixture::new();
    let mut audio = AudioBoundary::load(&mut fixture);
    let mut memory = ModuleMemory::load(0, 512, &[]).unwrap();
    memory.write_vec3(0, Vec3([4.0, 5.0, 6.0])).unwrap();
    let addresses = Addresses::Qvm { mask: 511 };
    {
        let mut services = fixture.engine(false);
        services.client_audio = Some(&mut audio);
        for (number, arguments) in [
            (28, [16, u64::MAX, 0, u64::MAX]),
            (29, [i32::MAX as u64, 0, 0, 0]),
            (31, [0, 0, 0, u64::MAX]),
            (80, [0, 0, 0, i32::MAX as u64]),
        ] {
            assert_eq!(
                invoke_call(
                    &mut services,
                    &mut memory,
                    addresses,
                    &Q3_CLIENT,
                    number,
                    &arguments
                ),
                Ok(0)
            );
        }
    }
    let mut batch = fixture.server.events.batch(fixture.output).unwrap();
    let mut diagnostics = 0;
    while let Some(record) = fixture.server.events.next(&mut batch) {
        assert!(matches!(record.event, FrameEvent::Print(_)));
        assert!(fixture.server.events.submit(
            fixture.output,
            record.sequence,
            OutputSubmission::BestEffort
        ));
        diagnostics += 1;
    }
    assert_eq!(diagnostics, 4);
    {
        let mut services = fixture.engine(false);
        services.client_audio = Some(&mut audio);
        assert_eq!(
            invoke_call(
                &mut services,
                &mut memory,
                addresses,
                &Q3_CLIENT,
                28,
                &[512, u64::MAX, 0, 0]
            ),
            Ok(0)
        );
    }
    let event = take_sound(&mut fixture);
    assert!(event.positioned);
    assert_eq!(event.position, Vec3([4.0, 5.0, 6.0]));
    assert_eq!(event.entity, Some(audio.emitter(-1)));
    assert_eq!(fixture.server.events.rejected_publications(), 0);
}

#[test]
fn client_playback_keeps_distinct_registration_ordinals_for_shared_pcm_aliases() {
    for addresses in modes() {
        let mut fixture = Fixture::new();
        let sample = fixture.registrar.bank.find_bytes(b"sound/a.wav").unwrap();
        let first = fixture
            .registrar
            .bank
            .registered_handle(b"sound/a.wav")
            .unwrap();
        fixture
            .registrar
            .bank
            .attempt_registration(b"sound\\a.wav", Some(b"sound/a.wav"))
            .unwrap();
        let second = fixture
            .registrar
            .bank
            .registered_handle(b"sound\\a.wav")
            .unwrap();
        assert_ne!(first, second);
        let mut memory = ModuleMemory::load(base(addresses), 512, &[]).unwrap();
        {
            let mut services = fixture.engine(false);
            for handle in [first, second] {
                assert_eq!(
                    invoke_call(
                        &mut services,
                        &mut memory,
                        addresses,
                        &Q3_UI,
                        32,
                        &[handle as u64, 0]
                    ),
                    Ok(0)
                );
            }
        }
        let first_event = take_sound(&mut fixture);
        let second_event = take_sound(&mut fixture);
        assert_eq!(first_event.sound, sample);
        assert_eq!(second_event.sound, sample);
        assert_eq!(first_event.source_ordinal, Some(first as u32));
        assert_eq!(second_event.source_ordinal, Some(second as u32));
        assert!(fixture.registrar.calls.is_empty());
    }
}

#[test]
fn background_track_ordinals_publish_leased_paths_in_all_address_modes() {
    for addresses in modes() {
        for (table, start, stop) in [(&Q3_CLIENT, 35, 69), (&Q3_UI, 63, 62)] {
            let mut fixture = Fixture::new();
            let base = base(addresses);
            let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
            memory.write(base + 16, b"music/intro\0").unwrap();
            memory.write(base + 64, b"music/loop.wav\0").unwrap();
            let first = base + 16;
            let second = base + 64;
            assert_eq!(
                invoke_call(
                    &mut fixture.engine(false),
                    &mut memory,
                    addresses,
                    table,
                    start,
                    &[first, second]
                ),
                Ok(0)
            );
            let mut batch = fixture.server.events.batch(fixture.output).unwrap();
            let record = fixture.server.events.next(&mut batch).unwrap();
            let FrameEvent::Music(qa_core::primitives::MusicControl::Start {
                owner,
                playback,
                intro,
                loop_track,
            }) = record.event
            else {
                panic!("music start");
            };
            assert_eq!(owner.module, Some(context().module));
            assert_eq!(owner.client, None);
            assert_eq!(
                playback,
                qa_gameplay::rules::music_playback(RuleSetId::Quake3)
            );
            assert_eq!(
                fixture.server.events.texts.get(intro),
                Some(b"music/intro.wav".as_slice())
            );
            assert_eq!(
                fixture.server.events.texts.get(loop_track),
                Some(b"music/loop.wav".as_slice())
            );
            assert!(fixture.server.events.submit(
                fixture.output,
                record.sequence,
                OutputSubmission::BestEffort
            ));
            assert_eq!(fixture.server.events.texts.get(intro), None);
            assert_eq!(
                invoke_call(
                    &mut fixture.engine(false),
                    &mut memory,
                    addresses,
                    table,
                    start,
                    &[0, second]
                ),
                Ok(0)
            );
            assert!(fixture.server.events.next(&mut batch).is_none());
            assert_eq!(
                invoke_call(
                    &mut fixture.engine(false),
                    &mut memory,
                    addresses,
                    table,
                    stop,
                    &[]
                ),
                Ok(0)
            );
            let mut batch = fixture.server.events.batch(fixture.output).unwrap();
            assert!(matches!(
                fixture.server.events.next(&mut batch).unwrap().event,
                FrameEvent::Music(qa_core::primitives::MusicControl::Stop { only_active: true })
            ));
        }
    }
}

#[path = "support/renderer_requests.rs"]
mod renderer_requests;
