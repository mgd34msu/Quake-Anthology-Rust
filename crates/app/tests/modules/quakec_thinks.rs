use super::*;

#[path = "../model_assets/support.rs"]
mod model_support;

const FIELDS: &[(u16, u16, &[u8])] = &[
    (3, 0, b"origin"),
    (3, 3, b"velocity"),
    (3, 6, b"angles"),
    (3, 9, b"mins"),
    (3, 12, b"maxs"),
    (3, 15, b"size"),
    (3, 18, b"absmin"),
    (3, 21, b"absmax"),
    (2, 24, b"solid"),
    (2, 25, b"flags"),
    (4, 26, b"owner"),
    (1, 27, b"model"),
    (2, 28, b"modelindex"),
    (2, 29, b"frame"),
    (2, 30, b"skin"),
    (2, 31, b"effects"),
    (2, 32, b"takedamage"),
    (2, 33, b"colormap"),
    (2, 34, b"nextthink"),
    (6, 35, b"think"),
    (2, 36, b"movetype"),
    (3, 37, b"avelocity"),
];
const GLOBALS: &[(u16, u16, &[u8])] = &[
    (4, 34, b"self"),
    (2, 35, b"time"),
    (4, 36, b"other"),
    (2, 37, b"frametime"),
];

struct Code {
    operations: Vec<(qa_formats::program::quakec::Opcode, i16, i16, i16)>,
    functions: Vec<i32>,
}
impl Code {
    fn new() -> Self {
        Self {
            operations: vec![],
            functions: vec![0],
        }
    }
    fn function(&mut self, body: &[(qa_formats::program::quakec::Opcode, i16, i16, i16)]) -> u32 {
        let id = self.functions.len() as u32;
        self.functions.push(self.operations.len() as i32 + 1);
        self.operations.extend_from_slice(body);
        id
    }
    fn builtin(&mut self, number: i32) -> u32 {
        let id = self.functions.len() as u32;
        self.functions.push(-number);
        id
    }
    fn vm(&self) -> Vm {
        self.vm_fields(FIELDS)
    }
    fn vm_fields(&self, fields: &[(u16, u16, &[u8])]) -> Vm {
        let bytes = quakec_program::program_with_definitions(
            &self.operations,
            &self.functions,
            GLOBALS,
            fields,
            40,
            128,
        );
        let mut vm = Vm::load(
            Image::parse(&bytes, Some(5927)).unwrap(),
            Layout {
                entities: 64,
                header_bytes: 16,
                extra_string_bytes: 128,
                state_step: 0.1,
            },
        )
        .unwrap();
        vm.image.globals[48] = 1.0f32.to_bits();
        vm.image.globals[50] = 9.0f32.to_bits();
        vm.image.globals[53] = 0; // origin field
        vm.image.globals[54] = 34; // nextthink field
        vm.image.globals[55] = 35; // think field
        vm.image.globals[56] = 29; // frame field
        vm.image.globals[57] = 36; // movetype field
        vm
    }
}
fn write(vm: &mut Vm, native: i32, field: &[u8], word: u32) {
    let value = native as u32 * vm.stride as u32;
    let address = vm
        .field_address(value, vm.image.field(field).unwrap() as u32, 1)
        .unwrap();
    vm.entities.write_word(address, word as i32).unwrap();
}
fn write_vector(vm: &mut Vm, native: i32, field: &[u8], value: Vec3) {
    let at = vm
        .field_address(
            native as u32 * vm.stride as u32,
            vm.image.field(field).unwrap() as u32,
            3,
        )
        .unwrap();
    vm.entities.write_vec3(at, value).unwrap();
}
fn request(
    runtime: &mut Runtime,
    module: ModuleId,
    rules: RuleSetId,
    vm: Vm,
    entries: Vec<u32>,
    frame: u32,
    rate: u32,
    rows: &[(i32, f32, f32, u32)],
) -> (ModuleRequest, Vec<EntityId>) {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[anchor.slot as usize] =
        Some(NativeEntity { module, slot: 0 });
    let mut vm = vm;
    let mut ids = Vec::new();
    for &(native, movetype, nextthink, think) in rows {
        let entity = runtime
            .server
            .entities
            .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
            .unwrap()
            .id;
        runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
            module,
            slot: native,
        });
        for (field, value) in [
            (b"movetype".as_slice(), movetype.to_bits()),
            (b"nextthink", nextthink.to_bits()),
            (b"think", think),
        ] {
            if vm.image.field(field).is_some() {
                write(&mut vm, native, field, value);
            }
        }
        ids.push(entity);
    }
    let callback = entries.iter().position(|&entry| entry == frame).unwrap() as u32;
    (
        ModuleRequest {
            context: CallContext {
                native_rules: rules,
                trace_rules: RuleSetId::Quake3,
                sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
                server_frame: 0,
                module,
                clock: ThinkTime::module_clock(rules),
                console: Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
                allocation: AllocationPolicy::for_rules(rules, 0),
                link_order: qa_gameplay::rules::link_order(rules),
            },
            phase: Phase::Server,
            timing_rules: rules,
            rate: TickRate::fixed(rate).unwrap(),
            anchor,
            program: Program::quakec(vm, None),
            entries,
            frame: Export::clocked(CallbackId(callback)),
            prepare: vec![],
            initialize: vec![],
            api: None,
            shutdown: vec![],
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        },
        ids,
    )
}
fn load(runtime: Runtime, requests: Vec<ModuleRequest>) -> FrameHost {
    load_clients(runtime, requests, 1)
}
fn load_clients(
    mut runtime: Runtime,
    requests: Vec<ModuleRequest>,
    max_clients: usize,
) -> FrameHost {
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append(
            &format!("sv_maxclients {max_clients}\nsv_maxtic 1\n"),
            Context::default(),
        )
        .unwrap();
    console.execute_frame(&mut runtime);
    FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap()
}
fn tick(host: &mut FrameHost, milliseconds: u64) {
    host.frame(
        &mut Source {
            time: EventTime::from_milliseconds(milliseconds),
            polls: 0,
        },
        true,
    );
}
fn native_entity(host: &FrameHost, module: ModuleId, native: i32) -> EntityId {
    host.runtime
        .server
        .entities
        .active()
        .find(|entity| {
            host.runtime.server.entities.columns.native_entity[entity.slot as usize]
                == Some(NativeEntity {
                    module,
                    slot: native,
                })
        })
        .unwrap()
}

#[test]
fn cold_qc_ambients_reach_the_mixer_once_through_qw_resources_and_packets() {
    use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::{configstrings::ConfigLayout, loopback::Endpoint};
    use qa_network::commands::packet::Protocol;
    use std::{num::NonZeroU32, sync::Arc};

    struct AmbientSource {
        clock: Source,
        mixer: qa_audio::Mixer,
        sample: qa_core::primitives::SoundId,
        client: ClientId,
        lifetime: EntityId,
        count: usize,
        audible_blocks: usize,
    }
    impl FrameSource for AmbientSource {
        fn begin_frame(&mut self) -> Duration {
            self.clock.begin_frame()
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            self.clock.poll_events(queue);
        }
        fn wait_time(&mut self, remaining: Duration) -> Duration {
            self.clock.wait_time(remaining)
        }
        fn elapsed(&self) -> Duration {
            self.clock.elapsed()
        }
        fn present(&mut self) {}
        fn sound(&mut self, event: SoundEvent) -> bool {
            assert!(self.count < 2);
            assert_eq!(event.sound, self.sample);
            assert_eq!(event.action, SoundAction::StartLoop);
            assert_eq!(
                event.spatial,
                qa_gameplay::rules::sound_policy(RuleSetId::Quake)
            );
            assert_eq!(event.client, Some(self.client));
            assert!(event.positioned);
            assert_eq!(
                event.position,
                [Vec3([0.0, 64.0, 22.0]), Vec3([0.0, -64.0, 22.0])][self.count]
            );
            assert_eq!(
                event.entity,
                Some(SoundEmitter::Static {
                    namespace: ResourceNamespace::Client(self.client),
                    lifetime: self.lifetime,
                    slot: self.count as u32,
                })
            );
            self.count += 1;
            self.mixer.sound(event)
        }
        fn listeners(
            &mut self,
            listeners: &[Option<qa_audio::Listener>; SeatId::COUNT],
            volume: f32,
        ) {
            self.mixer.listen(listeners, volume);
        }
        fn mix_audio(
            &mut self,
            entities: &[Option<qa_app::client_scene::ClientEntities<'_>>; SeatId::COUNT],
        ) {
            qa_app::audio::respatialize(&mut self.mixer, entities);
            let mut pcm = [0; 16];
            assert!(self.mixer.paint(&mut pcm));
            self.audible_blocks += usize::from(pcm.iter().any(|&sample| sample != 0));
        }
    }

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let precache = code.builtin(19);
        let ambient = code.builtin(74);
        let initialize = code.function(&[
            (StoreS, 80, 4, 0),
            (Call1, 83, 0, 0),
            (StoreV, 86, 4, 0),
            (StoreS, 80, 7, 0),
            (StoreF, 48, 10, 0),
            (StoreF, 48, 13, 0),
            (Call4, 84, 0, 0),
            (StoreV, 89, 4, 0),
            (StoreS, 80, 7, 0),
            (StoreF, 48, 10, 0),
            (StoreF, 48, 13, 0),
            (Call4, 84, 0, 0),
            (Return, 1, 0, 0),
        ]);
        let frame = code.function(&[(Return, 1, 0, 0)]);
        let mut vm = code.vm();
        vm.image.globals[80] = vm.insert_string(b"ambient.wav").unwrap() as u32;
        vm.image.globals[83] = precache;
        vm.image.globals[84] = ambient;
        for (first, position) in [
            (86, Vec3([0.0, 64.0625, 22.0])),
            (89, Vec3([0.0, -64.0625, 22.0])),
        ] {
            vm.image.globals[first..first + 3].copy_from_slice(&position.0.map(f32::to_bits));
        }
        let module = ModuleId(1);
        let mut runtime = Runtime::load(1, []).unwrap();
        let client = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake2,
                    trace: RuleSetId::QuakeWorld,
                },
                Protocol::QuakeWorld28,
                0,
            )
            .unwrap();
        assert!(
            runtime
                .server
                .bind_client_native(client, NativeEntity { module, slot: 1 })
        );
        runtime.client_resources[client.0 as usize]
            .as_mut()
            .unwrap()
            .state
            .set_sound_policy(qa_gameplay::rules::sound_policy(RuleSetId::Quake));
        let (mut request, _) = request(
            &mut runtime,
            module,
            rules,
            vm,
            vec![initialize, frame],
            frame,
            10,
            &[],
        );
        request.configstrings = ConfigLayout::for_rules(rules).count;
        request.initialize = vec![Export::clocked(CallbackId(0))];
        let mut host = load(runtime, vec![request]);
        host.local_clients[0] = Some(client);
        host.initialize_modules(Phase::Server);
        assert_eq!(
            host.module_state(module),
            Some(qa_app::modules::State::Running)
        );
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        assert!(host.runtime.sound_bank.is_none());
        let cursor = host
            .runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .output
            .unwrap();
        let mut cold = host.runtime.server.events.batch(cursor).unwrap();
        while let Some(record) = host.runtime.server.events.next(&mut cold) {
            assert!(!matches!(record.event, FrameEvent::Sound(_)));
        }
        assert_eq!(
            host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .sound(1),
            None,
        );

        let rate = NonZeroU32::new(44100).unwrap();
        let bank = Arc::new(
            qa_audio::Bank::load(
                rate,
                vec![(
                    "sound/ambient.wav".to_owned(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![1000, -1000, 1000, -1000],
                        loop_start: Some(1),
                    },
                )],
            )
            .unwrap(),
        );
        let sound = bank.find("sound/ambient.wav").unwrap();
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        let mut initial = host.runtime.server.events.batch(cursor).unwrap();
        let mut starts = 0;
        let mut ambient_end = 0;
        while let Some(record) = host.runtime.server.events.next(&mut initial) {
            if let FrameEvent::Sound(event) = record.event {
                assert_eq!(event.action, SoundAction::StartLoop);
                assert_eq!(event.sound, sound);
                assert_eq!(event.source_ordinal, Some(1));
                assert_eq!(
                    event.spatial,
                    qa_gameplay::rules::sound_policy(RuleSetId::Quake2)
                );
                assert_eq!(record.delivery.unwrap().source, rules);
                assert!(record.delivery.unwrap().reliable);
                starts += 1;
                ambient_end = record.sequence + 1;
            }
        }
        assert_eq!(starts, 2);
        let mut source = AmbientSource {
            clock: Source {
                time: EventTime(0),
                polls: 0,
            },
            mixer: qa_audio::Mixer::load(Arc::clone(&bank), 4, 1),
            sample: sound,
            client,
            lifetime: host
                .runtime
                .server
                .clients
                .get(client.0 as usize)
                .unwrap()
                .entity,
            count: 0,
            audible_blocks: 0,
        };
        for tick in 0..=24 {
            source.clock.time = EventTime::from_milliseconds(tick * 25);
            let result = host.frame(&mut source, true);
            assert_eq!(result.output.native_unsupported, 0);
        }
        assert_eq!(source.count, 2);
        assert_eq!(source.mixer.counts.started, 2);
        assert_eq!(source.mixer.counts.replaced, 0);
        assert_eq!(source.mixer.counts.invalid, 0);
        assert!(source.audible_blocks > 1);
        assert!(source.mixer.counts.frames > 4);
        let counters = host.runtime.server.events.counters(cursor).unwrap();
        assert!(
            counters.acknowledged_records >= 3,
            "resource row and both loops require real native ACKs"
        );
        assert_eq!(counters.overflow, 0);
        assert_eq!(counters.resyncs, 0);
        assert_eq!(counters.retired_on_resync, 0);
        assert!(host.runtime.server.events.sequence_range().start >= ambient_end);
        assert_eq!(host.runtime.network.command_errors, 0);
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        assert_eq!(
            host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .sound(1),
            Some(sound)
        );

        // Binding and the ordinary publication retry must not replay signon loops.
        host.bind_sounds(&bank).unwrap();
        for tick in 25..=36 {
            source.clock.time = EventTime::from_milliseconds(tick * 25);
            host.frame(&mut source, true);
        }
        assert_eq!(source.count, 2);
        assert_eq!(source.mixer.counts.started, 2);
        assert_eq!(source.mixer.counts.replaced, 0);
        assert!(source.audible_blocks > 12);
    }
}

#[test]
fn cold_qc_styles_and_literal_sounds_reach_client_through_shared_publication() {
    use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::configstrings::ConfigLayout;
    use qa_network::commands::packet::Protocol;
    use std::{num::NonZeroU32, sync::Arc};

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for protocol in [
            Protocol::Quake2_34,
            Protocol::Quake2Repro1038,
            Protocol::Quake3_68,
        ] {
            let mut code = Code::new();
            let sound = code.builtin(19);
            let lightstyle = code.builtin(35);
            let initialize = code.function(&[
                (StoreS, 80, 4, 0),
                (Call1, 83, 0, 0),
                (StoreS, 81, 4, 0),
                (Call1, 83, 0, 0),
                (StoreF, 48, 4, 0),
                (StoreS, 82, 7, 0),
                (Call2, 84, 0, 0),
                (Return, 1, 0, 0),
            ]);
            let frame = code.function(&[(Return, 1, 0, 0)]);
            let mut vm = code.vm();
            vm.image.globals[80] = vm.insert_string(b"#foo.wav").unwrap() as u32;
            vm.image.globals[81] = vm.insert_string(b"*foo.wav").unwrap() as u32;
            vm.image.globals[82] = vm.insert_string(b"aM\xff").unwrap() as u32;
            vm.image.globals[83] = sound;
            vm.image.globals[84] = lightstyle;
            let mut runtime = Runtime::load(1, []).unwrap();
            let module = ModuleId(1);
            let client = runtime
                .connect_local(
                    SeatId::FIRST,
                    SpawnAnchor {
                        position: Vec3::default(),
                        angles: Vec3::default(),
                        entity: 0,
                        fixture_fallback: false,
                    },
                    ClientPolicy {
                        client: RuleSetId::Quake3,
                        movement: RuleSetId::Quake2,
                        trace: RuleSetId::QuakeWorld,
                    },
                    protocol,
                    0,
                )
                .unwrap();
            assert!(
                runtime
                    .server
                    .bind_client_native(client, NativeEntity { module, slot: 1 })
            );
            let (mut request, _) = request(
                &mut runtime,
                module,
                rules,
                vm,
                vec![initialize, frame],
                frame,
                10,
                &[],
            );
            request.configstrings = ConfigLayout::for_rules(rules).count;
            request.initialize = vec![Export::clocked(CallbackId(0))];
            let mut host = load(runtime, vec![request]);
            host.local_clients[0] = Some(client);
            host.initialize_modules(Phase::Server);
            assert_eq!(
                host.module_state(module),
                Some(qa_app::modules::State::Running)
            );
            assert_eq!(host.module_counts(module).unwrap().traps, 0);
            let rate = NonZeroU32::new(44100).unwrap();
            let bank = Arc::new(
                qa_audio::Bank::load(
                    rate,
                    ["sound/#foo.wav", "sound/*foo.wav"]
                        .map(|path| {
                            (
                                path.to_owned(),
                                Pcm {
                                    rate,
                                    channels: PcmChannels::Mono,
                                    samples: vec![1; 2],
                                    loop_start: None,
                                },
                            )
                        })
                        .into(),
                )
                .unwrap(),
            );
            host.runtime.sound_bank = Some(Arc::clone(&bank));
            host.bind_sounds(&bank).unwrap();
            let first_sound = protocol.config_layout().unwrap().resources[1].first as usize;
            let resources = host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap();
            assert!(
                resources
                    .state
                    .configs
                    .config(first_sound + 1)
                    .unwrap()
                    .text
                    .is_empty()
            );
            for milliseconds in [0, 100, 200, 300, 400] {
                tick(&mut host, milliseconds);
            }
            let resources = &host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state;
            for (ordinal, path) in [(1, "sound/#foo.wav"), (2, "sound/*foo.wav")] {
                assert_eq!(resources.sound(ordinal), bank.find(path));
                let expected = if protocol == Protocol::Quake3_68 {
                    path.as_bytes().to_vec()
                } else {
                    let mut name = vec![b'#'];
                    name.extend_from_slice(path.as_bytes());
                    name
                };
                assert_eq!(
                    resources
                        .configs
                        .config(first_sound + ordinal as usize)
                        .unwrap()
                        .text,
                    expected
                );
            }
            if let Some(index) = resources.layout.style_index(1) {
                assert_eq!(resources.configs.config(index).unwrap().text, b"aM\xff");
            } else {
                assert_eq!(protocol, Protocol::Quake3_68);
            }
            assert_eq!(resources.rejected, 0);
            assert_eq!(host.runtime.network.command_errors, 0);
            assert_eq!(host.module_counts(module).unwrap().traps, 0);
        }
    }
}

#[test]
fn cold_qc_static_entities_survive_free_and_reuse_through_native_packets_and_scene() {
    use qa_app::{
        client_policy::ClientPolicy,
        map::SpawnAnchor,
        model_assets::{ModelCatalogBuilder, ModelRequest},
    };
    use qa_core::configstrings::ConfigLayout;
    use qa_network::commands::packet::Protocol;
    use std::{num::NonZeroU32, sync::Arc};

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut files = model_support::Fixture::models();
        let mut assets = qa_render::Assets::load().unwrap();
        let palette = model_support::palette(&mut assets);
        let profile = ModelRequest::native("", rules, Some(palette)).profile();
        let mut code = Code::new();
        let precache = code.builtin(20);
        let make_static = code.builtin(69);
        let setmodel = code.builtin(3);
        let setorigin = code.builtin(2);
        let spawn = code.builtin(14);
        let initialize = code.function(&[
            (StoreS, 80, 4, 0),
            (Call1, 82, 0, 0),
            (Call0, 99, 0, 0),
            (StoreEnt, 1, 81, 0),
            (Return, 81, 0, 0),
        ]);
        let convert = code.function(&[
            (StoreEnt, 81, 4, 0),
            (StoreS, 80, 7, 0),
            (Call2, 84, 0, 0),
            (StoreEnt, 81, 4, 0),
            (StoreV, 86, 7, 0),
            (Call2, 85, 0, 0),
            (Address, 81, 94, 78),
            (StorePV, 89, 78, 0),
            (Address, 81, 95, 78),
            (StorePF, 96, 78, 0),
            (Address, 81, 97, 78),
            (StorePF, 98, 78, 0),
            (StoreEnt, 81, 4, 0),
            (Call1, 83, 0, 0),
            (Return, 1, 0, 0),
        ]);
        let frame = code.function(&[(Return, 1, 0, 0)]);
        let mut vm = code.vm();
        let stride = vm.stride as u32;
        let name = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
        vm.image.globals[80] = name;
        vm.image.globals[81] = 40 * vm.stride as u32;
        vm.image.globals[82] = precache;
        vm.image.globals[83] = make_static;
        vm.image.globals[84] = setmodel;
        vm.image.globals[85] = setorigin;
        vm.image.globals[86..89].copy_from_slice(&[
            33.125f32.to_bits(),
            (-4.25f32).to_bits(),
            88.125f32.to_bits(),
        ]);
        vm.image.globals[89..92].copy_from_slice(&[
            90.0f32.to_bits(),
            180.0f32.to_bits(),
            270.0f32.to_bits(),
        ]);
        vm.image.globals[94] = 6; // angles
        vm.image.globals[95] = 28; // modelindex: string lookup owns the model.
        vm.image.globals[96] = 255.0f32.to_bits();
        vm.image.globals[97] = 31; // effects: absent from native static records.
        vm.image.globals[98] = 8192.0f32.to_bits();
        vm.image.globals[99] = spawn;
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.vfs = std::mem::take(&mut files.vfs);
        runtime.model_loader =
            Some(ModelCatalogBuilder::new(assets, [profile], [], Default::default()).unwrap());
        let module = ModuleId(1);
        let client = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    client: RuleSetId::Quake,
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::Quake2,
                },
                Protocol::QuakeWorld28,
                0,
            )
            .unwrap();
        assert!(
            runtime
                .server
                .bind_client_native(client, NativeEntity { module, slot: 1 })
        );
        let cursor = runtime
            .server
            .clients
            .get(client.0 as usize)
            .unwrap()
            .output
            .unwrap();
        let (mut request, _) = request(
            &mut runtime,
            module,
            rules,
            vm,
            vec![initialize, convert, frame],
            frame,
            10,
            &[],
        );
        request.configstrings = ConfigLayout::for_rules(rules).count;
        request.initialize = vec![Export::clocked(CallbackId(0))];
        let mut host = load(runtime, vec![request]);
        host.local_clients[0] = Some(client);
        host.initialize_modules(Phase::Server);
        assert_eq!(
            host.module_state(module),
            Some(qa_app::modules::State::Running)
        );
        let ModuleResult::QuakeC(native) = host.module_counts(module).unwrap().last_result.unwrap()
        else {
            panic!("QuakeC spawn result")
        };
        let source = host
            .runtime
            .server
            .entities
            .find_native(
                NativeEntity {
                    module,
                    slot: (native[0] / stride) as i32,
                },
                None,
            )
            .unwrap();
        host.call_module_export(
            module,
            Export::clocked(CallbackId(1)),
            ThinkTime::Seconds(1.0),
        )
        .unwrap();
        assert!(host.runtime.server.entities.resolve(source).is_none());
        let replacement = host
            .runtime
            .server
            .entities
            .allocate(
                ThinkTime::Seconds(100.0),
                ModuleId(7),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        assert_eq!(replacement.slot, source.slot);
        assert_ne!(replacement.generation, source.generation);
        host.runtime.server.entities.columns.position[replacement.slot as usize] = Vec3([999.0; 3]);
        let paths: Vec<_> = host
            .model_precaches()
            .into_iter()
            .map(|(source, path)| (source, std::str::from_utf8(path).unwrap().to_owned()))
            .collect();
        for (source, path) in paths {
            host.runtime
                .model_loader
                .as_mut()
                .unwrap()
                .register_request(
                    &host.runtime.vfs,
                    ModelRequest::native(&path, source, Some(palette)),
                )
                .unwrap();
        }
        let (assets, catalog) = host.runtime.model_loader.take().unwrap().finish();
        host.set_model_catalog(Arc::new(catalog), [(client, profile, None)]);
        let bank = Arc::new(qa_audio::Bank::load(NonZeroU32::new(44100).unwrap(), vec![]).unwrap());
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
        let end = host.runtime.server.events.sequence_range().end;
        assert!(
            host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .static_entities()
                .is_empty()
        );
        for milliseconds in [0, 25, 50, 100, 200, 300] {
            tick(&mut host, milliseconds);
        }
        let resources = &host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state;
        assert_eq!(resources.static_entities().len(), 1);
        let retained = resources.static_entities()[0];
        assert_eq!(retained.position, Vec3([33.125, -4.25, 88.125]));
        assert_eq!(
            retained.appearance.models[0],
            ModelRef::native(ResourceNamespace::Client(client), 1)
        );
        assert_eq!(retained.appearance.native.effects, 0);
        assert_eq!(host.runtime.network.command_errors, 0);
        let counts = host.runtime.server.events.counters(cursor).unwrap();
        assert!(counts.acknowledged_records >= 2);
        assert_eq!(counts.resyncs, 0);
        assert!(host.runtime.server.events.sequence_range().start >= end);
        let mut frontend = qa_render::FrontEnd::load(Default::default()).unwrap();
        let mut frame = frontend.begin_frame([0; 4]).unwrap();
        let input = host.client_scene_inputs()[0].take().unwrap();
        let counts = input.append(&mut frame, &assets, &qa_render::Refdef::default(), false);
        assert_eq!(counts.models, 1);
        assert_eq!(counts.missing_models, 0);
        assert_eq!(frame.finish().entity(0).origin, retained.position);
        host.bind_sounds(&bank).unwrap();
        for milliseconds in [400, 500, 600] {
            tick(&mut host, milliseconds);
        }
        assert_eq!(
            host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .static_entities(),
            &[retained]
        );
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

#[test]
fn native_resources_initialize_through_runtime_and_setmodel_uses_frozen_catalog() {
    use qa_app::model_assets::{ModelCatalogBuilder, ModelRequest};
    use qa_compat::services::ModelLookup;

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut files = model_support::Fixture::models();
        let mut assets = qa_render::Assets::load().unwrap();
        let palette = model_support::palette(&mut assets);
        let profile = ModelRequest::native("", rules, Some(palette)).profile();
        let mut code = Code::new();
        let precache = code.builtin(20);
        let setmodel = code.builtin(3);
        let initialize = code.function(&[
            (StoreS, 80, 4, 0),
            (Call1, 82, 0, 0),
            (StoreEnt, 81, 4, 0),
            (StoreS, 80, 7, 0),
            (Call2, 83, 0, 0),
            (Return, 1, 0, 0),
        ]);
        let frame = code.function(&[
            (StoreEnt, 81, 4, 0),
            (StoreS, 80, 7, 0),
            (Call2, 83, 0, 0),
            (Return, 1, 0, 0),
        ]);
        let late = code.function(&[(StoreS, 80, 4, 0), (Call1, 82, 0, 0), (Return, 1, 0, 0)]);
        let mut vm = code.vm();
        let name = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
        vm.image.globals[80] = name;
        vm.image.globals[81] = 2 * vm.stride as u32;
        vm.image.globals[82] = precache;
        vm.image.globals[83] = setmodel;
        write_vector(&mut vm, 2, b"mins", Vec3([-2.0; 3]));
        write_vector(&mut vm, 2, b"maxs", Vec3([3.0; 3]));
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.vfs = std::mem::take(&mut files.vfs);
        runtime.model_loader =
            Some(ModelCatalogBuilder::new(assets, [profile], [], Default::default()).unwrap());
        let module = ModuleId(1);
        let (mut request, entities) = request(
            &mut runtime,
            module,
            rules,
            vm,
            vec![initialize, frame, late],
            frame,
            10,
            &[(2, 0.0, 0.0, 0)],
        );
        request.configstrings = 512;
        request.initialize = vec![Export::clocked(CallbackId(0))];
        let mut host = load(runtime, vec![request]);
        assert_eq!(
            host.module_state(module),
            Some(qa_app::modules::State::Pending)
        );
        host.initialize_modules(Phase::Server);
        assert_eq!(
            host.module_state(module),
            Some(qa_app::modules::State::Running)
        );
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        let slot = entities[0].slot as usize;
        let columns = &host.runtime.server.entities.columns;
        assert_eq!(
            columns.appearance[slot].models[0],
            ModelRef::native(ResourceNamespace::Module(module), 1)
        );
        let expected_mins = if rules == RuleSetId::Quake {
            Vec3([-16.0; 3])
        } else {
            Vec3([-2.0; 3])
        };
        assert_eq!(columns.mins[slot], expected_mins);
        let loaded = host
            .runtime
            .model_loader
            .as_ref()
            .unwrap()
            .bounds(b"progs/test.mdl", rules);
        assert_eq!(loaded.is_some(), rules == RuleSetId::Quake);
        let precaches = host.model_precaches();
        assert_eq!(precaches, [(rules, b"progs/test.mdl".as_slice())]);
        // The normal startup bridge registers every module's precaches into
        // this same builder, including QW's name-only registration.
        // Own the cold names only across the split Runtime borrow.
        let requests = precaches
            .into_iter()
            .map(|(source_rules, path)| {
                (std::str::from_utf8(path).unwrap().to_owned(), source_rules)
            })
            .collect::<Vec<_>>();
        for (path, source_rules) in &requests {
            host.runtime
                .model_loader
                .as_mut()
                .unwrap()
                .register_request(
                    &host.runtime.vfs,
                    ModelRequest::native(path, *source_rules, Some(palette)),
                )
                .unwrap();
        }
        let (_assets, models) = host.runtime.model_loader.take().unwrap().finish();
        assert_eq!(
            models.metadata(b"progs/test.mdl").unwrap().bounds,
            Bounds {
                mins: Vec3([-16.0; 3]),
                maxs: Vec3([16.0; 3]),
            }
        );
        host.set_model_catalog(std::sync::Arc::new(models), []);
        files.remove("progs/test.mdl");
        host.call_module_export(
            module,
            Export::clocked(CallbackId(1)),
            ThinkTime::Seconds(1.0),
        )
        .unwrap();
        assert_eq!(
            host.runtime.server.entities.columns.mins[slot],
            expected_mins
        );
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        assert!(
            host.call_module_export(
                module,
                Export::clocked(CallbackId(2)),
                ThinkTime::Seconds(1.0)
            )
            .is_err()
        );
        assert_eq!(host.module_counts(module).unwrap().traps, 1);
        assert!(host.runtime.model_loader.is_none());
    }
}

#[test]
fn state_reaches_provider_none_phase_with_native_clock_clear_and_world_context() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[
            (StoreEnt, 64, 34, 0),
            (State, 50, 51, 0),
            (Return, 73, 0, 0),
        ]);
        let think = code.function(&[
            (LoadF, 34, 54, 73),
            (StoreF, 35, 74, 0),
            (StoreEnt, 36, 75, 0),
            (Return, 73, 0, 0),
        ]);
        let mut vm = code.vm();
        vm.image.globals[51] = think;
        vm.image.globals[64] = 40 * vm.stride as u32;
        let mut runtime = Runtime::load(1, []).unwrap();
        // Callback order deliberately differs from the native function table.
        let (request, entities) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![think, 0, frame],
            frame,
            125,
            &[(40, 0.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 125);
        let now = if rules == RuleSetId::Quake {
            0.0f32
        } else {
            0.125f32
        };
        let due = (f64::from(now) + 0.1) as f32;
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0, due.to_bits(), 0]))
        );
        let slot = entities[0].slot as usize;
        assert_eq!(
            host.runtime.server.entities.columns.appearance[slot].frame,
            9
        );
        assert_eq!(host.runtime.server.entities.columns.next_think[slot], None);
        assert_eq!(
            host.runtime.server.entities.columns.think_function(slot),
            Some(CallbackId(0))
        );
    }
}

#[test]
fn nq_runs_once_and_qw_repeats_reschedules_through_the_same_dispatcher() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[
            (StoreEnt, 64, 34, 0),
            (State, 50, 51, 0),
            (Return, 73, 0, 0),
        ]);
        let first = code.function(&[(State, 50, 52, 0), (StoreF, 35, 74, 0), (Return, 73, 0, 0)]);
        let second = code.function(&[
            (LoadF, 34, 54, 73),
            (StoreF, 35, 74, 0),
            (StoreEnt, 36, 75, 0),
            (Return, 73, 0, 0),
        ]);
        let mut vm = code.vm();
        vm.image.globals[51] = first;
        vm.image.globals[52] = second;
        vm.image.globals[64] = 40 * vm.stride as u32;
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, entities) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![second, frame, first, 0],
            frame,
            250,
            &[(40, 0.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 250);
        let row = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!(row.calls, if rules == RuleSetId::Quake { 2 } else { 3 });
        let deadline = host.runtime.server.entities.columns.next_think[entities[0].slot as usize];
        if rules == RuleSetId::Quake {
            let due = (f64::from(0.1f32) + 0.1) as f32;
            assert_eq!(deadline, Some(ThinkTime::Seconds(f64::from(due))));
        } else {
            assert_eq!(deadline, None);
            let first_due = (f64::from(0.25f32) + 0.1) as f32;
            let second_due = (f64::from(first_due) + 0.1) as f32;
            assert_eq!(
                row.last_result,
                Some(ModuleResult::QuakeC([0, second_due.to_bits(), 0]))
            );
        }
    }
}

#[test]
fn native_edict_order_is_independent_of_common_slots_and_other_physics_is_not_scanned() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 73, 0, 0)]);
    let think = code.function(&[
        (AddF, 70, 48, 70),
        (Address, 34, 53, 61),
        (StorePF, 70, 61, 0),
        (Return, 70, 0, 0),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, entities) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![think, frame, 0],
        frame,
        125,
        &[
            (41, 0.0, 0.05, think),
            (40, 0.0, 0.05, think),
            (42, 4.0, 0.05, think), // STEP: think belongs after movement.
            (43, 6.0, 0.05, think), // TOSS: think precedes its own movement.
            (44, 7.0, 0.05, think), // PUSH: ltime controls the due phase.
        ],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime.server.entities.columns.position[entities[1].slot as usize].0[0],
        1.0
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[entities[0].slot as usize].0[0],
        2.0
    );
    for entity in &entities[2..] {
        assert_eq!(
            host.runtime.server.entities.columns.position[entity.slot as usize].0[0],
            0.0
        );
        assert_eq!(
            host.runtime.server.entities.columns.next_think[entity.slot as usize],
            Some(ThinkTime::Seconds(f64::from(0.05f32)))
        );
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
        3
    );
}

#[test]
fn later_spawn_runs_but_earlier_reused_native_ordinal_waits_for_next_pass() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[
            (If, 64, 6, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 64, 0),
            (StoreEnt, 1, 34, 0),
            (State, 50, 51, 0),
            (StoreEnt, 49, 34, 0),
            (Return, 70, 0, 0),
        ]);
        let first = code.function(&[
            (AddF, 70, 48, 70),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (State, 50, 52, 0),
            (Return, 70, 0, 0),
        ]);
        let second = code.function(&[
            (AddF, 70, 48, 70),
            (StoreEnt, 64, 4, 0),
            (Call1, 59, 0, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (State, 50, 60, 0),
            (Return, 70, 0, 0),
        ]);
        let reused = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
        let spawn = code.builtin(14);
        let remove = code.builtin(15);
        let mut vm = code.vm();
        for (global, value) in [
            (51, first),
            (52, second),
            (60, reused),
            (58, spawn),
            (59, remove),
        ] {
            vm.image.globals[global] = value;
        }
        let entries = (0..code.functions.len() as u32).rev().collect();
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, _) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            entries,
            frame,
            500,
            &[],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 500);
        let ordinal = if rules == RuleSetId::Quake { 2 } else { 33 };
        let earlier = native_entity(&host, ModuleId(1), ordinal);
        assert!(earlier.generation > 1);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([2.0f32.to_bits(), 0, 0]))
        );
        assert!(host.runtime.server.entities.columns.next_think[earlier.slot as usize].is_some());
        tick(&mut host, 1000);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 5);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([3.0f32.to_bits(), 0, 0]))
        );
    }
}

#[test]
fn independent_provider_rates_supply_native_start_end_time_and_qw_frametime() {
    let mut code = Code::new();
    let frame = code.function(&[
        (StoreF, 35, 73, 0),
        (StoreF, 37, 74, 0),
        (StoreEnt, 36, 75, 0),
        (Return, 73, 0, 0),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (nq, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame, 0],
        frame,
        100,
        &[],
    );
    let (qw, _) = request(
        &mut runtime,
        ModuleId(2),
        RuleSetId::QuakeWorld,
        code.vm(),
        vec![frame, 0],
        frame,
        50,
        &[],
    );
    let mut host = load(runtime, vec![nq, qw]);
    tick(&mut host, 1000);
    tick(&mut host, 1250);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 5);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.1f32.to_bits(),
            0.1f32.to_bits(),
            0
        ]))
    );
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.25f32.to_bits(),
            0.05f32.to_bits(),
            0
        ]))
    );
}

#[test]
fn full_width_invalid_native_function_rejects_only_its_due_entity() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let think = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame, think, 0],
        frame,
        125,
        &[(40, 0.0, 0.05, 0x0100_0001), (41, 0.0, 0.05, think)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .think_function(ids[0].slot as usize),
        Some(CallbackId(0x0100_0001))
    );
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
        None
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([1.0f32.to_bits(), 0, 0]))
    );
    tick(&mut host, 250);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
        Some(ThinkTime::Seconds(0.0))
    );
}

#[test]
fn valid_native_function_above_u16_binds_to_its_actual_session_callback() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    code.functions.resize(65_536, 0);
    let think = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    assert_eq!(think, 65_536);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame, think],
        frame,
        125,
        &[(40, 0.0, 0.05, think)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .think_function(ids[0].slot as usize),
        Some(CallbackId(1))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([1.0f32.to_bits(), 0, 0]))
    );
}

#[test]
fn state_with_missing_typed_think_field_traps_only_when_executed() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let state = code.function(&[(State, 50, 51, 0), (Return, 70, 0, 0)]);
    let fields = FIELDS
        .iter()
        .copied()
        .filter(|(_, _, name)| *name != b"think")
        .collect::<Vec<_>>();
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm_fields(&fields),
        vec![frame, state, 0],
        frame,
        125,
        &[],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(1)),
            ThinkTime::Seconds(0.125)
        ),
        Err(CallError::Rejected)
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
}

#[test]
fn absent_think_dormant_and_future_deadlines_do_not_execute_a_rejection() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let fields = FIELDS
        .iter()
        .copied()
        .filter(|(_, _, name)| *name != b"think")
        .collect::<Vec<_>>();
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm_fields(&fields),
        vec![frame, 0],
        frame,
        125,
        &[(40, 0.0, 0.0, 0), (41, 0.0, 0.5, 0)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    tick(&mut host, 500);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    tick(&mut host, 625);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    // The next export imports native zero after the rejected scheduled call.
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[1].slot as usize],
        Some(ThinkTime::Seconds(0.0))
    );
}

#[test]
fn future_callback_time_does_not_change_native_reuse_grace_or_strict_delay() {
    for (provider_now, due) in [(1.99f64, 2.01f32), (3.0, 3.01)] {
        let mut code = Code::new();
        let frame = code.function(&[
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (Address, 34, 54, 61),
            (StorePF, 66, 61, 0),
            (Address, 34, 55, 61),
            (StorePF, 51, 61, 0),
            (Return, 73, 0, 0),
        ]);
        let think = code.function(&[
            (StoreEnt, 34, 4, 0),
            (Call1, 59, 0, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 73, 0),
            (StoreF, 35, 74, 0),
            (Return, 73, 0, 0),
        ]);
        let spawn_export = code.function(&[(Call0, 58, 0, 0), (Return, 1, 0, 0)]);
        let spawn = code.builtin(14);
        let remove = code.builtin(15);
        let mut vm = code.vm();
        let stride = vm.stride as u32;
        for (global, value) in [(51, think), (58, spawn), (59, remove), (66, due.to_bits())] {
            vm.image.globals[global] = value;
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        let (mut request, _) = request(
            &mut runtime,
            ModuleId(1),
            RuleSetId::Quake,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            40,
            &[],
        );
        // Seed the native SERVER clock explicitly. The physical clock epoch
        // does not initialize sv.time or change the native reuse grace.
        request.context.clock = ThinkTime::Seconds(provider_now);
        let mut host = load(runtime, vec![request]);
        let start_ms = (provider_now * 1000.0) as u64;
        tick(&mut host, start_ms);
        tick(&mut host, start_ms + 40);
        let expected_ordinal = if provider_now < 2.0 { 2 } else { 3 };
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                expected_ordinal * stride,
                due.to_bits(),
                0
            ]))
        );
        if provider_now > 2.0 {
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(spawn_export)),
                ThinkTime::Seconds(3.5),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([4 * stride, due.to_bits(), 0]))
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(spawn_export)),
                ThinkTime::Seconds(3.500_001),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([2 * stride, due.to_bits(), 0]))
            );
        }
    }
}

#[test]
fn omitted_native_entry_clears_only_its_due_deadline_before_rejection() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let omitted = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame],
        frame,
        125,
        &[(40, 0.0, 0.05, omitted)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .think_function(ids[0].slot as usize),
        Some(CallbackId(omitted))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    tick(&mut host, 250);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
        Some(ThinkTime::Seconds(0.0))
    );
}

#[test]
fn native_qw_minimum_accumulates_and_maximum_clamps_physics_without_stopping_clock() {
    let mut code = Code::new();
    let frame = code.function(&[(StoreF, 35, 73, 0), (StoreF, 37, 74, 0), (Return, 73, 0, 0)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (mut request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::QuakeWorld,
        code.vm(),
        vec![frame, 0],
        frame,
        1,
        &[],
    );
    request.rate = TickRate::FrameDriven;
    let mut host = load(runtime, vec![request]);
    host.console
        .append("sv_maxtic 0.1\n", Context::default())
        .unwrap();
    host.console.execute_frame(&mut host.runtime);
    tick(&mut host, 0);
    tick(&mut host, 17);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    tick(&mut host, 34);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.034f32.to_bits(),
            0.034f32.to_bits(),
            0
        ]))
    );
    tick(&mut host, 284);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.284f32.to_bits(),
            0.1f32.to_bits(),
            0
        ]))
    );
}

#[test]
fn noclip_reads_think_pose_changes_and_uses_native_float_arithmetic() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let think = code.function(&[
            (Address, 34, 60, 61),
            (StorePV, 80, 61, 0),
            (Address, 34, 62, 61),
            (StorePV, 83, 61, 0),
            (Address, 34, 65, 61),
            (StorePV, 86, 61, 0),
            (Address, 34, 53, 61),
            (StorePV, 89, 61, 0),
            // The selected NOCLIP phase continues even if think changes
            // movetype. The next native pass will select STEP instead.
            (Address, 34, 57, 61),
            (StorePF, 71, 61, 0),
            (Return, 70, 0, 0),
        ]);
        let mut vm = code.vm();
        let origin = Vec3([1234.5679, -0.0, 0.25]);
        let velocity = Vec3([-23.125, 0.0, -3.125]);
        let angles = Vec3([-123.25, -0.0, 31.0]);
        let angular_velocity = Vec3([0.75, -0.0, 8.125]);
        for (at, value) in [
            (80, velocity),
            (83, angular_velocity),
            (86, angles),
            (89, origin),
        ] {
            vm.image.globals[at..at + 3].copy_from_slice(&value.0.map(f32::to_bits));
        }
        vm.image.globals[60] = 3; // velocity
        vm.image.globals[62] = 37; // avelocity
        vm.image.globals[65] = 6; // angles
        vm.image.globals[71] = 4.0f32.to_bits();
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.server.clients[0].player.movement_rules = RuleSetId::Quake3;
        runtime.server.clients[0].player.trace_rules = RuleSetId::Quake2;
        let (mut request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame, think, 0],
            frame,
            333,
            &[(40, 8.0, 0.05, think)],
        );
        request.rate = TickRate::FrameDriven;
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 333);
        let slot = ids[0].slot as usize;
        let vector_ma = |start: Vec3, speed: Vec3| {
            let step = if rules == RuleSetId::Quake {
                0.1f32
            } else {
                0.333f32
            };
            std::array::from_fn::<_, 3, _>(|axis| (start.0[axis] + step * speed.0[axis]).to_bits())
        };
        assert_eq!(
            host.runtime.server.entities.columns.position[slot]
                .0
                .map(f32::to_bits),
            vector_ma(origin, velocity)
        );
        assert_eq!(
            host.runtime.server.entities.columns.angles[slot]
                .0
                .map(f32::to_bits),
            vector_ma(angles, angular_velocity)
        );
        assert_eq!(
            host.runtime.server.entities.columns.velocity[slot],
            velocity
        );
        assert_eq!(
            host.runtime.server.entities.columns.appearance[slot].source,
            rules
        );
        assert!(host.runtime.server.area.bounds(ids[0]).is_some());
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
        let position = host.runtime.server.entities.columns.position[slot];
        tick(&mut host, 666);
        assert_eq!(
            host.runtime.server.entities.columns.position[slot],
            position
        );
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
            1
        );
    }
}

#[test]
fn noclip_reused_native_row_continues_on_new_common_lifetime() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let think = code.function(&[
            (AddF, 70, 48, 70),
            (StoreEnt, 34, 4, 0),
            (Call1, 59, 0, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (Address, 34, 53, 61),
            (StorePV, 80, 61, 0),
            (Address, 34, 60, 61),
            (StorePV, 83, 61, 0),
            (Address, 34, 57, 61),
            (StorePF, 71, 61, 0),
            (State, 50, 52, 0),
            (Return, 70, 0, 0),
        ]);
        let replacement_think = code.function(&[
            (AddF, 70, 48, 70),
            (Address, 34, 60, 61),
            (StorePV, 86, 61, 0),
            (Return, 70, 0, 0),
        ]);
        let initialize = code.function(&[
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (Address, 34, 57, 61),
            (StorePF, 71, 61, 0),
            (Address, 34, 54, 61),
            (StorePF, 66, 61, 0),
            (Address, 34, 55, 61),
            (StorePF, 51, 61, 0),
            (Return, 1, 0, 0),
        ]);
        let spawn = code.builtin(14);
        let remove = code.builtin(15);
        let mut vm = code.vm();
        vm.image.globals[58] = spawn;
        vm.image.globals[59] = remove;
        vm.image.globals[51] = think;
        vm.image.globals[52] = replacement_think;
        vm.image.globals[60] = 3;
        vm.image.globals[66] = 0.05f32.to_bits();
        vm.image.globals[71] = 8.0f32.to_bits();
        vm.image.globals[80..83].copy_from_slice(&[100.0f32.to_bits(), 0, 0]);
        vm.image.globals[83..86].copy_from_slice(&[8.0f32.to_bits(), 0, 0]);
        vm.image.globals[86..89].copy_from_slice(&[16.0f32.to_bits(), 0, 0]);
        let native = if rules == RuleSetId::Quake { 2 } else { 33 };
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, _) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            125,
            &[],
        );
        let mut host = load(runtime, vec![request]);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(initialize)),
            ThinkTime::Seconds(0.0),
        )
        .unwrap();
        // Existing namespace seeds borrow external lifetimes; removal/reuse
        // must start with a real QC spawn owned by the shared allocator.
        let old = native_entity(&host, ModuleId(1), native);
        tick(&mut host, 0);
        tick(&mut host, 125);
        assert!(host.runtime.server.entities.resolve(old).is_none());
        let replacement = native_entity(&host, ModuleId(1), native);
        assert_ne!(replacement, old);
        assert_eq!(
            host.runtime.server.entities.columns.position[replacement.slot as usize],
            Vec3([
                if rules == RuleSetId::Quake {
                    101.0
                } else {
                    102.0
                },
                0.0,
                0.0
            ])
        );
        assert!(host.runtime.server.area.bounds(replacement).is_some());
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().calls,
            if rules == RuleSetId::Quake { 3 } else { 4 }
        );
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                if rules == RuleSetId::Quake {
                    1.0f32
                } else {
                    2.0f32
                }
                .to_bits(),
                8.0f32.to_bits(),
                0
            ]))
        );
        tick(&mut host, 250);
        assert_eq!(
            host.runtime.server.entities.columns.position[replacement.slot as usize],
            Vec3([
                if rules == RuleSetId::Quake {
                    103.0
                } else {
                    104.0
                },
                0.0,
                0.0
            ])
        );
        assert!(host.runtime.server.area.bounds(replacement).is_some());
        assert!(host.runtime.server.entities.resolve(old).is_none());
    }
}

#[test]
fn noclip_zero_motion_explicitly_relinks_without_trigger_touch() {
    use qa_world::area::{LinkFlags, LinkIntent};
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let touch = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    let mut vm = code.vm();
    write(&mut vm, 40, b"solid", 2.0f32.to_bits());
    write(&mut vm, 41, b"solid", 1.0f32.to_bits());
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        vec![frame, touch, 0],
        frame,
        125,
        &[(40, 8.0, 0.0, 0), (41, 0.0, 0.0, 0)],
    );
    let mut host = load(runtime, vec![request]);
    host.runtime.server.entities.columns.touch[ids[1].slot as usize] = Some(CallbackId(1));
    for (entity, flags) in [(ids[0], LinkFlags::SOLID), (ids[1], LinkFlags::TRIGGER)] {
        assert!(host.runtime.server.area.link(
            &host.runtime.server.entities,
            entity,
            flags,
            LinkOrder::Tail,
            LinkIntent::Explicit,
        ));
    }
    let bounds = Bounds {
        mins: Vec3([-8.0; 3]),
        maxs: Vec3([8.0; 3]),
    };
    let linked = |host: &FrameHost| {
        host.runtime
            .server
            .area
            .query(&host.runtime.server.entities, bounds, LinkFlags::LINKED)
            .map(|row| row.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(linked(&host), ids);
    let old_bounds = host.runtime.server.area.bounds(ids[0]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(linked(&host), vec![ids[1], ids[0]]);
    assert_eq!(host.runtime.server.area.bounds(ids[0]), old_bounds);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([0; 3]))
    );
}

#[test]
fn noclip_classification_uses_native_rules_and_excludes_native_client_ordinals() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let client = if rules == RuleSetId::Quake { 1 } else { 32 };
        let mut vm = code.vm();
        for native in [client, 40, 41] {
            write_vector(&mut vm, native, b"velocity", Vec3([8.0, 0.0, 0.0]));
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame, 0],
            frame,
            125,
            &[(client, 8.0, 0.0, 0), (40, 8.9, 0.0, 0), (41, 0.9, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 125);
        let columns = &host.runtime.server.entities.columns;
        assert_eq!(columns.position[ids[0].slot as usize], Vec3::default());
        assert_eq!(columns.position[ids[2].slot as usize], Vec3::default());
        assert_eq!(
            columns.position[ids[1].slot as usize],
            if rules == RuleSetId::QuakeWorld {
                Vec3([1.0, 0.0, 0.0])
            } else {
                Vec3::default()
            }
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
            if rules == RuleSetId::QuakeWorld { 1 } else { 3 }
        );
    }
}

#[test]
fn native_client_physics_reservations_survive_maxclients_changes() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let think = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
        let last_client = if rules == RuleSetId::Quake { 2 } else { 32 };
        let mover = last_client + 1;
        let thinker = mover + 1;
        let mut vm = code.vm();
        for native in [1, last_client, mover] {
            write_vector(&mut vm, native, b"velocity", Vec3([8.0, 0.0, 0.0]));
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame, think, 0],
            frame,
            125,
            &[
                (1, 8.0, 0.0, 0),
                (last_client, 8.0, 0.0, 0),
                (mover, 8.0, 0.0, 0),
                (thinker, 0.0, 0.05, think),
            ],
        );
        let mut host = load_clients(runtime, vec![request], 2);
        for (milliseconds, max_clients) in [(125, 0), (250, 63)] {
            host.console
                .append(
                    &format!("sv_maxclients {max_clients}\n"),
                    Context::default(),
                )
                .unwrap();
            host.console.execute_frame(&mut host.runtime);
            if milliseconds == 125 {
                tick(&mut host, 0);
            }
            tick(&mut host, milliseconds);
            let columns = &host.runtime.server.entities.columns;
            for &client in &ids[..2] {
                assert_eq!(columns.position[client.slot as usize], Vec3::default());
            }
            assert_eq!(
                columns.position[ids[2].slot as usize],
                Vec3([milliseconds as f32 / 125.0, 0.0, 0.0])
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
        }
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([1.0f32.to_bits(), 0, 0]))
        );
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
            4
        );
    }
}

#[test]
fn missing_angular_velocity_rejects_only_executed_noclip() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let think = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    let fields = FIELDS
        .iter()
        .copied()
        .filter(|(_, _, name)| *name != b"avelocity")
        .collect::<Vec<_>>();
    let mut vm = code.vm_fields(&fields);
    write_vector(&mut vm, 41, b"velocity", Vec3([8.0, 0.0, 0.0]));
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        vec![frame, think, 0],
        frame,
        125,
        &[(40, 0.0, 0.05, think), (41, 8.0, 0.0, 0)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
        0
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[ids[1].slot as usize],
        Vec3::default()
    );
    assert!(host.runtime.server.area.bounds(ids[1]).is_none());
}

#[test]
fn rejected_noclip_think_stops_only_its_entity_before_motion() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let mut vm = code.vm();
        for native in [40, 41] {
            write_vector(&mut vm, native, b"velocity", Vec3([8.0, 0.0, 0.0]));
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame, 0],
            frame,
            125,
            &[(40, 8.0, 0.05, 0x0100_0001), (41, 8.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 125);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize],
            Vec3::default()
        );
        assert!(host.runtime.server.area.bounds(ids[0]).is_none());
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[1].slot as usize],
            Vec3([1.0, 0.0, 0.0])
        );
        assert!(host.runtime.server.area.bounds(ids[1]).is_some());
    }
}

#[test]
fn noclip_qw_motion_uses_only_accepted_accumulated_and_clamped_span() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let mut vm = code.vm();
    write_vector(&mut vm, 40, b"velocity", Vec3([10.0, 0.0, 0.0]));
    let mut runtime = Runtime::load(1, []).unwrap();
    let (mut request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::QuakeWorld,
        vm,
        vec![frame, 0],
        frame,
        1,
        &[(40, 8.0, 0.0, 0)],
    );
    request.rate = TickRate::FrameDriven;
    let mut host = load(runtime, vec![request]);
    host.console
        .append("sv_maxtic 0.1\n", Context::default())
        .unwrap();
    host.console.execute_frame(&mut host.runtime);
    tick(&mut host, 0);
    tick(&mut host, 17);
    assert_eq!(
        host.runtime.server.entities.columns.position[ids[0].slot as usize],
        Vec3::default()
    );
    tick(&mut host, 34);
    let first = 0.034f32 * 10.0;
    assert_eq!(
        host.runtime.server.entities.columns.position[ids[0].slot as usize].0[0].to_bits(),
        first.to_bits()
    );
    tick(&mut host, 284);
    let after_stall = first + 0.1f32 * 10.0;
    assert_eq!(
        host.runtime.server.entities.columns.position[ids[0].slot as usize].0[0].to_bits(),
        after_stall.to_bits()
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
}
