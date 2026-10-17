use super::*;

#[path = "quakec_client_physics.rs"]
mod client_physics;

#[path = "quakec_userinfo.rs"]
mod userinfo;

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
    fn motion_vm(&self) -> Vm {
        let mut fields = FIELDS.to_vec();
        fields.extend_from_slice(&[
            (2, 40, b"gravity"),
            (2, 41, b"waterlevel"),
            (2, 42, b"watertype"),
            (4, 43, b"groundentity"),
            (6, 44, b"touch"),
        ]);
        self.vm_fields(&fields)
    }
    fn push_vm(&self) -> Vm {
        let mut fields = FIELDS.to_vec();
        fields.extend_from_slice(&[
            (2, 40, b"gravity"),
            (2, 41, b"waterlevel"),
            (2, 42, b"watertype"),
            (4, 43, b"groundentity"),
            (6, 44, b"touch"),
            (2, 45, b"ltime"),
            (6, 46, b"blocked"),
        ]);
        self.vm_fields(&fields)
    }
    fn push_links(&mut self) -> (u32, u32) {
        let size = self.builtin(4);
        let link = self.function(&[
            (StoreEnt, 80, 4, 0),
            (StoreV, 106, 7, 0),
            (StoreV, 109, 10, 0),
            (Call3, 83, 0, 0),
            (StoreEnt, 81, 4, 0),
            (StoreV, 100, 7, 0),
            (StoreV, 103, 10, 0),
            (Call3, 83, 0, 0),
            (StoreEnt, 82, 4, 0),
            (StoreV, 100, 7, 0),
            (StoreV, 103, 10, 0),
            (Call3, 83, 0, 0),
            (StoreEnt, 86, 4, 0),
            (StoreV, 100, 7, 0),
            (StoreV, 103, 10, 0),
            (Call3, 83, 0, 0),
            (Return, 0, 0, 0),
        ]);
        (size, link)
    }
    fn goal_vm(&self) -> Vm {
        let mut fields = FIELDS.to_vec();
        fields.extend_from_slice(&[
            (4, 40, b"goalentity"),
            (4, 41, b"enemy"),
            (2, 42, b"ideal_yaw"),
            (2, 43, b"yaw_speed"),
            (6, 44, b"touch"),
        ]);
        self.vm_fields(&fields)
    }
    fn vm_fields(&self, fields: &[(u16, u16, &[u8])]) -> Vm {
        let entity_fields = fields
            .iter()
            .map(|(kind, offset, _)| usize::from(*offset) + usize::from(*kind == 3) * 2 + 1)
            .max()
            .unwrap_or(1);
        let bytes = quakec_program::program_with_definitions(
            &self.operations,
            &self.functions,
            GLOBALS,
            fields,
            entity_fields,
            128,
        );
        let mut vm = Vm::load(
            Image::parse(&bytes, Some(5927)).unwrap(),
            Layout {
                entities: 64,
                header_bytes: 16,
                extra_string_bytes: 128,
                state_step: 0.1,
                random_seed: 1,
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

fn install_motion_world(runtime: &mut Runtime, floor: bool) -> GeometryId {
    install_motion_models(runtime, floor, false)
}

fn install_motion_models(runtime: &mut Runtime, floor: bool, pusher: bool) -> GeometryId {
    use qa_app::WorldCollision;
    use qa_world::collision::{
        Contents,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
    };

    let (mut planes, mut brushes, mut surfaces, brush_span, mut leaf_brushes) = if floor {
        let planes = [
            (Vec3([1.0, 0.0, 0.0]), 512.0),
            (Vec3([-1.0, 0.0, 0.0]), 512.0),
            (Vec3([0.0, 1.0, 0.0]), 512.0),
            (Vec3([0.0, -1.0, 0.0]), 512.0),
            (Vec3([0.0, 0.0, 1.0]), 0.0),
            (Vec3([0.0, 0.0, -1.0]), 64.0),
        ]
        .map(|(normal, distance)| Plane::oriented(normal, distance))
        .to_vec();
        (
            planes,
            vec![Brush {
                sides: IndexSpan { first: 0, count: 6 },
                contents: Contents::SOLID,
            }],
            vec![SurfaceFlags::default(); 6],
            IndexSpan { first: 0, count: 1 },
            vec![0],
        )
    } else {
        (vec![], vec![], vec![], IndexSpan::default(), vec![])
    };
    let mut leaves = vec![CollisionLeaf {
        stored_contents: None,
        brushes: brush_span,
        patches: Default::default(),
    }];
    let mut models = vec![ModelRoot::Leaf(0)];
    let mut bounds = vec![Bounds {
        mins: Vec3([-512.0; 3]),
        maxs: Vec3([512.0; 3]),
    }];
    if pusher {
        let first = planes.len() as u32;
        for normal in [
            Vec3([1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, 1.0, 0.0]),
            Vec3([0.0, -1.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
            Vec3([0.0, 0.0, -1.0]),
        ] {
            planes.push(Plane::oriented(normal, 4.0));
            surfaces.push(SurfaceFlags::default());
        }
        let brush = brushes.len() as u32;
        brushes.push(Brush {
            sides: IndexSpan { first, count: 6 },
            contents: Contents::SOLID,
        });
        let first = leaf_brushes.len() as u32;
        leaf_brushes.push(brush);
        leaves.push(CollisionLeaf {
            stored_contents: None,
            brushes: IndexSpan { first, count: 1 },
            patches: Default::default(),
        });
        models.push(ModelRoot::Leaf(1));
        bounds.push(Bounds {
            mins: Vec3([-4.0; 3]),
            maxs: Vec3([4.0; 3]),
        });
    }
    let geometry = runtime
        .geometry
        .load_brushes(
            planes,
            brushes,
            Vec::new(),
            SurfaceTable::flags(surfaces),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves,
                leaf_brushes,
                models,
                leaf_patches: Vec::new(),
            },
            bounds,
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    geometry
}

fn install_water_world(runtime: &mut Runtime) -> GeometryId {
    use qa_app::WorldCollision;
    use qa_world::collision::{
        Contents,
        brushes::{BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
    };
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: Some(Contents::WATER),
                    brushes: IndexSpan::default(),
                    patches: Default::default(),
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0)],
                leaf_patches: Vec::new(),
            },
            vec![Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    geometry
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
            q3_image: None,
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
    use std::num::NonZeroU32;

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
        fn sound(&mut self, _bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
            if event.action == SoundAction::RetireClient {
                return true;
            }
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
            self.mixer.sound(_bank.unwrap(), event)
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
            _bank: Option<&qa_audio::Bank>,
            entities: &[Option<qa_app::client_scene::ClientEntities<'_>>; SeatId::COUNT],
        ) {
            qa_app::audio::respatialize(&mut self.mixer, entities);
            let mut pcm = [0; 16];
            assert!(self.mixer.paint(_bank.unwrap(), &mut pcm));
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
                    input: RuleSetId::Quake3,
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
            .set_sound_policy(
                qa_gameplay::rules::sound_policy(RuleSetId::Quake),
                qa_gameplay::rules::loop_spatial_policy(RuleSetId::Quake),
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
        assert!(host.runtime.sound_assets.is_none());
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
        let bank = qa_audio::Bank::load(
            rate,
            vec![(
                "sound/ambient.wav".to_owned(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![1000, -1000, 1000, -1000],
                    loop_start: Some(1),
                    loop_end: None,
                    loop_invalid: false,
                },
            )],
        )
        .unwrap();
        let sound = bank.find("sound/ambient.wav").unwrap();
        host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
        host.bind_sounds().unwrap();
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
            mixer: qa_audio::Mixer::load(rate, 4, 1),
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
        host.bind_sounds().unwrap();
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
    use std::num::NonZeroU32;

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
            vm.image.globals[82] = vm.insert_string(b"aM\x80").unwrap() as u32;
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
                        input: RuleSetId::Quake3,
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
            let bank = qa_audio::Bank::load(
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
                                loop_end: None,
                                loop_invalid: false,
                            },
                        )
                    })
                    .into(),
            )
            .unwrap();
            host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
            host.bind_sounds().unwrap();
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
                assert_eq!(
                    resources.sound(ordinal),
                    host.runtime
                        .sound_assets
                        .as_deref()
                        .unwrap()
                        .bank()
                        .find(path)
                );
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
                assert_eq!(resources.configs.config(index).unwrap().text, b"aM\x80");
            } else {
                assert_eq!(protocol, Protocol::Quake3_68);
            }
            assert!(
                resources
                    .configs
                    .config(first_sound + 3)
                    .unwrap()
                    .text
                    .is_empty(),
                "the absent default PCM must not claim a CLIENT sound ordinal"
            );
            assert_eq!(host.runtime.sound_assets.as_ref().unwrap().reads, 0);
            assert_eq!(resources.rejected, 0, "{rules:?} -> {protocol:?}");
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
        model_assets::{ModelAssets, ModelRequest},
    };
    use qa_core::configstrings::ConfigLayout;
    use qa_network::commands::packet::Protocol;
    use std::num::NonZeroU32;

    for (rules, receive_while_loading) in [RuleSetId::Quake, RuleSetId::QuakeWorld]
        .into_iter()
        .flat_map(|rules| [false, true].map(|loading| (rules, loading)))
    {
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
        runtime.model_assets = Some(
            ModelAssets::new(&runtime.vfs, assets, [profile], [], Default::default()).unwrap(),
        );
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
                    input: RuleSetId::Quake,
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
                .model_assets
                .as_mut()
                .unwrap()
                .register_request(
                    &host.runtime.vfs,
                    ModelRequest::native(&path, source, Some(palette)),
                )
                .unwrap();
        }
        let bank = qa_audio::Bank::load(NonZeroU32::new(44100).unwrap(), vec![]).unwrap();
        host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
        host.bind_sounds().unwrap();
        let end = host.runtime.server.events.sequence_range().end;
        assert!(
            host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .static_entities()
                .is_empty()
        );
        if receive_while_loading {
            for milliseconds in [0, 25, 50, 100, 200, 300] {
                tick(&mut host, milliseconds);
            }
            let builder = host.runtime.model_assets.as_ref().unwrap();
            if rules == RuleSetId::Quake {
                use qa_compat::services::ModelLookup;
                assert!(
                    builder
                        .catalog()
                        .model_handle(b"progs/test.mdl", RuleSetId::QuakeWorld)
                        .is_none()
                );
            }
            let retained = host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .static_entities();
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0].appearance.source, RuleSetId::QuakeWorld);
        }
        host.bind_client_model_assets([(client, profile, None)]);
        for milliseconds in if receive_while_loading {
            [325, 350, 375, 400, 425, 450]
        } else {
            [0, 25, 50, 100, 200, 300]
        } {
            tick(&mut host, milliseconds);
        }
        let resources = &host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state;
        assert_eq!(resources.static_entities().len(), 1);
        let retained = resources.static_entities()[0].record;
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
        let counts = input.append(
            &mut frame,
            host.runtime.model_assets.as_ref().unwrap().assets(),
            &qa_render::Refdef::default(),
            false,
            None,
        );
        assert_eq!(counts.models, 1);
        assert_eq!(counts.missing_models, 0);
        assert_eq!(frame.finish().entity(0).origin, retained.position);
        host.bind_sounds().unwrap();
        for milliseconds in [500, 600, 700] {
            tick(&mut host, milliseconds);
        }
        let after = host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .static_entities();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].record, retained);
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

#[test]
fn native_resources_initialize_through_runtime_and_setmodel_uses_frozen_catalog() {
    use qa_app::model_assets::{ModelAssets, ModelRequest};
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
        runtime.model_assets = Some(
            ModelAssets::new(&runtime.vfs, assets, [profile], [], Default::default()).unwrap(),
        );
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
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
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
                .model_assets
                .as_mut()
                .unwrap()
                .register_request(
                    &host.runtime.vfs,
                    ModelRequest::native(path, *source_rules, Some(palette)),
                )
                .unwrap();
        }
        let models = host.runtime.model_assets.as_ref().unwrap().catalog();
        assert_eq!(
            models.metadata(b"progs/test.mdl").unwrap().bounds,
            Bounds {
                mins: Vec3([-16.0; 3]),
                maxs: Vec3([16.0; 3]),
            }
        );
        host.bind_client_model_assets([]);
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
        assert!(
            !host
                .runtime
                .model_assets
                .as_ref()
                .unwrap()
                .initial_registration()
        );
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
        assert_eq!(
            host.runtime.server.entities.columns.next_think[slot],
            Some(ThinkTime::Seconds(0.0))
        );
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
            assert_eq!(deadline, Some(ThinkTime::Seconds(0.0)));
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
            (43, 3.0, 0.05, think), // WALK belongs to a later motion slice.
            (44, 7.0, 0.05, think), // PUSH executes its own phase and rejects missing ltime.
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
        1
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
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
        Some(ThinkTime::Seconds(0.0))
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
    host.execute_commands();
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
            // movetype. The next native pass selects PUSH.
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
        vm.image.globals[71] = 7.0f32.to_bits();
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
            0
        );
        // The next captured PUSH phase executes and rejects missing ltime.
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    }
}

#[test]
fn noclip_native_vector_aliases_keep_component_and_phase_write_order() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let read_origin = code.function(&[(LoadV, 80, 81, 70), (Return, 70, 0, 0)]);
        let read_angles = code.function(&[(LoadV, 80, 82, 70), (Return, 70, 0, 0)]);
        let fields = FIELDS
            .iter()
            .map(|&(kind, offset, name)| {
                let offset = match name {
                    b"origin" => 1,
                    b"velocity" => 0,
                    b"avelocity" => 5,
                    _ => offset,
                };
                (kind, offset, name)
            })
            .collect::<Vec<_>>();
        let mut vm = code.vm_fields(&fields);
        let initial = [4.0f32, -3.25, 0.125, 9.0, 0.0, -5.0, 2.0, -7.0, 1.0];
        let native = 40u32 * vm.stride as u32;
        for (word, value) in initial.iter().copied().enumerate() {
            let address = vm.field_address(native, word as u32, 1).unwrap();
            vm.entities
                .write_word(address, value.to_bits() as i32)
                .unwrap();
        }
        vm.image.globals[80] = native;
        vm.image.globals[81] = 1;
        vm.image.globals[82] = 6;
        let mut expected = initial;
        // WinQuake/QW VectorMA reads and writes each axis before the next.
        // These legal field definitions overlap within each vector operation.
        for axis in 0..3 {
            expected[6 + axis] = expected[6 + axis] + 0.125f32 * expected[5 + axis];
        }
        for axis in 0..3 {
            expected[1 + axis] = expected[1 + axis] + 0.125f32 * expected[axis];
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame, read_origin, read_angles, 0],
            frame,
            125,
            &[(40, 8.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 125);
        let slot = ids[0].slot as usize;
        for (actual, first) in [
            (host.runtime.server.entities.columns.position[slot], 1),
            (host.runtime.server.entities.columns.velocity[slot], 0),
            (host.runtime.server.entities.columns.angles[slot], 6),
            (
                host.runtime.server.entities.columns.angular_velocity[slot].unwrap(),
                5,
            ),
        ] {
            assert_eq!(
                actual.0.map(f32::to_bits),
                expected[first..first + 3]
                    .iter()
                    .copied()
                    .map(f32::to_bits)
                    .collect::<Vec<_>>()
                    .as_slice()
            );
        }
        for (callback, first) in [(1, 1), (2, 6)] {
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(callback)),
                ThinkTime::Seconds(0.125),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC(std::array::from_fn(|axis| {
                    expected[first + axis].to_bits()
                })))
            );
        }
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
        assert_eq!(
            host.runtime.server.entities.columns.motion[slot],
            Motion::Noclip
        );
        assert!(host.runtime.server.area.bounds(ids[0]).is_some());
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
            if rules == RuleSetId::QuakeWorld { 0 } else { 3 }
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
            host.execute_commands();
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
            if rules == RuleSetId::QuakeWorld { 0 } else { 4 }
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
    host.execute_commands();
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

#[test]
fn quakec_step_falls_before_think_and_suppressed_rows_still_think_then_check_water() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let think = code.function(&[
            // Copy the current flags into an inert presentation field. The
            // falling row can observe ONGROUND only if motion ran first.
            (LoadF, 34, 101, 70),
            (Address, 34, 102, 61),
            (StorePF, 70, 61, 0),
            // The provider must run water after think and reread this native
            // mutation rather than retaining the value from the fall phase.
            (Address, 34, 100, 61),
            (StorePF, 104, 61, 0),
            (Return, 70, 0, 0),
        ]);
        let inspections = (0..4)
            .map(|index| {
                code.function(&[
                    (LoadF, 83 + index, 103, 70),
                    (LoadF, 83 + index, 100, 71),
                    (LoadF, 83 + index, 102, 72),
                    (Return, 70, 0, 0),
                ])
            })
            .collect::<Vec<_>>();
        let mut fields = FIELDS
            .iter()
            .copied()
            .filter(|(_, _, name)| *name != b"avelocity")
            .collect::<Vec<_>>();
        fields.extend_from_slice(&[
            (2, 40, b"gravity"),
            (2, 41, b"waterlevel"),
            (2, 42, b"watertype"),
            (4, 43, b"groundentity"),
            (6, 44, b"touch"),
        ]);
        let mut vm = code.vm_fields(&fields);
        let stride = vm.stride as u32;
        vm.image.globals[100] = vm.image.field(b"watertype").unwrap() as u32;
        vm.image.globals[101] = vm.image.field(b"flags").unwrap() as u32;
        vm.image.globals[102] = vm.image.field(b"frame").unwrap() as u32;
        vm.image.globals[103] = vm.image.field(b"waterlevel").unwrap() as u32;
        vm.image.globals[104] = (-1.0f32).to_bits();
        for (index, (native, flags)) in [(40, 0.0f32), (41, 512.0), (42, 1.0), (43, 2.0)]
            .into_iter()
            .enumerate()
        {
            let x = index as f32 * 10.0;
            write_vector(&mut vm, native, b"origin", Vec3([x, 0.0, 5.0]));
            write_vector(
                &mut vm,
                native,
                b"velocity",
                if index == 0 {
                    Vec3::default()
                } else {
                    Vec3([0.0, 0.0, -123.0])
                },
            );
            write_vector(&mut vm, native, b"mins", Vec3([-1.0; 3]));
            write_vector(&mut vm, native, b"maxs", Vec3([1.0; 3]));
            write(&mut vm, native, b"solid", 2.0f32.to_bits());
            write(&mut vm, native, b"flags", flags.to_bits());
            write(&mut vm, native, b"gravity", 1.0f32.to_bits());
            write(&mut vm, native, b"waterlevel", 0.0f32.to_bits());
            write(&mut vm, native, b"watertype", 0.0f32.to_bits());
            vm.image.globals[83 + index] = native as u32 * stride;
        }

        let mut runtime = Runtime::load(1, []).unwrap();
        install_motion_world(&mut runtime, true);
        let (mut request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[
                (40, 4.0, 0.05, think),
                (41, 4.0, 0.05, think),
                (42, 4.0, 0.05, think),
                (43, 4.0, 0.05, think),
            ],
        );
        // Native gravity/sweep rules and caller-selected clipping are
        // independent. The arena epsilon makes that selection observable.
        request.context.trace_rules = RuleSetId::Quake3;
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 100);

        let columns = &host.runtime.server.entities.columns;
        let stopped_z = 1.0
            + qa_world::collision::trace_policy(RuleSetId::Quake3)
                .0
                .contact_epsilon as f32;
        assert_eq!(
            columns.position[ids[0].slot as usize],
            Vec3([0.0, 0.0, stopped_z])
        );
        assert_eq!(columns.velocity[ids[0].slot as usize], Vec3::default());
        assert_eq!(columns.flags[ids[0].slot as usize], 512);
        for (index, entity) in ids.iter().enumerate().skip(1) {
            assert_eq!(
                columns.position[entity.slot as usize],
                Vec3([index as f32 * 10.0, 0.0, 5.0])
            );
            assert_eq!(
                columns.velocity[entity.slot as usize],
                Vec3([0.0, 0.0, -123.0])
            );
        }
        assert!(
            ids.iter()
                .all(|entity| columns.angular_velocity[entity.slot as usize].is_none())
        );
        for (index, inspect) in inspections.into_iter().enumerate() {
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect)),
                ThinkTime::Seconds(0.1),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    (-1.0f32).to_bits(),
                    (-1.0f32).to_bits(),
                    [512.0f32, 512.0, 1.0, 2.0][index].to_bits(),
                ]))
            );
        }
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!((counts.rejected, counts.unsupported_physics), (0, 0));
    }
}

#[test]
fn quakec_step_contacts_ground_before_touch_then_relinks_sounds_thinks_and_reuses() {
    use qa_core::configstrings::ConfigLayout;
    use std::num::NonZeroU32;

    let mut code = Code::new();
    let frame = code.function(&[(Return, 0, 0, 0)]);
    let removed_touch = code.function(&[
        (MulF, 89, 94, 126),
        (AddF, 126, 95, 89),
        (LoadF, 34, 71, 90),
        (StoreEnt, 34, 4, 0),
        (Call1, 59, 0, 0),
        (Return, 0, 0, 0),
    ]);
    let healthy_touch = code.function(&[
        (MulF, 89, 94, 126),
        (AddF, 126, 96, 89),
        (LoadF, 34, 71, 91),
        (LoadF, 34, 78, 92),
        (Return, 0, 0, 0),
    ]);
    let trigger_touch =
        code.function(&[(MulF, 89, 94, 126), (AddF, 126, 97, 89), (Return, 0, 0, 0)]);
    let removed_think = code.function(&[(StoreF, 48, 93, 0), (Return, 0, 0, 0)]);
    let healthy_think = code.function(&[
        (MulF, 89, 94, 126),
        (AddF, 126, 98, 89),
        // Water must consume the value written by the just-completed think.
        (Address, 34, 73, 126),
        (StorePF, 83, 126, 0),
        (Return, 0, 0, 0),
    ]);
    let inspect_order = code.function(&[(Return, 89, 0, 0)]);
    let inspect_state = code.function(&[
        (LoadF, 64, 72, 70),
        (LoadF, 64, 73, 71),
        (LoadF, 64, 78, 72),
        (Return, 70, 0, 0),
    ]);
    let inspect_removed_think = code.function(&[
        (StoreF, 93, 70, 0),
        (StoreF, 49, 71, 0),
        (StoreF, 49, 72, 0),
        (Return, 70, 0, 0),
    ]);
    let spawn = code.builtin(14);
    let remove = code.builtin(15);
    let setorigin = code.builtin(2);
    let precache = code.builtin(19);
    let reuse = code.function(&[(Call0, 58, 0, 0), (Return, 1, 0, 0)]);
    let initialize = code.function(&[
        (StoreS, 62, 4, 0),
        (Call1, 61, 0, 0),
        // Two actual QC allocations make removal and later native-row reuse
        // exercise owned lifetimes rather than borrowed request seed rows.
        (Call0, 58, 0, 0),
        (StoreEnt, 1, 63, 0),
        (Address, 63, 77, 126),
        (StorePF, 79, 126, 0),
        (Address, 63, 66, 126),
        (StorePV, 99, 126, 0),
        (Address, 63, 67, 126),
        (StorePV, 123, 126, 0),
        (Address, 63, 68, 126),
        (StorePV, 108, 126, 0),
        (Address, 63, 69, 126),
        (StorePV, 111, 126, 0),
        (Address, 63, 70, 126),
        (StorePF, 80, 126, 0),
        (Address, 63, 74, 126),
        (StorePF, 84, 126, 0),
        (Address, 63, 75, 126),
        (StorePF, 82, 126, 0),
        (Address, 63, 76, 126),
        (StorePF, 87, 126, 0),
        (StoreEnt, 63, 4, 0),
        (StoreV, 99, 7, 0),
        (Call2, 60, 0, 0),
        (Call0, 58, 0, 0),
        (StoreEnt, 1, 64, 0),
        (Address, 64, 77, 126),
        (StorePF, 79, 126, 0),
        (Address, 64, 66, 126),
        (StorePV, 102, 126, 0),
        (Address, 64, 67, 126),
        (StorePV, 105, 126, 0),
        (Address, 64, 68, 126),
        (StorePV, 108, 126, 0),
        (Address, 64, 69, 126),
        (StorePV, 111, 126, 0),
        (Address, 64, 70, 126),
        (StorePF, 80, 126, 0),
        (Address, 64, 74, 126),
        (StorePF, 85, 126, 0),
        (Address, 64, 75, 126),
        (StorePF, 82, 126, 0),
        (Address, 64, 76, 126),
        (StorePF, 88, 126, 0),
        (StoreEnt, 64, 4, 0),
        (StoreV, 102, 7, 0),
        (Call2, 60, 0, 0),
        // The landing relink overlaps this trigger. Its callback must finish
        // before the landing sound and the subsequent think phase.
        (Call0, 58, 0, 0),
        (StoreEnt, 1, 65, 0),
        (Address, 65, 66, 126),
        (StorePV, 114, 126, 0),
        (Address, 65, 68, 126),
        (StorePV, 117, 126, 0),
        (Address, 65, 69, 126),
        (StorePV, 120, 126, 0),
        (Address, 65, 70, 126),
        (StorePF, 81, 126, 0),
        (Address, 65, 74, 126),
        (StorePF, 86, 126, 0),
        (StoreEnt, 65, 4, 0),
        (StoreV, 114, 7, 0),
        (Call2, 60, 0, 0),
        (Return, 0, 0, 0),
    ]);

    let mut vm = code.motion_vm();
    let stride = vm.stride as u32;
    vm.image.globals[58] = spawn;
    vm.image.globals[59] = remove;
    vm.image.globals[60] = setorigin;
    vm.image.globals[61] = precache;
    vm.image.globals[62] = vm.insert_string(b"demon/dland2.wav").unwrap() as u32;
    for (global, field) in [
        (66, b"origin".as_slice()),
        (67, b"velocity"),
        (68, b"mins"),
        (69, b"maxs"),
        (70, b"solid"),
        (71, b"flags"),
        (72, b"waterlevel"),
        (73, b"watertype"),
        (74, b"touch"),
        (75, b"nextthink"),
        (76, b"think"),
        (77, b"movetype"),
        (78, b"groundentity"),
    ] {
        vm.image.globals[global] = vm.image.field(field).unwrap() as u32;
    }
    vm.image.globals[79] = 4.0f32.to_bits();
    vm.image.globals[80] = 2.0f32.to_bits();
    vm.image.globals[81] = 1.0f32.to_bits();
    vm.image.globals[82] = 0.05f32.to_bits();
    vm.image.globals[83] = (-1.0f32).to_bits();
    vm.image.globals[84] = removed_touch;
    vm.image.globals[85] = healthy_touch;
    vm.image.globals[86] = trigger_touch;
    vm.image.globals[87] = removed_think;
    vm.image.globals[88] = healthy_think;
    vm.image.globals[94] = 10.0f32.to_bits();
    vm.image.globals[95] = 4.0f32.to_bits();
    vm.image.globals[96] = 1.0f32.to_bits();
    vm.image.globals[97] = 2.0f32.to_bits();
    vm.image.globals[98] = 3.0f32.to_bits();
    vm.image.globals[99..102].copy_from_slice(&Vec3([0.0, 0.0, 5.0]).0.map(f32::to_bits));
    vm.image.globals[102..105].copy_from_slice(&Vec3([20.0, 0.0, 15.0]).0.map(f32::to_bits));
    vm.image.globals[105..108].copy_from_slice(&Vec3([0.0, 0.0, -100.0]).0.map(f32::to_bits));
    vm.image.globals[108..111].fill((-1.0f32).to_bits());
    vm.image.globals[111..114].fill(1.0f32.to_bits());
    vm.image.globals[114..117].copy_from_slice(&Vec3([20.0, 0.0, 1.0]).0.map(f32::to_bits));
    vm.image.globals[117..120].fill((-2.0f32).to_bits());
    vm.image.globals[120..123].fill(2.0f32.to_bits());

    let mut runtime = Runtime::load(1, []).unwrap();
    install_motion_world(&mut runtime, true);
    let (mut request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        (0..code.functions.len() as u32).collect(),
        frame,
        100,
        &[],
    );
    request.context.trace_rules = RuleSetId::Quake3;
    request.configstrings = ConfigLayout::for_rules(RuleSetId::Quake).count;
    request.initialize = vec![Export::clocked(CallbackId(initialize))];
    let mut host = load(runtime, vec![request]);
    host.initialize_modules(Phase::Server);
    let removed = native_entity(&host, ModuleId(1), 2);
    let healthy = native_entity(&host, ModuleId(1), 3);

    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = qa_audio::Bank::load(
        rate,
        vec![(
            "sound/demon/dland2.wav".to_owned(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![1000, -1000],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let landing = bank.find("sound/demon/dland2.wav").unwrap();
    host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    host.bind_sounds().unwrap();
    let observer = host
        .runtime
        .server
        .events
        .bind(qa_core::events::OutputTarget::Module(ModuleId(9)))
        .unwrap();

    tick(&mut host, 0);
    tick(&mut host, 100);
    assert!(host.runtime.server.entities.resolve(removed).is_none());
    assert!(host.runtime.server.entities.resolve(healthy).is_some());
    let healthy_slot = healthy.slot as usize;
    let stopped_z = 1.0
        + qa_world::collision::trace_policy(RuleSetId::Quake3)
            .0
            .contact_epsilon as f32;
    assert_eq!(
        host.runtime.server.entities.columns.position[healthy_slot],
        Vec3([20.0, 0.0, stopped_z])
    );
    assert_eq!(
        host.runtime.server.entities.columns.flags[healthy_slot],
        512
    );

    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(inspect_order)),
        ThinkTime::Seconds(0.1),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            4123.0f32.to_bits(),
            512.0f32.to_bits(),
            512.0f32.to_bits(),
        ]))
    );
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(inspect_state)),
        ThinkTime::Seconds(0.1),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            // Native SV_CheckWaterTransition stores non-water contents in
            // waterlevel. The foreign linked BODY now projects to SOLID.
            (-2.0f32).to_bits(),
            (-1.0f32).to_bits(),
            0,
        ]))
    );
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(inspect_removed_think)),
        ThinkTime::Seconds(0.1),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([0, 0, 0]))
    );

    let events = &host.runtime.server.events;
    let mut batch = events.batch(observer).unwrap();
    let sounds = std::iter::from_fn(|| events.next(&mut batch))
        .filter_map(|record| match record.event {
            FrameEvent::Sound(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sounds.len(), 1);
    assert_eq!(sounds[0].sound, landing);
    assert_eq!(sounds[0].action, SoundAction::Play);
    assert_eq!(sounds[0].source_ordinal, Some(1));
    assert_eq!(sounds[0].entity, Some(SoundEmitter::Entity(healthy)));

    tick(&mut host, 700);
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(reuse)),
        ThinkTime::Seconds(0.7),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([2 * stride, 0, 0]))
    );
    let replacement = native_entity(&host, ModuleId(1), 2);
    assert_ne!(replacement, removed);
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!((counts.rejected, counts.unsupported_physics), (0, 0));
}

#[test]
fn quakec_toss_bounce_fly_and_missile_run_through_the_shared_motion_phase() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let toss_state = code.function(&[(LoadF, 80, 81, 70), (LoadF, 80, 82, 71), (Return, 70, 0, 0)]);
    let fly_water = code.function(&[(LoadF, 83, 84, 70), (LoadF, 83, 85, 71), (Return, 70, 0, 0)]);
    let mut vm = code.motion_vm();
    let stride = vm.stride as u32;
    for (native, origin, velocity, gravity) in [
        (40, Vec3([0.0, 0.0, 5.0]), Vec3::default(), 0.0f32),
        (
            41,
            Vec3([20.0, 0.0, 10.0]),
            Vec3([0.0, 0.0, -200.0]),
            1.0f32,
        ),
        (42, Vec3([40.0, 0.0, 20.0]), Vec3([20.0, 0.0, 0.0]), -7.0f32),
        (43, Vec3([60.0, 0.0, 20.0]), Vec3([30.0, 0.0, 0.0]), -7.0f32),
    ] {
        write_vector(&mut vm, native, b"origin", origin);
        write_vector(&mut vm, native, b"velocity", velocity);
        write_vector(&mut vm, native, b"mins", Vec3([-1.0; 3]));
        write_vector(&mut vm, native, b"maxs", Vec3([1.0; 3]));
        write(&mut vm, native, b"solid", 2.0f32.to_bits());
        write(&mut vm, native, b"flags", 0.0f32.to_bits());
        write(&mut vm, native, b"gravity", gravity.to_bits());
        write(&mut vm, native, b"waterlevel", 0.0f32.to_bits());
        write(&mut vm, native, b"watertype", 0.0f32.to_bits());
    }
    // A clear trace returns before water transition. FLY also proves that a
    // negative per-edict gravity is ignored by no-gravity modes.
    write(&mut vm, 42, b"waterlevel", 7.0f32.to_bits());
    write(&mut vm, 42, b"watertype", (-3.0f32).to_bits());
    vm.image.globals[80] = 40 * stride;
    vm.image.globals[81] = vm.image.field(b"flags").unwrap() as u32;
    vm.image.globals[82] = vm.image.field(b"groundentity").unwrap() as u32;
    vm.image.globals[83] = 42 * stride;
    vm.image.globals[84] = vm.image.field(b"waterlevel").unwrap() as u32;
    vm.image.globals[85] = vm.image.field(b"watertype").unwrap() as u32;

    let mut runtime = Runtime::load(1, []).unwrap();
    install_motion_world(&mut runtime, true);
    let (mut request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        (0..code.functions.len() as u32).collect(),
        frame,
        100,
        &[
            (40, 6.0, 0.0, 0),
            (41, 10.0, 0.0, 0),
            (42, 5.0, 0.0, 0),
            (43, 9.0, 0.0, 0),
        ],
    );
    request.context.trace_rules = RuleSetId::Quake;
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 100);

    let columns = &host.runtime.server.entities.columns;
    let toss = ids[0].slot as usize;
    let stopped_z = 1.0
        + qa_world::collision::trace_policy(RuleSetId::Quake)
            .0
            .contact_epsilon as f32;
    assert_eq!(
        columns.position[toss].0[2].to_bits(),
        stopped_z.to_bits(),
        "toss body after native frame: {:?}",
        columns.body(toss),
    );
    assert_eq!(columns.velocity[toss], Vec3::default());
    assert_eq!(columns.angular_velocity[toss], Some(Vec3::default()));
    assert!(host.runtime.server.area.bounds(ids[0]).is_some());

    let bounce = ids[1].slot as usize;
    assert_eq!(columns.position[bounce].0[2].to_bits(), stopped_z.to_bits());
    assert!(columns.velocity[bounce].0[2] > 60.0);
    assert_eq!(columns.motion[bounce], Motion::Bounce);

    assert_eq!(
        columns.position[ids[2].slot as usize],
        Vec3([42.0, 0.0, 20.0])
    );
    assert_eq!(
        columns.velocity[ids[2].slot as usize],
        Vec3([20.0, 0.0, 0.0])
    );
    assert_eq!(columns.motion[ids[2].slot as usize], Motion::Fly);
    assert_eq!(
        columns.position[ids[3].slot as usize],
        Vec3([63.0, 0.0, 20.0])
    );
    assert_eq!(
        columns.velocity[ids[3].slot as usize],
        Vec3([30.0, 0.0, 0.0])
    );
    assert_eq!(columns.motion[ids[3].slot as usize], Motion::FlyMissile);

    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(toss_state)),
        ThinkTime::Seconds(0.1),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([512.0f32.to_bits(), 0, 0]))
    );
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(fly_water)),
        ThinkTime::Seconds(0.1),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            7.0f32.to_bits(),
            (-3.0f32).to_bits(),
            0,
        ]))
    );
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!((counts.rejected, counts.unsupported_physics), (0, 0));
}

#[test]
fn quakec_flight_relinks_triggers_then_rereads_both_impact_callbacks() {
    for reuse_target in [true, false] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let think = code.function(&[(MulF, 70, 80, 100), (AddF, 100, 81, 70), (Return, 70, 0, 0)]);
        let trigger = code.function(&[
            (MulF, 70, 80, 100),
            (AddF, 100, 82, 70),
            // Trigger callbacks see the mover as other. Install its impact
            // function only now, proving relink triggers precede impact.
            (Address, 36, 86, 102),
            (StorePF, 87, 102, 0),
            (Return, 70, 0, 0),
        ]);
        let mut mover_body = vec![(MulF, 70, 80, 100), (AddF, 100, 83, 70)];
        if reuse_target {
            mover_body.extend_from_slice(&[
                // Replace the traced obstacle lifetime at the same native
                // ordinal before its half of the impact callback pair.
                (StoreEnt, 88, 4, 0),
                (Call1, 89, 0, 0),
                (Call0, 93, 0, 0),
                (StoreEnt, 1, 88, 0),
            ]);
        }
        mover_body.extend_from_slice(&[
            // The second callback must reread the current ordinal's function
            // and solid, whether it is the old or replacement lifetime.
            (Address, 88, 86, 102),
            (StorePF, 90, 102, 0),
            (Address, 88, 91, 102),
            (StorePF, 92, 102, 0),
            // qsrc prepares time once for the impact pair. A mutation by the
            // first callback remains visible to the second callback.
            (StoreF, 108, 35, 0),
            // Removing self must not erase the weak other value delivered to
            // the surviving obstacle callback.
            (StoreEnt, 34, 4, 0),
            (Call1, 89, 0, 0),
            (Return, 70, 0, 0),
        ]);
        let mover_touch = code.function(&mover_body);
        let stale_target_touch =
            code.function(&[(MulF, 70, 80, 100), (AddF, 100, 84, 70), (Return, 70, 0, 0)]);
        let current_target_touch = code.function(&[
            (MulF, 70, 80, 100),
            (AddF, 100, 85, 70),
            (StoreEnt, 36, 71, 0),
            (StoreF, 35, 72, 0),
            (Return, 70, 0, 0),
        ]);
        let inspect = code.function(&[(Return, 70, 0, 0)]);
        let spawn_export = code.function(&[(Call0, 93, 0, 0), (Return, 1, 0, 0)]);
        let remove = code.builtin(15);
        let spawn = code.builtin(14);
        let setorigin = code.builtin(2);
        let link_contacts = code.function(&[
            (StoreEnt, 109, 4, 0),
            (StoreV, 77, 7, 0),
            (Call2, 107, 0, 0),
            (StoreEnt, 103, 4, 0),
            (StoreV, 104, 7, 0),
            (Call2, 107, 0, 0),
            (StoreEnt, 88, 4, 0),
            (StoreV, 94, 7, 0),
            (Call2, 107, 0, 0),
            (Return, 0, 0, 0),
        ]);
        let initialize_entities = code.function(&[
            // Removal is valid only for module-owned QC allocations. Build
            // the whole contact set through Spawn rather than seeding borrowed
            // common lifetimes through the test request.
            (Call0, 93, 0, 0),
            (StoreEnt, 1, 109, 0),
            (Address, 109, 114, 102),
            (StorePF, 115, 102, 0),
            (Address, 109, 111, 102),
            (StorePV, 116, 102, 0),
            (Address, 109, 112, 102),
            (StorePV, 119, 102, 0),
            (Address, 109, 113, 102),
            (StorePV, 122, 102, 0),
            (Address, 109, 91, 102),
            (StorePF, 125, 102, 0),
            (Address, 109, 73, 102),
            (StorePF, 75, 102, 0),
            (Address, 109, 74, 102),
            (StorePF, 76, 102, 0),
            (Call0, 93, 0, 0),
            (StoreEnt, 1, 103, 0),
            (Address, 103, 110, 102),
            (StorePV, 104, 102, 0),
            (Address, 103, 112, 102),
            (StorePV, 119, 102, 0),
            (Address, 103, 113, 102),
            (StorePV, 122, 102, 0),
            (Address, 103, 91, 102),
            (StorePF, 126, 102, 0),
            (Address, 103, 86, 102),
            (StorePF, 127, 102, 0),
            (Call0, 93, 0, 0),
            (StoreEnt, 1, 88, 0),
            (Address, 88, 110, 102),
            (StorePV, 94, 102, 0),
            (Address, 88, 112, 102),
            (StorePV, 119, 102, 0),
            (Address, 88, 113, 102),
            (StorePV, 122, 102, 0),
            (Address, 88, 91, 102),
            (StorePF, 125, 102, 0),
            (Address, 88, 86, 102),
            (StorePF, 99, 102, 0),
            (Return, 0, 0, 0),
        ]);
        let mut vm = code.motion_vm();
        let stride = vm.stride as u32;
        let mover = 2;
        let trigger_entity = 3;
        let target = 4;
        vm.image.globals[73] = vm.image.field(b"nextthink").unwrap() as u32;
        vm.image.globals[74] = vm.image.field(b"think").unwrap() as u32;
        vm.image.globals[75] = 0.05f32.to_bits();
        vm.image.globals[76] = think;
        vm.image.globals[80] = 10.0f32.to_bits();
        vm.image.globals[81] = 1.0f32.to_bits();
        vm.image.globals[82] = 2.0f32.to_bits();
        vm.image.globals[83] = 3.0f32.to_bits();
        vm.image.globals[84] = 9.0f32.to_bits();
        vm.image.globals[85] = 4.0f32.to_bits();
        vm.image.globals[86] = vm.image.field(b"touch").unwrap() as u32;
        vm.image.globals[87] = mover_touch;
        vm.image.globals[88] = target as u32 * stride;
        vm.image.globals[89] = remove;
        vm.image.globals[90] = current_target_touch;
        vm.image.globals[91] = vm.image.field(b"solid").unwrap() as u32;
        vm.image.globals[92] = if reuse_target {
            2.0f32.to_bits()
        } else {
            0.0f32.to_bits()
        };
        vm.image.globals[93] = spawn;
        vm.image.globals[94..97].copy_from_slice(&[
            8.0f32.to_bits(),
            0.0f32.to_bits(),
            0.0f32.to_bits(),
        ]);
        vm.image.globals[103] = trigger_entity as u32 * stride;
        vm.image.globals[104..107].copy_from_slice(&[
            5.0f32.to_bits(),
            0.0f32.to_bits(),
            0.0f32.to_bits(),
        ]);
        vm.image.globals[107] = setorigin;
        vm.image.globals[108] = 123.0f32.to_bits();
        vm.image.globals[110] = vm.image.field(b"origin").unwrap() as u32;
        vm.image.globals[111] = vm.image.field(b"velocity").unwrap() as u32;
        vm.image.globals[112] = vm.image.field(b"mins").unwrap() as u32;
        vm.image.globals[113] = vm.image.field(b"maxs").unwrap() as u32;
        vm.image.globals[114] = vm.image.field(b"movetype").unwrap() as u32;
        vm.image.globals[115] = 5.0f32.to_bits();
        vm.image.globals[116..119].copy_from_slice(&[
            100.0f32.to_bits(),
            0.0f32.to_bits(),
            0.0f32.to_bits(),
        ]);
        vm.image.globals[119..122].fill((-1.0f32).to_bits());
        vm.image.globals[122..125].fill(1.0f32.to_bits());
        vm.image.globals[125] = 2.0f32.to_bits();
        vm.image.globals[126] = 1.0f32.to_bits();
        vm.image.globals[127] = trigger;
        vm.image.globals[99] = stale_target_touch;

        let mut runtime = Runtime::load(1, []).unwrap();
        install_motion_world(&mut runtime, false);
        let (mut request, _) = request(
            &mut runtime,
            ModuleId(1),
            RuleSetId::Quake,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[],
        );
        request.context.trace_rules = RuleSetId::Quake;
        let mut host = load(runtime, vec![request]);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(initialize_entities)),
            ThinkTime::Seconds(0.0),
        )
        .unwrap();
        let ids = [
            native_entity(&host, ModuleId(1), mover),
            native_entity(&host, ModuleId(1), trigger_entity),
            native_entity(&host, ModuleId(1), target),
        ];
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(link_contacts)),
            ThinkTime::Seconds(0.0),
        )
        .unwrap();
        let old_mover = ids[0];
        tick(&mut host, 0);
        tick(&mut host, 100);
        let counts = host.module_counts(ModuleId(1)).unwrap();
        let diagnostics = (counts.rejected, counts.traps, counts.last_result);
        assert!(
            host.runtime.server.entities.resolve(old_mover).is_none(),
            "mover body after impact frame: {:?}; counts: {:?}",
            host.runtime
                .server
                .entities
                .columns
                .body(old_mover.slot as usize),
            diagnostics,
        );
        let current_target = native_entity(&host, ModuleId(1), target);
        if reuse_target {
            assert!(host.runtime.server.entities.resolve(ids[2]).is_none());
            assert_ne!(current_target, ids[2]);
        } else {
            assert_eq!(current_target, ids[2]);
        }

        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect)),
            ThinkTime::Seconds(0.1),
        )
        .unwrap();
        let expected_log: f32 = if reuse_target { 1234.0 } else { 123.0 };
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                expected_log.to_bits(),
                if reuse_target {
                    mover as u32 * stride
                } else {
                    0
                },
                if reuse_target { 123.0f32.to_bits() } else { 0 },
            ]))
        );
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!((counts.rejected, counts.unsupported_physics), (0, 0));

        // After the original half-second reuse grace, spawn may reuse the
        // removed native row but receives a distinct canonical lifetime.
        tick(&mut host, 700);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(spawn_export)),
            ThinkTime::Seconds(0.7),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([mover as u32 * stride, 0, 0]))
        );
        let replacement = native_entity(&host, ModuleId(1), mover);
        assert_ne!(replacement, old_mover);
    }
}

#[test]
fn quakec_attached_projectile_callback_sees_transport_without_double_motion() {
    use qa_core::primitives::{BodyAttachment, BodyFollow};

    let mut code = Code::new();
    let frame = code.function(&[(Return, 0, 0, 0)]);
    let think = code.function(&[(LoadV, 34, 80, 90), (Return, 90, 0, 0)]);
    let inspect_seen = code.function(&[(Return, 90, 0, 0)]);
    let inspect_native = code.function(&[(LoadV, 81, 80, 70), (Return, 70, 0, 0)]);
    let mut vm = code.motion_vm();
    let child = 2;
    let stale_native_position = Vec3([1.0, 2.0, 3.0]);
    let velocity = Vec3([100.0, 20.0, 0.0]);
    write_vector(&mut vm, child, b"origin", stale_native_position);
    write_vector(&mut vm, child, b"velocity", velocity);
    write_vector(&mut vm, child, b"mins", Vec3([-1.0; 3]));
    write_vector(&mut vm, child, b"maxs", Vec3([1.0; 3]));
    write(&mut vm, child, b"solid", 2.0f32.to_bits());
    write(&mut vm, child, b"flags", 0.0f32.to_bits());
    write(&mut vm, child, b"gravity", 1.0f32.to_bits());
    write(&mut vm, child, b"waterlevel", 0.0f32.to_bits());
    write(&mut vm, child, b"watertype", 0.0f32.to_bits());
    vm.image.globals[80] = vm.image.field(b"origin").unwrap() as u32;
    vm.image.globals[81] = child as u32 * vm.stride as u32;

    let mut runtime = Runtime::load(1, []).unwrap();
    install_motion_world(&mut runtime, false);
    let (mut request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        (0..code.functions.len() as u32).collect(),
        frame,
        100,
        &[(child, 9.0, 0.05, think)],
    );
    request.context.trace_rules = RuleSetId::Quake;
    let mut host = load(runtime, vec![request]);
    let anchor = host
        .runtime
        .server
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(9),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    host.runtime.server.entities.columns.position[anchor.slot as usize] = Vec3([20.0, 30.0, 40.0]);
    host.runtime
        .server
        .entities
        .attach(
            ids[0],
            BodyAttachment {
                anchor,
                follow: BodyFollow::Translation,
                offset: Vec3([2.0, 3.0, 4.0]),
            },
        )
        .unwrap();
    let transported = host.runtime.server.commit_attachments();
    assert_eq!((transported.visited, transported.moved), (1, 1));
    let position = Vec3([22.0, 33.0, 44.0]);
    assert_eq!(
        host.runtime.server.entities.columns.position[ids[0].slot as usize],
        position
    );

    tick(&mut host, 0);
    tick(&mut host, 100);
    assert_eq!(
        host.runtime.server.entities.columns.position[ids[0].slot as usize],
        position
    );
    assert_eq!(
        host.runtime.server.entities.columns.velocity[ids[0].slot as usize],
        velocity
    );
    for inspect in [inspect_seen, inspect_native] {
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect)),
            ThinkTime::Seconds(0.1),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC(position.0.map(f32::to_bits)))
        );
    }
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!((counts.rejected, counts.unsupported_physics), (0, 0));
}

#[test]
fn quakec_water_entry_writes_native_state_and_publishes_one_shared_sound() {
    use qa_core::configstrings::ConfigLayout;
    use std::num::NonZeroU32;

    let mut code = Code::new();
    let precache = code.builtin(19);
    let initialize = code.function(&[(StoreS, 80, 4, 0), (Call1, 81, 0, 0), (Return, 0, 0, 0)]);
    let frame = code.function(&[(Return, 0, 0, 0)]);
    let inspect = code.function(&[(LoadF, 82, 83, 70), (LoadF, 82, 84, 71), (Return, 70, 0, 0)]);
    let setorigin = code.builtin(2);
    let link_target = code.function(&[
        (StoreEnt, 85, 4, 0),
        (StoreV, 86, 7, 0),
        (Call2, 89, 0, 0),
        (Return, 0, 0, 0),
    ]);
    let mut vm = code.motion_vm();
    let stride = vm.stride as u32;
    let mover = 2;
    let target = 3;
    vm.image.globals[80] = vm.insert_string(b"misc/h2ohit1.wav").unwrap() as u32;
    vm.image.globals[81] = precache;
    vm.image.globals[82] = mover as u32 * stride;
    vm.image.globals[83] = vm.image.field(b"waterlevel").unwrap() as u32;
    vm.image.globals[84] = vm.image.field(b"watertype").unwrap() as u32;
    vm.image.globals[85] = target as u32 * stride;
    vm.image.globals[86..89].copy_from_slice(&[
        8.0f32.to_bits(),
        0.0f32.to_bits(),
        0.0f32.to_bits(),
    ]);
    vm.image.globals[89] = setorigin;
    for (native, origin) in [(mover, Vec3::default()), (target, Vec3([8.0, 0.0, 0.0]))] {
        write_vector(&mut vm, native, b"origin", origin);
        write_vector(&mut vm, native, b"mins", Vec3([-1.0; 3]));
        write_vector(&mut vm, native, b"maxs", Vec3([1.0; 3]));
        write(&mut vm, native, b"solid", 2.0f32.to_bits());
        write(&mut vm, native, b"flags", 0.0f32.to_bits());
        write(&mut vm, native, b"gravity", 1.0f32.to_bits());
        write(&mut vm, native, b"waterlevel", 0.0f32.to_bits());
        write(&mut vm, native, b"watertype", (-1.0f32).to_bits());
        write(&mut vm, native, b"touch", 0);
    }
    write_vector(&mut vm, mover, b"velocity", Vec3([100.0, 0.0, 0.0]));

    let mut runtime = Runtime::load(1, []).unwrap();
    install_water_world(&mut runtime);
    let (mut request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        (0..code.functions.len() as u32).collect(),
        frame,
        100,
        &[(mover, 5.0, 0.0, 0), (target, 0.0, 0.0, 0)],
    );
    request.context.trace_rules = RuleSetId::Quake;
    request.configstrings = ConfigLayout::for_rules(RuleSetId::Quake).count;
    request.initialize = vec![Export::clocked(CallbackId(initialize))];
    let mut host = load(runtime, vec![request]);
    host.initialize_modules(Phase::Server);

    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = qa_audio::Bank::load(
        rate,
        vec![(
            "sound/misc/h2ohit1.wav".to_owned(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![1000, -1000],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let splash = bank.find("sound/misc/h2ohit1.wav").unwrap();
    host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    host.bind_sounds().unwrap();
    let observer = host
        .runtime
        .server
        .events
        .bind(qa_core::events::OutputTarget::Module(ModuleId(9)))
        .unwrap();
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(link_target)),
        ThinkTime::Seconds(0.0),
    )
    .unwrap();
    tick(&mut host, 0);
    tick(&mut host, 100);
    tick(&mut host, 200);

    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(inspect)),
        ThinkTime::Seconds(0.2),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            1.0f32.to_bits(),
            (-3.0f32).to_bits(),
            0,
        ]))
    );

    let events = &host.runtime.server.events;
    let mut batch = events.batch(observer).unwrap();
    let sounds = std::iter::from_fn(|| events.next(&mut batch))
        .filter_map(|record| match record.event {
            FrameEvent::Sound(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sounds.len(), 1);
    assert_eq!(sounds[0].sound, splash);
    assert_eq!(sounds[0].action, SoundAction::Play);
    assert_eq!(sounds[0].entity, Some(SoundEmitter::Entity(ids[0])));
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!((counts.rejected, counts.unsupported_physics), (0, 0));
}

fn qc_client_vm() -> Vm {
    let mut fields = FIELDS.to_vec();
    fields.extend_from_slice(&[
        (2, 40, b"health".as_slice()),
        (2, 41, b"armorvalue"),
        (2, 42, b"team"),
        (1, 43, b"netname"),
        (2, 44, b"gravity"),
        (2, 45, b"maxspeed"),
        (2, 46, b"fixangle"),
        (3, 47, b"v_angle"),
        (2, 50, b"button0"),
        (2, 51, b"button2"),
        (2, 52, b"impulse"),
        (2, 53, b"waterlevel"),
        (2, 54, b"watertype"),
        (2, 55, b"teleport_time"),
        (4, 56, b"groundentity"),
        (6, 57, b"touch"),
        (3, 58, b"punchangle"),
        (3, 61, b"view_ofs"),
        (3, 64, b"oldorigin"),
        (3, 67, b"movedir"),
    ]);
    let mut globals = GLOBALS.to_vec();
    let names = [
        b"SetNewParms".as_slice(),
        b"ClientConnect",
        b"PutClientInServer",
        b"PlayerPreThink",
        b"PlayerPostThink",
        b"ClientDisconnect",
    ];
    for (index, name) in names.iter().enumerate() {
        globals.push((6, 140 + index as u16, *name));
    }
    let parms = [
        b"parm1".as_slice(),
        b"parm2",
        b"parm3",
        b"parm4",
        b"parm5",
        b"parm6",
        b"parm7",
        b"parm8",
        b"parm9",
        b"parm10",
        b"parm11",
        b"parm12",
        b"parm13",
        b"parm14",
        b"parm15",
        b"parm16",
    ];
    for (index, name) in parms.iter().enumerate() {
        globals.push((2, 160 + index as u16, *name));
    }
    let mut code = Code::new();
    let new = code.function(&[(StoreF, 80, 160, 0), (Return, 80, 0, 0)]);
    let connect = code.function(&[
        (Address, 34, 81, 90),
        (StorePF, 82, 90, 0),
        (Return, 0, 0, 0),
    ]);
    let begin = code.function(&[
        (Address, 34, 81, 90),
        (StorePF, 83, 90, 0),
        (Address, 34, 84, 90),
        (StorePV, 100, 90, 0),
        (Address, 34, 85, 90),
        (StorePV, 103, 90, 0),
        (Address, 34, 86, 90),
        (StorePF, 87, 90, 0),
        (Return, 0, 0, 0),
    ]);
    let pre = code.function(&[
        (LoadF, 34, 81, 91),
        (AddF, 91, 88, 92),
        (Address, 34, 81, 90),
        (StorePF, 92, 90, 0),
        (Return, 0, 0, 0),
    ]);
    let post = code.function(&[
        (LoadF, 34, 89, 91),
        (AddF, 91, 88, 92),
        (Address, 34, 89, 90),
        (StorePF, 92, 90, 0),
        (Return, 0, 0, 0),
    ]);
    let disconnect = code.function(&[(Return, 0, 0, 0)]);
    let frame = code.function(&[(Return, 35, 0, 0)]);
    let bytes = quakec_program::program_with_definitions(
        &code.operations,
        &code.functions,
        &globals,
        &fields,
        70,
        192,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 256,
            header_bytes: 16,
            extra_string_bytes: 8192,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    for (index, value) in [new, connect, begin, pre, post, disconnect]
        .into_iter()
        .enumerate()
    {
        vm.image.globals[140 + index] = value;
    }
    for (at, value) in [
        (80, 999.0f32.to_bits()),
        (81, 40),
        (82, 100.0f32.to_bits()),
        (83, 0.5f32.to_bits()),
        (84, 9),
        (85, 12),
        (86, 24),
        (87, 2.0f32.to_bits()),
        (88, 1.0f32.to_bits()),
        (89, 41),
    ] {
        vm.image.globals[at] = value;
    }
    for (at, value) in [
        (100, -16.0f32),
        (101, -16.0),
        (102, -24.0),
        (103, 16.0),
        (104, 16.0),
        (105, 32.0),
    ] {
        vm.image.globals[at] = value.to_bits();
    }
    assert_eq!(frame, 7);
    vm
}

fn qc_client_host(
    rules: RuleSetId,
    movement: RuleSetId,
    vm: Vm,
    rows: &[(i32, f32, f32, u32)],
) -> (Box<FrameHost>, ClientId, GeometryId, EntityId, EntityId) {
    use qa_app::{WorldCollision, client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_network::commands::packet::Protocol;
    use qa_world::collision::{
        brushes::{BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
    };
    let mut runtime = Runtime::load(1, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    brushes: IndexSpan { first: 0, count: 0 },
                    patches: Default::default(),
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0)],
                leaf_patches: Vec::new(),
            },
            vec![Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    let id = runtime
        .connect_local(
            SeatId::ALL[0],
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement,
                trace: RuleSetId::Quake3,
            },
            Protocol::native(rules),
            0,
        )
        .unwrap();
    let canonical = runtime.server.clients[id.0 as usize].entity;
    let (mut req, _) = request(
        &mut runtime,
        ModuleId(1),
        rules,
        vm,
        (0..8).collect(),
        7,
        20,
        rows,
    );
    req.context.clock = ThinkTime::Seconds(7.0);
    if rules == RuleSetId::Quake {
        req.rate = TickRate::FrameDriven;
    }
    let anchor = req.anchor;
    let host = load_clients(runtime, vec![req], 1);
    (Box::new(host), id, geometry, canonical, anchor)
}

fn qc_bot_namespace_host(native: NativeEntity) -> (Box<FrameHost>, ClientId, ClientId) {
    let mut runtime = Runtime::load(2, []).unwrap();
    let requests = [ModuleId(1), ModuleId(2)]
        .into_iter()
        .map(|module| {
            request(
                &mut runtime,
                module,
                RuleSetId::Quake,
                qc_client_vm(),
                (0..8).collect(),
                7,
                20,
                &[],
            )
            .0
        })
        .collect();
    let mut host = Box::new(load_clients(runtime, requests, 1));
    let first = host
        .runtime
        .server
        .connect(
            qa_session::clients::Connection::Bot,
            ModuleId(1),
            Some(NativeEntity {
                module: ModuleId(1),
                slot: 1,
            }),
        )
        .unwrap();
    assert_eq!(first, ClientId(0));
    assert_eq!(
        host.connect_module_client(ModuleId(1), first, b"\\name\\first")
            .unwrap(),
        0,
    );
    let second = host
        .runtime
        .server
        .connect(
            qa_session::clients::Connection::Bot,
            ModuleId(2),
            Some(native),
        )
        .unwrap();
    assert_eq!(second, ClientId(1));
    assert!(host.runtime.snapshots[second.0 as usize].is_none());
    assert_eq!(host.client_info(ModuleId(2)).unwrap().clients, 1);
    (host, first, second)
}

#[test]
fn quakec_bot_native_namespace_is_independent_of_common_client_id() {
    let native = NativeEntity {
        module: ModuleId(2),
        slot: 1,
    };
    let (mut host, first, second) = qc_bot_namespace_host(native);
    let first_entity = host.runtime.server.clients[first.0 as usize].entity;
    let first_calls = host.module_counts(ModuleId(1)).unwrap().calls;
    let canonical = host.runtime.server.clients[second.0 as usize].entity;
    assert_eq!(
        host.connect_module_client(ModuleId(2), second, b"\\name\\second")
            .unwrap(),
        0,
    );
    assert_eq!(
        host.runtime.server.clients[second.0 as usize].entity,
        canonical
    );
    assert_eq!(host.runtime.server.module_moves(second), Some(native));
    assert_eq!(
        host.runtime.server.clients[second.0 as usize].module_spawn_parms[0],
        999.0f32.to_bits(),
    );
    assert_eq!(
        host.runtime.server.clients[first.0 as usize].entity,
        first_entity
    );
    assert_eq!(
        host.runtime.server.module_moves(first),
        Some(NativeEntity {
            module: ModuleId(1),
            slot: 1,
        }),
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, first_calls);
}

#[test]
fn quakec_bot_native_namespace_rejects_other_module_and_world_edict_without_rebinding() {
    for native in [
        NativeEntity {
            module: ModuleId(1),
            slot: 2,
        },
        NativeEntity {
            module: ModuleId(2),
            slot: 0,
        },
    ] {
        let (mut host, first, second) = qc_bot_namespace_host(native);
        let first_entity = host.runtime.server.clients[first.0 as usize].entity;
        let canonical = host.runtime.server.clients[second.0 as usize].entity;
        let calls =
            [ModuleId(1), ModuleId(2)].map(|module| host.module_counts(module).unwrap().calls);
        assert!(
            host.connect_module_client(ModuleId(2), second, b"\\name\\invalid")
                .is_err(),
        );
        assert_eq!(
            host.runtime.server.clients[second.0 as usize].entity,
            canonical
        );
        assert_eq!(
            host.runtime.server.entities.columns.native_entity[canonical.slot as usize],
            Some(native),
        );
        assert!(host.runtime.server.module_moves(second).is_none());
        assert_eq!(
            host.runtime.server.clients[first.0 as usize].entity,
            first_entity
        );
        assert_eq!(
            host.runtime.server.module_moves(first),
            Some(NativeEntity {
                module: ModuleId(1),
                slot: 1,
            }),
        );
        assert_eq!(
            [ModuleId(1), ModuleId(2)].map(|module| host.module_counts(module).unwrap().calls),
            calls,
        );
    }
}

#[test]
fn qw_client_lifecycle_uses_canonical_slot_shared_movement_and_packet_postthink_once() {
    use qa_world::collision::WorldTrace;
    let (mut owned, id, geometry, canonical, anchor) = qc_client_host(
        RuleSetId::QuakeWorld,
        RuleSetId::QuakeWorld,
        qc_client_vm(),
        &[],
    );
    let host = &mut *owned;
    let info = host.client_info(ModuleId(1)).unwrap();
    assert_eq!(
        (info.rules, info.clients, info.userinfo_bytes),
        (
            RuleSetId::QuakeWorld,
            32,
            qa_compat::quakec::QW_USERINFO_BYTES
        )
    );
    assert_eq!(
        host.connect_module_client(ModuleId(1), id, b"\\name\\native")
            .unwrap(),
        0
    );
    assert_ne!(canonical, anchor);
    let client = &host.runtime.server.clients[id.0 as usize];
    assert_eq!(client.entity, canonical);
    assert_eq!(client.module_spawn_parms[0], 999.0f32.to_bits());
    assert!(client.module_movement);
    assert_eq!(
        host.runtime.server.entities.columns.native_entity[canonical.slot as usize],
        Some(NativeEntity {
            module: ModuleId(1),
            slot: 1
        })
    );
    assert_eq!(
        (client.player.movement_rules, client.player.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake3)
    );
    assert_eq!(client.player.health, 0); // native .5 is alive for movement
    let command = UserCmd {
        duration_ms: 200,
        movement: [200.0, 0.0, 0.0],
        units: qa_gameplay::rules::command_units(RuleSetId::QuakeWorld),
        ..Default::default()
    };
    let mut expected = PlayerState {
        movement_rules: client.player.movement_rules,
        trace_rules: client.player.trace_rules,
        movement: client.player.movement,
        body: client.player.body,
        view_angles: client.player.view_angles,
        ..Default::default()
    };
    qa_movement::set_bounds(&mut expected);
    expected.movement.mode = MovementMode::Walk;
    expected.movement.tuning.max_speed = Some(320.0.into());
    for leaf in qa_movement::quakeworld_steps(command) {
        let mut scratch = host.runtime.geometry.scratch();
        let mut trace = WorldTrace::new(
            &host.runtime.geometry,
            geometry,
            0,
            &host.runtime.server.entities,
            &host.runtime.server.area,
            &mut scratch,
            CollisionOwner::Lifetime(canonical),
        );
        qa_movement::pmove(
            leaf,
            &mut expected,
            &mut trace,
            Default::default(),
            qa_movement::no_boundary,
        )
        .unwrap();
    }
    let before = host.module_counts(ModuleId(1)).unwrap().calls;
    assert!(host.runtime.send_client_command(
        id,
        &command,
        EventTime::from_milliseconds(200),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    host.drain(&mut Default::default());
    let client = &host.runtime.server.clients[id.0 as usize];
    assert_eq!(client.player.body.position, expected.body.position);
    assert_eq!(client.player.body.velocity, expected.body.velocity);
    assert_eq!(client.player.movement.mode, MovementMode::Walk);
    assert_eq!((client.player.health, client.player.armor), (4, 1));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 5); // four PreThink, one packet PostThink
    let position = client.player.body.position;
    let world = host.runtime.collision.as_mut().unwrap();
    assert_eq!(
        host.runtime.server.move_pending_clients(
            &host.runtime.geometry,
            geometry,
            0,
            &mut world.scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            |_, error| panic!("{error:?}")
        ),
        0
    );
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        position
    );
    tick(host, 201);
    assert_eq!(host.runtime.server.clients[id.0 as usize].player.armor, 1);
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        position
    );
    let before_shutdown = host.module_counts(ModuleId(1)).unwrap().calls;
    host.shutdown_modules();
    assert!(
        host.runtime.server.clients[id.0 as usize]
            .connection
            .is_none()
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().calls,
        before_shutdown + 1
    );
}

fn replace_client_function(
    vm: &mut Vm,
    entry: usize,
    body: &[(qa_formats::program::quakec::Opcode, i16, i16, i16)],
) {
    let mut statements = vm.image.statements.to_vec();
    vm.image.functions[entry].first_statement = statements.len() as i32;
    statements.extend(body.iter().map(|&(opcode, a, b, c)| {
        qa_formats::program::quakec::Statement {
            opcode,
            source_opcode: opcode as u16,
            operands: [a, b, c],
        }
    }));
    vm.image.statements = statements.into_boxed_slice();
}

#[test]
fn netquake_queued_client_runs_none_phases_and_shared_think_at_native_frame_time() {
    let mut vm = qc_client_vm();
    for (at, word) in [
        (110, 7.0f32.to_bits()),
        (111, 6),
        (112, 34),
        (113, 35),
        (114, 47),
        (115, 6),
        (116, 50),
        (117, 52),
        (118, 29),
    ] {
        vm.image.globals[at] = word;
    }
    replace_client_function(
        &mut vm,
        4,
        &[
            (LoadF, 34, 81, 91),
            (AddF, 91, 88, 92),
            (LoadF, 34, 116, 93),
            (AddF, 92, 93, 92),
            (Address, 34, 81, 90),
            (StorePF, 92, 90, 0),
            (Address, 34, 112, 90),
            (StorePF, 110, 90, 0),
            (Address, 34, 113, 90),
            (StorePF, 111, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        6,
        &[
            (LoadF, 34, 81, 91),
            (AddF, 91, 91, 92),
            (Address, 34, 81, 90),
            (StorePF, 92, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        5,
        &[
            (LoadF, 34, 89, 91),
            (AddF, 91, 88, 92),
            (Address, 34, 89, 90),
            (StorePF, 92, 90, 0),
            (LoadV, 34, 114, 120),
            (Address, 34, 115, 90),
            (StorePV, 120, 90, 0),
            (LoadF, 34, 117, 91),
            (Address, 34, 118, 90),
            (StorePF, 91, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    let (mut owned, id, geometry, canonical, anchor) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake3, vm, &[]);
    let host = &mut *owned;
    host.connect_module_client(
        ModuleId(1),
        id,
        b"\\name\\native nq\\topcolor\\3\\bottomcolor\\7",
    )
    .unwrap();
    let client = &host.runtime.server.clients[id.0 as usize];
    assert_ne!(canonical, anchor);
    assert_eq!(client.entity, canonical);
    assert_eq!(client.module_spawn_parms[0], 999.0f32.to_bits());
    assert_eq!(
        host.runtime.server.module_moves(id),
        Some(NativeEntity {
            module: ModuleId(1),
            slot: 1
        })
    );
    assert_eq!(
        (client.player.movement_rules, client.player.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake3)
    );
    assert_eq!((client.player.health, client.player.armor), (0, 0));
    tick(host, 0);
    let before = host.module_counts(ModuleId(1)).unwrap().calls;
    let command = UserCmd {
        duration_ms: 200,
        movement: [200.0, 0.0, 0.0],
        units: qa_gameplay::rules::command_units(RuleSetId::Quake),
        view_angles: Vec3([45.0, 90.0, 0.0]),
        buttons: buttons::ATTACK | buttons::JUMP,
        impulse: 9,
        ..Default::default()
    };
    assert!(host.runtime.send_client_command(
        id,
        &command,
        EventTime::from_milliseconds(20),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    // Framed command ingress consumes no client physics or callbacks.
    host.drain(&mut Default::default());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before);
    tick(host, 20);
    let client = &host.runtime.server.clients[id.0 as usize];
    assert_eq!((client.player.health, client.player.armor), (5, 1));
    assert_eq!(
        host.runtime.server.entities.columns.appearance[canonical.slot as usize].frame,
        9
    );
    assert_eq!(
        host.runtime.server.entities.columns.angles[canonical.slot as usize],
        command.view_angles
    );
    assert_eq!(client.player.body.position, Vec3::default());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 4); // StartFrame, PreThink, think, PostThink.
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
        0
    );
    let world = host.runtime.collision.as_mut().unwrap();
    assert_eq!(
        host.runtime.server.move_pending_clients(
            &host.runtime.geometry,
            geometry,
            0,
            &mut world.scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            |_, error| panic!("{error:?}")
        ),
        0
    );
    // Provider phases continue with the retained command when no new packet arrives.
    tick(host, 40);
    let client = &host.runtime.server.clients[id.0 as usize];
    assert_eq!((client.player.health, client.player.armor), (14, 2));
    assert_eq!(
        host.runtime.server.entities.columns.appearance[canonical.slot as usize].frame,
        9
    );
    assert_eq!(client.player.body.position, Vec3::default());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 8);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
}

#[test]
fn netquake_prethink_sanitizes_raw_fields_before_projection_and_routes_diagnostics() {
    let mut vm = qc_client_vm();
    for (at, word) in [(110, 0), (111, 3)] {
        vm.image.globals[at] = word;
    }
    for (at, value) in [
        (112, f32::NAN),
        (113, -0.0),
        (114, f32::INFINITY),
        (115, f32::NEG_INFINITY),
        (116, 2001.0),
        (117, -2001.0),
    ] {
        vm.image.globals[at] = value.to_bits();
    }
    replace_client_function(
        &mut vm,
        4,
        &[
            (Address, 34, 110, 90),
            (StorePV, 112, 90, 0),
            (Address, 34, 111, 90),
            (StorePV, 115, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(&mut vm, 5, &[(LoadV, 34, 110, 120), (Return, 120, 0, 0)]);
    let (mut owned, id, _, canonical, _) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake3, vm, &[]);
    let host = &mut *owned;
    host.connect_module_client(ModuleId(1), id, b"\\name\\sanitize")
        .unwrap();
    tick(host, 0);
    tick(host, 20);
    let body = host.runtime.server.clients[id.0 as usize].player.body;
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.0f32.to_bits(),
            (-0.0f32).to_bits(),
            0.0f32.to_bits()
        ]))
    );
    assert_eq!(body.position, Vec3::default());
    assert_eq!(body.velocity, Vec3([0.0, 2000.0, -2000.0]));
    assert_eq!(
        host.runtime.server.entities.columns.position[canonical.slot as usize],
        body.position
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    let messages = host
        .console
        .scrollback
        .chunks(&host.runtime.server.events.texts)
        .filter(|bytes| bytes.starts_with(b"Got a NaN "))
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        [
            b"Got a NaN velocity on \n".to_vec(),
            b"Got a NaN origin on \n".to_vec(),
            b"Got a NaN origin on \n".to_vec()
        ]
    );
}

#[test]
fn netquake_none_explicitly_relinks_and_retouches_triggers_before_postthink() {
    use qa_world::area::{LinkFlags, LinkIntent};
    let mut vm = qc_client_vm();
    for (at, word) in [(110, 0), (111, 10.0f32.to_bits())] {
        vm.image.globals[at] = word;
    }
    for (at, value) in [(112, 64.0f32), (113, 0.0), (114, 0.0)] {
        vm.image.globals[at] = value.to_bits();
    }
    replace_client_function(
        &mut vm,
        4,
        &[
            (Address, 34, 110, 90),
            (StorePV, 112, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        6,
        &[
            (LoadF, 36, 89, 91),
            (AddF, 91, 111, 92),
            (Address, 36, 89, 90),
            (StorePF, 92, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    write(&mut vm, 2, b"solid", 1.0f32.to_bits());
    write(&mut vm, 2, b"touch", 6);
    write_vector(&mut vm, 2, b"origin", Vec3([64.0, 0.0, 0.0]));
    write_vector(&mut vm, 2, b"mins", Vec3([-16.0; 3]));
    write_vector(&mut vm, 2, b"maxs", Vec3([16.0; 3]));
    let (mut owned, id, _, canonical, _) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake3, vm, &[(2, 0.0, 0.0, 0)]);
    let host = &mut *owned;
    host.connect_module_client(ModuleId(1), id, b"\\name\\trigger")
        .unwrap();
    let trigger = native_entity(host, ModuleId(1), 2);
    assert!(host.runtime.server.area.link(
        &host.runtime.server.entities,
        trigger,
        LinkFlags::TRIGGER,
        qa_gameplay::rules::link_order(RuleSetId::Quake),
        LinkIntent::Explicit
    ));
    tick(host, 0);
    let before = host.module_counts(ModuleId(1)).unwrap().calls;
    tick(host, 20);
    assert_eq!(host.runtime.server.clients[id.0 as usize].player.armor, 11);
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        Vec3([64.0, 0.0, 0.0])
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 4); // frame, PreThink, touch, PostThink
    let bounds = host.runtime.server.area.bounds(canonical).unwrap();
    assert!(bounds.mins.0[0] > 40.0 && bounds.maxs.0[0] > 64.0);
    tick(host, 40);
    assert_eq!(host.runtime.server.clients[id.0 as usize].player.armor, 22);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 8);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
}

#[test]
fn netquake_load_selects_frame_clients_and_rejects_inactive_connection() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let (req, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        qc_client_vm(),
        (0..8).collect(),
        7,
        20,
        &[],
    );
    let mut host = load(runtime, vec![req]);
    assert_eq!(
        host.client_info(ModuleId(1)).unwrap().rules,
        RuleSetId::Quake
    );
    assert!(
        host.connect_module_client(ModuleId(1), ClientId(0), b"\\name\\nq")
            .is_err()
    );
}

#[test]
fn netquake_native_inventory_reaches_client_and_hud_through_queued_packets() {
    use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::loopback::Endpoint;
    use qa_network::commands::packet::Protocol;

    for extra_items in [false, true] {
        let mut fields = FIELDS.to_vec();
        fields.extend_from_slice(&[
            (2, 40, b"health".as_slice()),
            (2, 41, b"armorvalue"),
            (2, 42, b"frags"),
            (2, 43, b"currentammo"),
            (2, 44, b"ammo_shells"),
            (2, 45, b"ammo_nails"),
            (2, 46, b"ammo_rockets"),
            (2, 47, b"ammo_cells"),
            (2, 48, b"items"),
            (2, 49, b"weapon"),
            (1, 50, b"weaponmodel"),
            (2, 51, b"weaponframe"),
            (3, 52, b"view_ofs"),
            (3, 55, b"punchangle"),
            (3, 58, b"v_angle"),
            (2, 61, b"waterlevel"),
            (2, 62, b"watertype"),
        ]);
        if extra_items {
            fields.push((2, 63, b"items2"));
        }
        let mut globals = GLOBALS.to_vec();
        globals.push((2, 38, b"serverflags"));
        let bytes = quakec_program::program_with_definitions(
            &[(Return, 35, 0, 0)],
            &[0, 1],
            &globals,
            &fields,
            64,
            128,
        );
        let mut vm = Vm::load(
            Image::parse(&bytes, Some(5927)).unwrap(),
            Layout {
                entities: 64,
                header_bytes: 16,
                extra_string_bytes: 128,
                state_step: 0.1,
                random_seed: 1,
            },
        )
        .unwrap();
        vm.image.globals[38] = 15.0f32.to_bits();
        let raw_items = 1 | 2 | 256 | 8192 | 131072;
        for (name, value) in [
            (b"health".as_slice(), 0.5),
            (b"armorvalue", 12.75),
            (b"frags", 9.0),
            (b"currentammo", 300.0),
            (b"ammo_shells", 300.0),
            (b"ammo_nails", -3.0),
            (b"ammo_rockets", 13.0),
            (b"ammo_cells", 12.0),
            (b"items", raw_items as f32),
            (b"weapon", 2.0),
            (b"weaponframe", 258.0),
        ] {
            write(&mut vm, 1, name, value.to_bits());
        }
        if extra_items {
            write(&mut vm, 1, b"items2", 3.0f32.to_bits());
        }
        write_vector(&mut vm, 1, b"view_ofs", Vec3([0.0, 0.0, 22.0]));
        let module = ModuleId(1);
        let mut runtime = Runtime::load(1, []).unwrap();
        let id = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3([16.0, 24.0, 32.0]),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: RuleSetId::Quake3,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::Quake2,
                },
                Protocol::NetQuake15,
                0,
            )
            .unwrap();
        assert!(
            runtime
                .server
                .bind_client_native(id, NativeEntity { module, slot: 1 })
        );
        let (req, _) = request(
            &mut runtime,
            module,
            RuleSetId::Quake,
            vm,
            vec![1],
            1,
            10,
            &[],
        );
        let mut owned = Box::new(load(runtime, vec![req]));
        let host = &mut *owned;
        host.local_clients[SeatId::FIRST.index()] = Some(id);
        let values = host.runtime.catalog.hud.values.native(RuleSetId::Quake);
        let mask_binding = values.items.unwrap();
        let stat15 = values.stats[15];
        let expected_mask = raw_items | if extra_items { 3 << 23 } else { 15 << 28 };
        let weapon = host
            .runtime
            .catalog
            .registry
            .native_weapon(RuleSetId::Quake, 2)
            .unwrap();
        let ammo = [11, 12, 13, 14].map(|number| {
            host.runtime
                .catalog
                .registry
                .native_item(RuleSetId::Quake, number)
                .unwrap()
        });

        tick(host, 0);
        tick(host, 100);
        assert!(host.module_counts(module).unwrap().calls > 0);
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        let server = &host.runtime.server.clients[id.0 as usize].player;
        assert_eq!(mask_binding.export(&server.values), Some(expected_mask));
        assert_eq!(server.inventory.item_counts[ammo[0].0 as usize], 300);
        assert_eq!(server.inventory.item_counts[ammo[1].0 as usize], -3);
        let prediction = &host.runtime.prediction[SeatId::FIRST.index()].player;
        assert_eq!((prediction.health, prediction.armor), (0, 12));
        assert_eq!(prediction.view_weapon.frame, 2);
        assert_eq!(prediction.inventory.weapon, weapon);
        assert_eq!(mask_binding.export(&prediction.values), Some(expected_mask));
        assert_eq!(values_of_items(prediction, ammo), [44, 253, 13, 12]);
        assert_eq!(
            prediction.values.get(stat15.id),
            Some(NumericValue::integer(0))
        );
        assert_eq!(
            (prediction.movement_rules, prediction.trace_rules),
            (RuleSetId::Quake3, RuleSetId::Quake2)
        );
        assert!(
            host.runtime
                .network
                .get(id, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_some()
        );
        assert!(
            host.runtime.snapshots[id.0 as usize]
                .as_ref()
                .unwrap()
                .applied
                > 0
        );
        assert_eq!(
            host.runtime.snapshots[id.0 as usize]
                .as_ref()
                .unwrap()
                .rejected,
            0
        );
        let hud = &host.runtime.server.clients[id.0 as usize].hud;
        assert_eq!(hud.inventory.weapon, weapon);
        assert_eq!(hud.inventory.item_counts[ammo[0].0 as usize], 44);
        assert_eq!(hud.inventory.item_counts[ammo[1].0 as usize], 253);

        host.runtime
            .server
            .events
            .push_delivered(
                NativeDelivery {
                    module,
                    source: RuleSetId::Quake,
                    target: NativeTarget::Client(id),
                    origin: Vec3::default(),
                    reliable: true,
                    dupe_key: 0,
                    component: 0,
                    emission: None,
                    check_portals: false,
                },
                FrameEvent::PlayerValue {
                    id: stat15.id,
                    value: NumericValue::integer(0x1234),
                },
            )
            .unwrap();
        tick(host, 200);
        tick(host, 300);
        let player = &host.runtime.prediction[SeatId::FIRST.index()].player;
        assert_eq!(
            player.values.get(stat15.id),
            Some(NumericValue::integer(0x1234))
        );
        assert_eq!(mask_binding.export(&player.values), Some(expected_mask));
        assert_eq!(values_of_items(player, ammo), [44, 253, 13, 12]);
        assert_eq!(player.inventory.weapon, weapon);
        assert_eq!(
            host.module_state(module),
            Some(qa_app::modules::State::Running)
        );
        assert_eq!(host.client_info(module).unwrap().rules, RuleSetId::Quake);
        assert!(host.runtime.server.module_moves(id).is_none()); // Prebound publication does not connect a module client.
    }
}

fn values_of_items(player: &PlayerState, items: [ItemId; 4]) -> [i32; 4] {
    items.map(|item| player.inventory.item_counts[item.0 as usize])
}

#[test]
fn qc_trap_publishes_admitted_entvar_writes_and_the_next_call_remains_healthy() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 70, 0, 0)]);
        let failing = code.function(&[
            (Address, 34, 53, 90),
            (StorePV, 80, 90, 0),
            (Call0, 91, 0, 0),
            (Return, 70, 0, 0),
        ]);
        let inspect = code.function(&[(LoadV, 34, 53, 94), (Return, 94, 0, 0)]);
        let mut vm = code.vm();
        let position = Vec3([11.0, 22.0, 33.0]);
        vm.image.globals[80..83].copy_from_slice(&position.0.map(f32::to_bits));
        vm.image.globals[91] = u32::MAX;
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame, failing, inspect],
            frame,
            100,
            &[(2, 0.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        let call = CallbackCall::Export {
            entity: ids[0],
            time: ThinkTime::Seconds(5.0),
            arguments: [0; 9],
            client: None,
            command: CommandArgs::default(),
        };
        assert!(host.call_module(CallbackId(1), call).is_err());
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize],
            position
        );
        assert!(host.runtime.server.area.bounds(ids[0]).is_none());
        host.call_module(CallbackId(2), call).unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC(position.0.map(f32::to_bits)))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Running)
        );
    }
}

#[test]
fn qc_failed_entry_restores_prepared_disconnect_globals_before_the_next_call() {
    let mut vm = qc_client_vm();
    // NewParameters reads the retained globals without preparing self/time.
    let inspect = vm.image.functions[1].first_statement as usize;
    vm.image.statements[inspect].opcode = Return;
    vm.image.statements[inspect].source_opcode = Return as u16;
    vm.image.statements[inspect].operands = [34, 0, 0];
    // A builtin cannot be entered as a bytecode callback. Preparation still
    // happens before the runner rejects this selected entry.
    vm.image.functions[6].first_statement = -12;
    let (mut host, id, _, canonical, _) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake3, vm, &[]);
    host.connect_module_client(ModuleId(1), id, b"\\name\\restore")
        .unwrap();
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(7)),
        ThinkTime::Seconds(5.0),
    )
    .unwrap();
    let client_call = |event| CallbackCall::Client {
        entity: canonical,
        client: id,
        event,
        time: ThinkTime::Seconds(99.0),
        arguments: [0; 9],
        command: CommandArgs::default(),
    };
    assert!(
        host.call_module(CallbackId(6), client_call(ClientCallback::Disconnect))
            .is_err()
    );
    host.call_module(CallbackId(1), client_call(ClientCallback::NewParameters))
        .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([0, 5.0f32.to_bits(), 0]))
    );
    assert!(host.runtime.server.clients[id.0 as usize].is_active());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
}

#[test]
fn qw_recovered_packet_stops_at_client_callback_trap_and_reconnects_cleanly() {
    use qa_app::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::loopback::Endpoint;
    use qa_network::commands::packet::Protocol;
    let mut vm = qc_client_vm();
    let first = vm.image.functions[4].first_statement as usize;
    vm.image.statements[first].opcode = Call0;
    vm.image.statements[first].source_opcode = Call0 as u16;
    vm.image.statements[first].operands = [189, 0, 0];
    vm.image.globals[189] = u32::MAX;
    let (mut owned, id, _, _, _) =
        qc_client_host(RuleSetId::QuakeWorld, RuleSetId::QuakeWorld, vm, &[]);
    let host = &mut *owned;
    host.connect_module_client(ModuleId(1), id, b"\\name\\trap")
        .unwrap();
    let before = host.module_counts(ModuleId(1)).unwrap().calls;
    let command = UserCmd {
        duration_ms: 100,
        movement: [200.0, 0.0, 0.0],
        units: qa_gameplay::rules::command_units(RuleSetId::QuakeWorld),
        ..Default::default()
    };
    assert!(host.runtime.send_client_command(
        id,
        &command,
        EventTime::from_milliseconds(100),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    // Drop exactly one framed packet, causing native QW command recovery on
    // the next packet. No command history or direct ACK is invented here.
    host.runtime.loopback.clear_client(id);
    assert!(host.runtime.send_client_command(
        id,
        &command,
        EventTime::from_milliseconds(200),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    host.drain(&mut Default::default());
    assert_eq!(host.runtime.network.commands, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 2); // one failed PreThink, then Disconnect; no tail/PostThink
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
    assert!(
        host.runtime.server.clients[id.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.network.get(id, Endpoint::Server).is_none());
    assert!(host.runtime.network.get(id, Endpoint::Client).is_none());
    tick(host, 201); // normal Presentation retires the old audio lifetime
    let reconnected = host
        .runtime
        .connect_local(
            SeatId::ALL[0],
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::QuakeWorld,
                trace: RuleSetId::Quake3,
            },
            Protocol::QuakeWorld28,
            0,
        )
        .unwrap();
    assert_eq!(reconnected, id);
    assert!(host.runtime.network.get(id, Endpoint::Server).is_some());
    host.connect_module_client(ModuleId(1), id, b"\\name\\retry")
        .unwrap();
    assert!(host.runtime.server.clients[id.0 as usize].module_movement);
}

#[test]
fn qw_native_command_subdivision_precedes_foreign_q3_movement_limits() {
    use qa_world::collision::WorldTrace;
    let (mut owned, id, geometry, canonical, _) = qc_client_host(
        RuleSetId::QuakeWorld,
        RuleSetId::Quake3,
        qc_client_vm(),
        &[],
    );
    let host = &mut *owned;
    host.connect_module_client(ModuleId(1), id, b"\\name\\foreign")
        .unwrap();
    let client = &host.runtime.server.clients[id.0 as usize];
    let mut expected = PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake3,
        movement: client.player.movement,
        body: client.player.body,
        view_angles: client.player.view_angles,
        ..Default::default()
    };
    qa_movement::set_bounds(&mut expected);
    expected.body.position = expected.body.position + Vec3([-1.0, -1.0, 0.0]); // native QW mins to selected Q3 mins
    expected.movement.tuning.max_speed = Some(320.0.into());
    let command = UserCmd {
        duration_ms: 250,
        movement: [200.0, 0.0, 0.0],
        units: qa_gameplay::rules::command_units(RuleSetId::QuakeWorld),
        ..Default::default()
    };
    let leaves = qa_movement::quakeworld_steps(command).collect::<Vec<_>>();
    assert_eq!(leaves.len(), 8);
    assert_eq!(leaves.iter().map(|cmd| cmd.duration_ms).sum::<u16>(), 248);
    for mut leaf in leaves {
        leaf.server_time_ms = expected
            .movement
            .command_time_ms
            .wrapping_add(i32::from(leaf.duration_ms));
        let mut scratch = host.runtime.geometry.scratch();
        let mut trace = WorldTrace::new(
            &host.runtime.geometry,
            geometry,
            0,
            &host.runtime.server.entities,
            &host.runtime.server.area,
            &mut scratch,
            CollisionOwner::Lifetime(canonical),
        );
        qa_movement::pmove(
            leaf,
            &mut expected,
            &mut trace,
            Default::default(),
            qa_movement::no_boundary,
        )
        .unwrap();
    }
    let before = host.module_counts(ModuleId(1)).unwrap().calls;
    assert!(host.runtime.send_client_command(
        id,
        &command,
        EventTime::from_milliseconds(250),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    host.drain(&mut Default::default());
    let client = &host.runtime.server.clients[id.0 as usize];
    assert_eq!(client.player.body.position, expected.body.position);
    assert_eq!(client.player.body.velocity, expected.body.velocity);
    assert_eq!(client.player.movement.command_time_ms, 248);
    assert_eq!((client.player.health, client.player.armor), (8, 1));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, before + 9);
}

fn noclip_client_vm(velocity: Vec3) -> Vm {
    let mut vm = qc_client_vm();
    let first = vm.image.functions[3].first_statement as usize;
    let mut begin = vm.image.statements[first..]
        .iter()
        .take_while(|s| s.opcode != Return)
        .map(|s| (s.opcode, s.operands[0], s.operands[1], s.operands[2]))
        .collect::<Vec<_>>();
    begin.extend_from_slice(&[
        (Address, 34, 125, 90),
        (StorePF, 126, 90, 0),
        (Address, 34, 127, 90),
        (StorePV, 128, 90, 0),
        (Address, 34, 131, 90),
        (StorePV, 132, 90, 0),
        (Return, 0, 0, 0),
    ]);
    vm.image.globals[125] = 36;
    vm.image.globals[126] = 8.0f32.to_bits();
    vm.image.globals[127] = 3;
    vm.image.globals[128..131].copy_from_slice(&velocity.0.map(f32::to_bits));
    vm.image.globals[131] = 37;
    vm.image.globals[132..135].copy_from_slice(&[1000.0f32, 2000.0, 3000.0].map(f32::to_bits));
    replace_client_function(&mut vm, 3, &begin);
    vm
}

#[test]
fn netquake_noclip_control_precedes_frame_and_integrates_think_velocity_once() {
    let mut vm = noclip_client_vm(Vec3::default());
    for (at, word) in [
        (110, 3),
        (111, 37),
        (112, 34),
        (113, 35),
        (114, 6),
        (115, 0),
        (150, vm.stride as u32),
        (151, 0),
    ] {
        vm.image.globals[at] = word;
    }
    vm.image.globals[120..123].copy_from_slice(&[11.0f32, 5.0, -2.0].map(f32::to_bits));
    vm.image.globals[180..183].copy_from_slice(&[7.0f32, -2.0, 3.0].map(f32::to_bits));
    replace_client_function(
        &mut vm,
        7,
        &[
            // StartFrame publishes the still-unintegrated origin and controlled
            // velocity to the world row, independently of the client callbacks.
            (LoadV, 150, 110, 97),
            (Address, 150, 111, 90),
            (StorePV, 97, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        4,
        &[
            (Address, 34, 110, 90),
            (StorePV, 180, 90, 0),
            (Address, 34, 112, 90),
            (StorePF, 35, 90, 0),
            (Address, 34, 113, 90),
            (StorePF, 114, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        6,
        &[
            (Address, 34, 110, 90),
            (StorePV, 120, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    let (mut host, id, geometry, canonical, _) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake, vm, &[]);
    host.connect_module_client(ModuleId(1), id, b"\\name\\noclip")
        .unwrap();
    tick(&mut host, 0);
    let command = UserCmd {
        movement: [3.0, 2.0, 6.0],
        units: qa_gameplay::rules::command_units(RuleSetId::Quake),
        buttons: buttons::JUMP | buttons::CROUCH,
        ..Default::default()
    };
    assert!(host.runtime.send_client_command(
        id,
        &command,
        EventTime::from_milliseconds(20),
        &mut Source {
            time: EventTime(0),
            polls: 0
        }
    ));
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        Vec3::default()
    );
    tick(&mut host, 20);
    let columns = &host.runtime.server.entities.columns;
    assert_eq!(
        columns.angular_velocity[canonical.slot as usize]
            .unwrap()
            .0
            .map(f32::to_bits),
        [3.0f32, -2.0, 6.0].map(f32::to_bits)
    );
    let advanced = Vec3([0.02f32 * 11.0, 0.02f32 * 5.0, 0.02f32 * -2.0]);
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        advanced
    );
    assert_eq!(columns.position[canonical.slot as usize], advanced);
    assert_eq!(
        columns.angles[canonical.slot as usize].0.map(f32::to_bits),
        [-0.0f32, 0.0, 0.0].map(f32::to_bits)
    );
    assert!(host.runtime.server.area.bounds(canonical).is_some());
    let world = host.runtime.collision.as_mut().unwrap();
    assert_eq!(
        host.runtime.server.move_pending_clients(
            &host.runtime.geometry,
            geometry,
            0,
            &mut world.scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            |_, error| panic!("{error:?}")
        ),
        0
    );
    tick(&mut host, 40);
    let columns = &host.runtime.server.entities.columns;
    assert_eq!(
        columns.angular_velocity[canonical.slot as usize].unwrap().0[0].to_bits(),
        3.0f32.to_bits()
    );
    assert_eq!(
        columns.angular_velocity[canonical.slot as usize].unwrap().0[2].to_bits(),
        6.0f32.to_bits()
    );
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        advanced + advanced
    );
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!(
        (counts.rejected, counts.traps, counts.unsupported_physics),
        (0, 0, 0)
    );
}

#[test]
fn netquake_captured_noclip_continues_after_think_changes_mode_to_none() {
    let mut vm = noclip_client_vm(Vec3::default());
    for (at, word) in [
        (110, 3),
        (111, 34),
        (112, 35),
        (113, 6),
        (114, 36),
        (115, 0),
    ] {
        vm.image.globals[at] = word;
    }
    vm.image.globals[120..123].copy_from_slice(&[7.0f32, -2.0, 3.0].map(f32::to_bits));
    replace_client_function(
        &mut vm,
        4,
        &[
            (Address, 34, 111, 90),
            (StorePF, 35, 90, 0),
            (Address, 34, 112, 90),
            (StorePF, 113, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    replace_client_function(
        &mut vm,
        6,
        &[
            (Address, 34, 110, 90),
            (StorePV, 120, 90, 0),
            (Address, 34, 114, 90),
            (StorePF, 115, 90, 0),
            (Return, 0, 0, 0),
        ],
    );
    let (mut host, id, _, canonical, _) =
        qc_client_host(RuleSetId::Quake, RuleSetId::Quake, vm, &[]);
    host.connect_module_client(ModuleId(1), id, b"\\name\\captured")
        .unwrap();
    tick(&mut host, 0);
    tick(&mut host, 20);
    let advanced = Vec3([0.02f32 * 7.0, 0.02f32 * -2.0, 0.02f32 * 3.0]);
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        advanced
    );
    assert_eq!(
        host.runtime.server.entities.columns.motion[canonical.slot as usize],
        Motion::None
    );
    tick(&mut host, 40);
    assert_eq!(
        host.runtime.server.clients[id.0 as usize]
            .player
            .body
            .position,
        advanced
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
        0
    );
}

#[test]
fn netquake_foreign_noclip_preserves_substeps_and_final_native_snap() {
    for (rules, seconds, milliseconds, axes, velocity, steps) in [
        (
            RuleSetId::Quake3,
            0.15,
            150,
            [127.0, -19.0, 23.0],
            Vec3([0.25, -0.125, 0.375]),
            3,
        ),
        (
            RuleSetId::Quake2,
            0.001,
            1,
            [50.0, 0.0, 0.0],
            Vec3::default(),
            1,
        ),
    ] {
        use qa_world::collision::WorldTrace;
        let mut vm = noclip_client_vm(velocity);
        for entry in [4, 5, 7] {
            replace_client_function(&mut vm, entry, &[(Return, 0, 0, 0)]);
        }
        let (mut host, id, geometry, canonical, _) =
            qc_client_host(RuleSetId::Quake, rules, vm, &[]);
        host.connect_module_client(ModuleId(1), id, b"\\name\\foreign q3")
            .unwrap();
        tick(&mut host, 0);
        host.console
            .append(
                &format!("host_framerate {seconds}\n"),
                Context {
                    source: RuleSetId::Quake,
                    ..Context::default()
                },
            )
            .unwrap();
        host.execute_commands();
        host.runtime.server.clients[id.0 as usize]
            .player
            .trace_rules = RuleSetId::Quake2;
        let player = &host.runtime.server.clients[id.0 as usize].player;
        let base = player.movement.tuning;
        let mut expected = PlayerState {
            movement_rules: player.movement_rules,
            trace_rules: player.trace_rules,
            movement: player.movement,
            body: player.body,
            view_angles: player.view_angles,
            view: player.view,
            ..Default::default()
        };
        expected.movement.mode = MovementMode::Noclip;
        expected.movement.tuning = qa_console::movement::MovementHandles::bind(&host.console.cvars)
            .unwrap()
            .resolve(&host.console.cvars, rules, base);
        let command = UserCmd {
            movement: axes,
            units: qa_gameplay::rules::command_units(RuleSetId::Quake),
            ..Default::default()
        };
        let mut scratch = host.runtime.geometry.scratch();
        let mut trace = WorldTrace::new(
            &host.runtime.geometry,
            geometry,
            0,
            &host.runtime.server.entities,
            &host.runtime.server.area,
            &mut scratch,
            CollisionOwner::Lifetime(canonical),
        );
        let result = qa_movement::pmove(
            UserCmd {
                server_time_ms: milliseconds,
                duration_ms: milliseconds as u16,
                ..command
            },
            &mut expected,
            &mut trace,
            Default::default(),
            qa_movement::no_boundary,
        )
        .unwrap();
        assert_eq!(result.steps, steps);
        if rules == RuleSetId::Quake2 {
            assert_eq!(expected.body.position, Vec3::default());
        }
        assert_ne!(expected.body.velocity, player.body.velocity);
        assert!(host.runtime.send_client_command(
            id,
            &command,
            EventTime::from_milliseconds(20),
            &mut Source {
                time: EventTime(0),
                polls: 0
            }
        ));
        host.drain(&mut Default::default());
        tick(&mut host, 20);
        let actual = &host.runtime.server.clients[id.0 as usize].player;
        assert_eq!(
            actual.body.position.0.map(f32::to_bits),
            expected.body.position.0.map(f32::to_bits)
        );
        assert_eq!(
            actual.body.velocity.0.map(f32::to_bits),
            expected.body.velocity.0.map(f32::to_bits)
        );
        if rules == RuleSetId::Quake3 {
            assert_eq!(actual.movement.command_time_ms, milliseconds);
        }
        assert_eq!(
            (actual.movement_rules, actual.trace_rules),
            (rules, RuleSetId::Quake2)
        );
        assert_eq!(
            host.runtime.server.entities.columns.position[canonical.slot as usize],
            expected.body.position
        );
        let world = host.runtime.collision.as_mut().unwrap();
        assert_eq!(
            host.runtime.server.move_pending_clients(
                &host.runtime.geometry,
                geometry,
                0,
                &mut world.scratch,
                qa_core::primitives::PatchClip::All,
                None,
                None,
                |_, error| panic!("{error:?}")
            ),
            0
        );
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!(
            (counts.rejected, counts.traps, counts.unsupported_physics),
            (0, 0, 0)
        );
    }
}

#[test]
fn qc_walkmove_reentrant_triggers_preserve_posttouch_order_and_recover_after_traps() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for trap in [false, true] {
            let mut code = Code::new();
            let frame = code.function(&[(Return, 0, 0, 0)]);
            let walk = code.builtin(32);
            let setorigin = code.builtin(2);
            let setsize = code.builtin(4);
            let mut touch_body = vec![
                (MulF, 70, 71, 82),
                (AddF, 82, 72, 70),
                // This second QC32 parks the trigger's QC activation while
                // its own cursor visits and skips the trigger itself.
                (StoreF, 63, 4, 0),
                (StoreF, 63, 7, 0),
                (Call2, 58, 0, 0),
                (StoreF, 1, 83, 0),
                (StoreEnt, 34, 84, 0),
                (StoreEnt, 36, 85, 0),
                (MulF, 70, 71, 82),
                (AddF, 82, 73, 70),
            ];
            if trap {
                // A trapped callback still publishes admitted raw entvars.
                // Its parent must abort without applying the pending ground clear.
                touch_body.extend_from_slice(&[(Address, 36, 53, 98), (StorePV, 110, 98, 0)]);
            } else {
                touch_body.extend_from_slice(&[
                    (StoreEnt, 36, 4, 0),
                    (StoreV, 110, 7, 0),
                    (Call2, 59, 0, 0),
                ]);
            }
            touch_body.extend_from_slice(&[
                (Address, 36, 66, 98),
                (StorePF, 69, 98, 0),
                (StoreF, 75, 35, 0),
            ]);
            if trap {
                touch_body.push((Call0, 61, 0, 0));
            }
            touch_body.push((Return, 0, 0, 0));
            let touch = code.function(&touch_body);
            let root = code.function(&[
                (StoreF, 48, 70, 0),
                (StoreEnt, 64, 34, 0),
                (StoreEnt, 65, 36, 0),
                (StoreF, 63, 4, 0),
                (StoreF, 63, 7, 0),
                (Call2, 58, 0, 0),
                (StoreF, 1, 90, 0),
                (MulF, 70, 71, 82),
                (AddF, 82, 74, 70),
                (LoadF, 64, 66, 91),
                (StoreF, 35, 92, 0),
                // Capture before a later export prepares different globals.
                (StoreEnt, 34, 79, 0),
                (StoreEnt, 36, 80, 0),
                (Return, 90, 0, 0),
            ]);
            let link = code.function(&[
                (StoreEnt, 64, 4, 0),
                (StoreV, 100, 7, 0),
                (StoreV, 103, 10, 0),
                (Call3, 60, 0, 0),
                (StoreEnt, 65, 4, 0),
                (StoreV, 100, 7, 0),
                (StoreV, 103, 10, 0),
                (Call3, 60, 0, 0),
                (Return, 0, 0, 0),
            ]);
            let inspect_log = code.function(&[(Return, 70, 0, 0)]);
            let inspect_nested = code.function(&[(Return, 83, 0, 0)]);
            let inspect_globals = code.function(&[(Return, 79, 0, 0)]);
            // No floor path writes groundentity, so do not supply that optional
            // field just to make a shallow fixture admit the builtin.
            let mut fields = FIELDS.to_vec();
            fields.push((6, 40, b"touch"));
            let mut vm = code.vm_fields(&fields);
            let stride = vm.stride as u32;
            let actor = 2;
            let trigger = 3;
            let origin = Vec3([0.0, 0.0, 8.0]);
            let teleported = Vec3([80.0, 16.0, 8.0]);
            for native in [actor, trigger] {
                write_vector(&mut vm, native, b"origin", origin);
                write(&mut vm, native, b"flags", 1536.0f32.to_bits());
                write(
                    &mut vm,
                    native,
                    b"solid",
                    if native == actor {
                        2.0f32.to_bits()
                    } else {
                        1.0f32.to_bits()
                    },
                );
            }
            write(&mut vm, trigger, b"touch", touch);
            for (at, word) in [
                (58, walk),
                (59, setorigin),
                (60, setsize),
                (61, u32::MAX),
                (63, 0),
                (64, actor as u32 * stride),
                (65, trigger as u32 * stride),
                (66, vm.image.field(b"flags").unwrap() as u32),
                (69, 544.0f32.to_bits()),
                (71, 10.0f32.to_bits()),
                (72, 2.0f32.to_bits()),
                (73, 3.0f32.to_bits()),
                (74, 4.0f32.to_bits()),
                (75, 42.0f32.to_bits()),
            ] {
                vm.image.globals[at] = word;
            }
            vm.image.globals[100..103].fill((-1.0f32).to_bits());
            vm.image.globals[103..106].fill(1.0f32.to_bits());
            vm.image.globals[110..113].copy_from_slice(&teleported.0.map(f32::to_bits));
            let mut runtime = Runtime::load(1, []).unwrap();
            install_motion_world(&mut runtime, false);
            let (mut request, ids) = request(
                &mut runtime,
                ModuleId(1),
                rules,
                vm,
                (0..code.functions.len() as u32).collect(),
                frame,
                100,
                &[(actor, 0.0, 0.0, 0), (trigger, 0.0, 0.0, 0)],
            );
            request.context.trace_rules = rules;
            let mut healthy_code = Code::new();
            let healthy = healthy_code.function(&[(Return, 70, 0, 0)]);
            let mut healthy_vm = healthy_code.vm();
            healthy_vm.image.globals[70] = 77.0f32.to_bits();
            let (healthy_request, _) = self::request(
                &mut runtime,
                ModuleId(2),
                rules,
                healthy_vm,
                vec![healthy],
                healthy,
                100,
                &[],
            );
            let mut host = load(runtime, vec![request, healthy_request]);
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(link)),
                ThinkTime::Seconds(5.0),
            )
            .unwrap();
            assert!(host.runtime.server.area.bounds(ids[0]).is_some());
            assert!(host.runtime.server.area.bounds(ids[1]).is_some());
            let result = host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(root)),
                ThinkTime::Seconds(5.0),
            );
            if trap {
                assert!(result.is_err());
                assert_eq!(
                    host.runtime.server.entities.columns.flags[ids[0].slot as usize],
                    544
                );
            } else {
                result.unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC([
                        1.0f32.to_bits(),
                        32.0f32.to_bits(),
                        42.0f32.to_bits()
                    ]))
                );
                assert_eq!(
                    host.runtime.server.entities.columns.flags[ids[0].slot as usize],
                    32
                );
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(inspect_globals)),
                    ThinkTime::Seconds(6.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC([
                        actor as u32 * stride,
                        trigger as u32 * stride,
                        0
                    ]))
                );
            }
            assert_eq!(
                host.runtime.server.entities.columns.position[ids[0].slot as usize],
                teleported
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect_log)),
                ThinkTime::Seconds(6.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    (if trap { 123.0f32 } else { 1234.0f32 }).to_bits(),
                    10.0f32.to_bits(),
                    2.0f32.to_bits(),
                ]))
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect_nested)),
                ThinkTime::Seconds(6.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    1.0f32.to_bits(),
                    trigger as u32 * stride,
                    actor as u32 * stride
                ]))
            );
            host.call_module_export(
                ModuleId(2),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(6.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(2)).unwrap().last_result,
                Some(ModuleResult::QuakeC([77.0f32.to_bits(), 0, 0]))
            );
            let counts = host.module_counts(ModuleId(1)).unwrap();
            // The successful touch moves the cursor's saved next link (the
            // actor) to another area node. Existing traversal counts that
            // interruption once; a trapped touch aborts before completion.
            assert_eq!(
                (counts.rejected, counts.traps, counts.unsupported_physics),
                (u64::from(!trap), if trap { 2 } else { 0 }, 0)
            );
            assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
            assert_eq!(
                host.module_state(ModuleId(1)),
                Some(qa_app::modules::State::Running)
            );
        }
    }
}

#[test]
fn qc_walkmove_freed_mover_keeps_its_native_self_until_the_outer_function_returns() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 0, 0, 0)]);
    let walk = code.builtin(32);
    let setorigin = code.builtin(2);
    let setsize = code.builtin(4);
    let remove = code.builtin(15);
    let spawn = code.builtin(14);
    let touch = code.function(&[
        (MulF, 70, 71, 82),
        (AddF, 82, 72, 70),
        (StoreEnt, 36, 4, 0),
        (Call1, 61, 0, 0),
        (MulF, 70, 71, 82),
        (AddF, 82, 73, 70),
        (Return, 0, 0, 0),
    ]);
    let initialize = code.function(&[
        (Call0, 62, 0, 0),
        (StoreEnt, 1, 64, 0),
        (Address, 64, 66, 98),
        (StorePF, 76, 98, 0),
        (Address, 64, 67, 98),
        (StorePF, 77, 98, 0),
        (StoreEnt, 64, 4, 0),
        (StoreV, 110, 7, 0),
        (Call2, 59, 0, 0),
        (StoreEnt, 64, 4, 0),
        (StoreV, 100, 7, 0),
        (StoreV, 103, 10, 0),
        (Call3, 60, 0, 0),
        (Call0, 62, 0, 0),
        (StoreEnt, 1, 65, 0),
        (Address, 65, 66, 98),
        (StorePF, 76, 98, 0),
        (Address, 65, 67, 98),
        (StorePF, 78, 98, 0),
        (Address, 65, 68, 98),
        (StorePF, 69, 98, 0),
        (StoreEnt, 65, 4, 0),
        (StoreV, 110, 7, 0),
        (Call2, 59, 0, 0),
        (StoreEnt, 65, 4, 0),
        (StoreV, 100, 7, 0),
        (StoreV, 103, 10, 0),
        (Call3, 60, 0, 0),
        (Return, 0, 0, 0),
    ]);
    let root = code.function(&[
        (StoreF, 48, 70, 0),
        (StoreEnt, 64, 34, 0),
        (StoreEnt, 65, 36, 0),
        (StoreF, 63, 4, 0),
        (StoreF, 63, 7, 0),
        (Call2, 58, 0, 0),
        (StoreF, 1, 90, 0),
        (MulF, 70, 71, 82),
        (AddF, 82, 74, 70),
        (StoreF, 70, 91, 0),
        (StoreEnt, 34, 92, 0),
        (Return, 90, 0, 0),
    ]);
    let mut fields = FIELDS.to_vec();
    fields.push((6, 40, b"touch"));
    let mut vm = code.vm_fields(&fields);
    let stride = vm.stride as u32;
    for (at, word) in [
        (58, walk),
        (59, setorigin),
        (60, setsize),
        (61, remove),
        (62, spawn),
        (63, 0),
        (66, vm.image.field(b"flags").unwrap() as u32),
        (67, vm.image.field(b"solid").unwrap() as u32),
        (68, vm.image.field(b"touch").unwrap() as u32),
        (69, touch),
        (71, 10.0f32.to_bits()),
        (72, 2.0f32.to_bits()),
        (73, 3.0f32.to_bits()),
        (74, 4.0f32.to_bits()),
        (76, 1536.0f32.to_bits()),
        (77, 2.0f32.to_bits()),
        (78, 1.0f32.to_bits()),
    ] {
        vm.image.globals[at] = word;
    }
    vm.image.globals[100..103].fill((-1.0f32).to_bits());
    vm.image.globals[103..106].fill(1.0f32.to_bits());
    vm.image.globals[110..113].copy_from_slice(&[0.0f32, 0.0, 8.0].map(f32::to_bits));
    let mut runtime = Runtime::load(1, []).unwrap();
    install_motion_world(&mut runtime, false);
    let (mut request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        vm,
        (0..code.functions.len() as u32).collect(),
        frame,
        100,
        &[],
    );
    request.context.trace_rules = RuleSetId::Quake;
    let mut host = load(runtime, vec![request]);
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(initialize)),
        ThinkTime::Seconds(5.0),
    )
    .unwrap();
    let actor = native_entity(&host, ModuleId(1), 2);
    let trigger = native_entity(&host, ModuleId(1), 3);
    assert!(host.runtime.server.area.bounds(actor).is_some());
    host.call_module(
        CallbackId(root),
        CallbackCall::Export {
            entity: actor,
            time: ThinkTime::Seconds(5.0),
            arguments: [0; 9],
            client: None,
            command: CommandArgs::default(),
        },
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            1.0f32.to_bits(),
            1234.0f32.to_bits(),
            2 * stride
        ]))
    );
    assert!(host.runtime.server.entities.resolve(actor).is_none());
    assert!(host.runtime.server.area.bounds(actor).is_none());
    assert!(host.runtime.server.entities.resolve(trigger).is_some());
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(frame)),
        ThinkTime::Seconds(6.0),
    )
    .unwrap();
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

#[test]
fn qc_walkmove_anonymous_common_box_blocks_without_needing_native_ground() {
    use qa_world::{
        area::{LinkFlags, LinkIntent},
        collision::Contents,
    };
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let walk = code.builtin(32);
        let root = code.function(&[
            (StoreEnt, 64, 34, 0),
            (StoreF, 63, 4, 0),
            (StoreF, 71, 7, 0),
            (Call2, 58, 0, 0),
            (Return, 1, 0, 0),
        ]);
        let mut vm = code.vm();
        vm.image.globals[58] = walk;
        vm.image.globals[63] = 0;
        vm.image.globals[64] = 2 * vm.stride as u32;
        vm.image.globals[71] = 10.0f32.to_bits();
        write(&mut vm, 2, b"flags", 512.0f32.to_bits());
        write(&mut vm, 2, b"solid", 2.0f32.to_bits());
        write_vector(&mut vm, 2, b"mins", Vec3([-1.0; 3]));
        write_vector(&mut vm, 2, b"maxs", Vec3([1.0; 3]));
        let mut runtime = Runtime::load(1, []).unwrap();
        install_motion_world(&mut runtime, false);
        let (mut request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[(2, 0.0, 0.0, 0)],
        );
        request.context.trace_rules = rules;
        let mut host = load(runtime, vec![request]);
        let blocker = host
            .runtime
            .server
            .entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(9),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let columns = &mut host.runtime.server.entities.columns;
        let slot = blocker.slot as usize;
        columns.position[slot] = Vec3([10.0, 0.0, 0.0]);
        columns.mins[slot] = Vec3([-4.0, -4.0, -100.0]);
        columns.maxs[slot] = Vec3([4.0, 4.0, 100.0]);
        columns.collision_shape[slot] = CollisionShape::Box;
        columns.collision_contents[slot] = Contents::SOLID.0;
        assert!(columns.native_entity[slot].is_none());
        assert!(host.runtime.server.area.link(
            &host.runtime.server.entities,
            blocker,
            LinkFlags::SOLID,
            qa_gameplay::rules::link_order(rules),
            LinkIntent::Explicit,
        ));
        let links = host.runtime.server.area.relinks;
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(root)),
            ThinkTime::Seconds(5.0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0, 0, 0]))
        );
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize],
            Vec3::default()
        );
        assert_eq!(
            host.runtime.server.entities.columns.flags[ids[0].slot as usize],
            512
        );
        assert!(host.runtime.server.area.bounds(ids[0]).is_none());
        assert_eq!(host.runtime.server.area.relinks, links);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn qc_walkmove_lands_on_anonymous_ground_and_reads_its_pose_through_nonzero_alias() {
    use qa_world::{
        area::{LinkFlags, LinkIntent},
        collision::Contents,
    };
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let walk = code.builtin(32);
        let root = code.function(&[
            (StoreEnt, 64, 34, 0),
            (StoreF, 63, 4, 0),
            (StoreF, 71, 7, 0),
            (Call2, 58, 0, 0),
            (StoreF, 1, 90, 0),
            (LoadEnt, 34, 72, 91),
            (LoadV, 91, 73, 100),
            (StoreF, 100, 92, 0),
            (Return, 90, 0, 0),
        ]);
        let mut vm = code.motion_vm();
        vm.image.globals[58] = walk;
        vm.image.globals[63] = 0;
        vm.image.globals[64] = 2 * vm.stride as u32;
        vm.image.globals[71] = 10.0f32.to_bits();
        vm.image.globals[72] = 43;
        vm.image.globals[73] = 0;
        write(&mut vm, 2, b"flags", 512.0f32.to_bits());
        write(&mut vm, 2, b"solid", 2.0f32.to_bits());
        write_vector(&mut vm, 2, b"origin", Vec3([0.0, 0.0, 2.0]));
        write_vector(&mut vm, 2, b"mins", Vec3([-1.0; 3]));
        write_vector(&mut vm, 2, b"maxs", Vec3([1.0; 3]));
        let mut runtime = Runtime::load(1, []).unwrap();
        // Native CheckBottom ignores box entities. The world floor supports
        // this one-unit anonymous step while the full-body sweep hits the step.
        install_motion_world(&mut runtime, true);
        let (mut request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[(2, 0.0, 0.0, 0)],
        );
        request.context.trace_rules = rules;
        let mut host = load(runtime, vec![request]);
        let floor = host
            .runtime
            .server
            .entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(9),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let columns = &mut host.runtime.server.entities.columns;
        columns.set_body(
            floor.slot as usize,
            Body {
                position: Vec3([25.0, 0.0, 1.0]),
                mins: Vec3([-100.0, -100.0, -8.0]),
                maxs: Vec3([100.0, 100.0, 0.0]),
                ..Default::default()
            },
        );
        columns.collision_shape[floor.slot as usize] = CollisionShape::Box;
        columns.collision_contents[floor.slot as usize] = Contents::SOLID.0;
        assert!(columns.native_entity[floor.slot as usize].is_none());
        assert!(host.runtime.server.area.link(
            &host.runtime.server.entities,
            floor,
            LinkFlags::SOLID,
            qa_gameplay::rules::link_order(rules),
            LinkIntent::Explicit
        ));
        let bounds = host.runtime.server.area.bounds(floor).unwrap();
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(root)),
            ThinkTime::Seconds(5.0),
        )
        .unwrap();
        let Some(ModuleResult::QuakeC(words)) =
            host.module_counts(ModuleId(1)).unwrap().last_result
        else {
            unreachable!()
        };
        assert_eq!(words[0], 1.0f32.to_bits());
        assert_ne!(words[1], 0);
        assert_eq!(words[2], 25.0f32.to_bits());
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize].0[0],
            10.0
        );
        assert_ne!(
            host.runtime.server.entities.columns.flags[ids[0].slot as usize] & 512,
            0
        );
        assert_eq!(host.runtime.server.area.bounds(floor), Some(bounds));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    }
}

#[test]
fn qc_movetogoal_mobility_rejection_only_clears_scalar_return_without_random() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let goal = code.builtin(67);
        let random = code.builtin(7);
        let root = code.function(&[
            (StoreV, 100, 1, 0),
            (StoreF, 48, 4, 0),
            (Call1, 58, 0, 0),
            (StoreV, 1, 90, 0),
            (Call0, 59, 0, 0),
            (StoreF, 1, 93, 0),
            (Return, 90, 0, 0),
        ]);
        let inspect_random = code.function(&[(Return, 93, 0, 0)]);
        let mut fields = FIELDS.to_vec();
        fields.push((4, 40, b"goalentity"));
        // enemy, ideal_yaw and yaw_speed are absent on this rejected path.
        let mut vm = code.vm_fields(&fields);
        vm.image.globals[58] = goal;
        vm.image.globals[59] = random;
        vm.image.globals[100..103].copy_from_slice(&[73.0f32.to_bits(), 0x8000_0000, 0x7fc1_2345]);
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[(2, 0.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        host.call_module(
            CallbackId(root),
            CallbackCall::Export {
                entity: ids[0],
                time: ThinkTime::Seconds(5.0),
                arguments: [0; 9],
                client: None,
                command: CommandArgs::default(),
            },
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0, 0x8000_0000, 0x7fc1_2345]))
        );
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect_random)),
            ThinkTime::Seconds(5.0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0x3f0a_cf16, 0, 0]))
        );
        assert_eq!(host.runtime.server.area.relinks, 0);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn qc_movetogoal_close_enough_reads_world_and_freed_goal_backing_without_link_or_random() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for freed_goal in [false, true] {
            let mut code = Code::new();
            let frame = code.function(&[(Return, 0, 0, 0)]);
            let goal = code.builtin(67);
            let random = code.builtin(7);
            let spawn = code.builtin(14);
            let remove = code.builtin(15);
            let initialize = code.function(&[
                (Call0, 60, 0, 0),
                (StoreEnt, 1, 65, 0),
                (Address, 64, 66, 98),
                (StorePEnt, 65, 98, 0),
                (StoreEnt, 65, 4, 0),
                (Call1, 61, 0, 0),
                // Retained native backing is readable even after its common
                // lifetime is gone. These raw absolute bounds are not a link.
                (Address, 65, 67, 98),
                (StorePV, 106, 98, 0),
                (Address, 65, 68, 98),
                (StorePV, 109, 98, 0),
                (Return, 65, 0, 0),
            ]);
            let root = code.function(&[
                (StoreV, 100, 1, 0),
                (StoreF, 48, 4, 0),
                (Call1, 58, 0, 0),
                (StoreV, 1, 90, 0),
                (Call0, 59, 0, 0),
                (StoreF, 1, 93, 0),
                (Return, 90, 0, 0),
            ]);
            let inspect_random = code.function(&[(Return, 93, 0, 0)]);
            let mut vm = code.goal_vm();
            let stride = vm.stride as u32;
            for (at, value) in [
                (58, goal),
                (59, random),
                (60, spawn),
                (61, remove),
                (64, 2 * stride),
                (66, vm.image.field(b"goalentity").unwrap() as u32),
                (67, vm.image.field(b"absmin").unwrap() as u32),
                (68, vm.image.field(b"absmax").unwrap() as u32),
            ] {
                vm.image.globals[at] = value;
            }
            vm.image.globals[100..103].copy_from_slice(&[
                73.0f32.to_bits(),
                0x8000_0000,
                0x7fc1_2345,
            ]);
            vm.image.globals[106..112].fill(0);
            write(&mut vm, 2, b"flags", 512.0f32.to_bits());
            write(&mut vm, 2, b"enemy", 2 * stride);
            write_vector(&mut vm, 2, b"absmin", Vec3([-1.0; 3]));
            write_vector(&mut vm, 2, b"absmax", Vec3([1.0; 3]));
            let mut runtime = Runtime::load(1, []).unwrap();
            install_motion_world(&mut runtime, false);
            let (request, ids) = request(
                &mut runtime,
                ModuleId(1),
                rules,
                vm,
                (0..code.functions.len() as u32).collect(),
                frame,
                100,
                &[(2, 0.0, 0.0, 0)],
            );
            let mut host = load(runtime, vec![request]);
            if freed_goal {
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(initialize)),
                    ThinkTime::Seconds(5.0),
                )
                .unwrap();
                let ordinal = if rules == RuleSetId::QuakeWorld {
                    33
                } else {
                    3
                };
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC([ordinal * stride, 40, 18]))
                );
                assert!(!host.runtime.server.entities.active().any(|entity| {
                    host.runtime.server.entities.columns.native_entity[entity.slot as usize]
                        == Some(NativeEntity {
                            module: ModuleId(1),
                            slot: ordinal as i32,
                        })
                }));
            }
            let links = host.runtime.server.area.relinks;
            host.call_module(
                CallbackId(root),
                CallbackCall::Export {
                    entity: ids[0],
                    time: ThinkTime::Seconds(5.0),
                    arguments: [0; 9],
                    client: None,
                    command: CommandArgs::default(),
                },
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    73.0f32.to_bits(),
                    0x8000_0000,
                    0x7fc1_2345
                ]))
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect_random)),
                ThinkTime::Seconds(5.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([0x3f0a_cf16, 0, 0]))
            );
            assert_eq!(host.runtime.server.area.relinks, links);
            assert!(host.runtime.server.area.bounds(ids[0]).is_none());
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        }
    }
}

#[test]
fn qc_movetogoal_facing_restore_still_touches_and_keeps_callback_returns_or_trap_prefix() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for trap in [false, true] {
            let mut code = Code::new();
            let frame = code.function(&[(Return, 0, 0, 0)]);
            let goal = code.builtin(67);
            let setorigin = code.builtin(2);
            let setsize = code.builtin(4);
            let random = code.builtin(7);
            let mut touch_body = vec![
                (MulF, 70, 71, 82),
                (AddF, 82, 72, 70),
                (Call0, 62, 0, 0),
                (StoreF, 1, 86, 0),
                // StepDirection must restore the facing-rejected displacement
                // before linking and entering this overlapping trigger.
                (LoadV, 36, 53, 94),
            ];
            if trap {
                touch_body.extend_from_slice(&[(Address, 36, 53, 98), (StorePV, 112, 98, 0)]);
            } else {
                touch_body.extend_from_slice(&[
                    (StoreEnt, 36, 4, 0),
                    (StoreV, 112, 7, 0),
                    (Call2, 59, 0, 0),
                ]);
            }
            touch_body.extend_from_slice(&[
                (Address, 36, 66, 98),
                (StorePF, 76, 98, 0),
                (StoreF, 75, 35, 0),
                (MulF, 70, 71, 82),
                (AddF, 82, 73, 70),
                (StoreEnt, 63, 34, 0),
            ]);
            if trap {
                touch_body.push((Call0, 61, 0, 0));
            }
            touch_body.push((Return, 103, 0, 0));
            let touch = code.function(&touch_body);
            let root = code.function(&[
                (StoreF, 48, 70, 0),
                (StoreEnt, 65, 36, 0),
                (StoreV, 100, 1, 0),
                (StoreF, 71, 4, 0),
                (Call1, 58, 0, 0),
                (StoreV, 1, 90, 0),
                (MulF, 70, 71, 82),
                (AddF, 82, 74, 70),
                (StoreEnt, 34, 79, 0),
                (StoreEnt, 36, 80, 0),
                (StoreF, 35, 81, 0),
                (Call0, 62, 0, 0),
                (StoreF, 1, 97, 0),
                (Return, 90, 0, 0),
            ]);
            let link = code.function(&[
                (StoreEnt, 64, 4, 0),
                (StoreV, 106, 7, 0),
                (StoreV, 109, 10, 0),
                (Call3, 60, 0, 0),
                (StoreEnt, 65, 4, 0),
                (StoreV, 106, 7, 0),
                (StoreV, 109, 10, 0),
                (Call3, 60, 0, 0),
                (Return, 0, 0, 0),
            ]);
            let inspect_context = code.function(&[(Return, 79, 0, 0)]);
            let inspect_seen_origin = code.function(&[(Return, 94, 0, 0)]);
            let inspect_random = code.function(&[
                (StoreF, 86, 115, 0),
                (StoreF, 97, 116, 0),
                (StoreF, 70, 117, 0),
                (Return, 115, 0, 0),
            ]);
            let probe_random = code.function(&[(Call0, 62, 0, 0), (Return, 1, 0, 0)]);
            let mut vm = code.goal_vm();
            let stride = vm.stride as u32;
            for (at, value) in [
                (58, goal),
                (59, setorigin),
                (60, setsize),
                (61, u32::MAX),
                (62, random),
                (63, 0),
                (64, 2 * stride),
                (65, 3 * stride),
                (66, vm.image.field(b"flags").unwrap() as u32),
                (71, 10.0f32.to_bits()),
                (72, 2.0f32.to_bits()),
                (73, 3.0f32.to_bits()),
                (74, 4.0f32.to_bits()),
                (75, 42.0f32.to_bits()),
                (76, 544.0f32.to_bits()),
            ] {
                vm.image.globals[at] = value;
            }
            vm.image.globals[100..103].copy_from_slice(&[73.0f32, -5.0, 7.0].map(f32::to_bits));
            vm.image.globals[103..106].copy_from_slice(&[19.0f32, 23.0, 29.0].map(f32::to_bits));
            vm.image.globals[106..109].fill((-1.0f32).to_bits());
            vm.image.globals[109..112].fill(1.0f32.to_bits());
            vm.image.globals[112..115].copy_from_slice(&[0.0f32, 0.0, 32.0].map(f32::to_bits));
            write_vector(&mut vm, 2, b"origin", Vec3([0.0, 0.0, 8.0]));
            write_vector(&mut vm, 2, b"angles", Vec3([0.0, 180.0, 0.0]));
            write(&mut vm, 2, b"flags", 1536.0f32.to_bits());
            write(&mut vm, 2, b"solid", 2.0f32.to_bits());
            write_vector(&mut vm, 3, b"origin", Vec3([0.0, 0.0, 8.0]));
            write(&mut vm, 3, b"solid", 1.0f32.to_bits());
            write(&mut vm, 3, b"touch", touch);
            let mut runtime = Runtime::load(1, []).unwrap();
            install_motion_world(&mut runtime, false);
            let (mut primary, ids) = request(
                &mut runtime,
                ModuleId(1),
                rules,
                vm,
                (0..code.functions.len() as u32).collect(),
                frame,
                100,
                &[(2, 0.0, 0.0, 0), (3, 0.0, 0.0, 0)],
            );
            primary.context.trace_rules = rules;
            let mut peer_code = Code::new();
            let peer_frame = peer_code.function(&[(Return, 70, 0, 0)]);
            let mut peer_vm = peer_code.vm();
            peer_vm.image.globals[70] = 77.0f32.to_bits();
            let (peer, _) = request(
                &mut runtime,
                ModuleId(2),
                rules,
                peer_vm,
                vec![peer_frame],
                peer_frame,
                100,
                &[],
            );
            let mut host = load(runtime, vec![primary, peer]);
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(link)),
                ThinkTime::Seconds(5.0),
            )
            .unwrap();
            let result = host.call_module(
                CallbackId(root),
                CallbackCall::Export {
                    entity: ids[0],
                    time: ThinkTime::Seconds(5.0),
                    arguments: [0; 9],
                    client: None,
                    command: CommandArgs::default(),
                },
            );
            if trap {
                assert!(result.is_err());
            } else {
                result.unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC(
                        [19.0f32, 23.0, 29.0].map(f32::to_bits)
                    ))
                );
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(inspect_context)),
                    ThinkTime::Seconds(6.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC([
                        2 * stride,
                        3 * stride,
                        42.0f32.to_bits()
                    ]))
                );
            }
            // ClearGround belongs before StepDirection's touch. A later clear
            // copied from WalkMove would destroy this callback's new ground bit.
            assert_eq!(
                host.runtime.server.entities.columns.flags[ids[0].slot as usize],
                544
            );
            assert_eq!(
                host.runtime.server.entities.columns.position[ids[0].slot as usize],
                Vec3([0.0, 0.0, 32.0])
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect_seen_origin)),
                ThinkTime::Seconds(6.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([0.0f32, 0.0, 8.0].map(f32::to_bits)))
            );
            if trap {
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(probe_random)),
                    ThinkTime::Seconds(6.0),
                )
                .unwrap();
                let Some(ModuleResult::QuakeC(words)) =
                    host.module_counts(ModuleId(1)).unwrap().last_result
                else {
                    panic!()
                };
                assert_eq!(words[0], 0x3e43_4987);
            } else {
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(inspect_random)),
                    ThinkTime::Seconds(6.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC([
                        0x3e8f_191e,
                        0x3e43_4987,
                        1234.0f32.to_bits()
                    ]))
                );
            }
            host.call_module_export(
                ModuleId(2),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(6.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(2)).unwrap().last_result,
                Some(ModuleResult::QuakeC([77.0f32.to_bits(), 0, 0]))
            );
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!(
                (counts.rejected, counts.traps),
                (0, if trap { 2 } else { 0 })
            );
            assert_eq!(
                host.module_state(ModuleId(1)),
                Some(qa_app::modules::State::Running)
            );
        }
    }
}

#[test]
fn qc_movetogoal_blocked_step_touches_then_chases_the_captured_live_goal_with_shared_rng() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let goal = code.builtin(67);
        let setorigin = code.builtin(2);
        let setsize = code.builtin(4);
        let random = code.builtin(7);
        let second_touch = code.function(&[
            (MulF, 70, 71, 82),
            (AddF, 82, 73, 70),
            (Call0, 62, 0, 0),
            (StoreF, 1, 87, 0),
            (StoreEnt, 63, 34, 0),
            (Return, 103, 0, 0),
        ]);
        let first_touch = code.function(&[
            (MulF, 70, 71, 82),
            (AddF, 82, 72, 70),
            (Call0, 62, 0, 0),
            (StoreF, 1, 86, 0),
            (Address, 36, 66, 98),
            (StorePF, 48, 98, 0),
            (Address, 36, 68, 98),
            (StorePEnt, 77, 98, 0),
            // Goal pointer was captured before this callback, but its origin
            // must be read now: both a stale goal pose and the new goal point X.
            (Address, 76, 53, 98),
            (StorePV, 112, 98, 0),
            (Address, 34, 79, 98),
            (StorePFnc, 99, 98, 0),
            (Return, 103, 0, 0),
        ]);
        let root = code.function(&[
            (StoreF, 48, 70, 0),
            (StoreV, 100, 1, 0),
            (StoreF, 71, 4, 0),
            (Call1, 58, 0, 0),
            (StoreV, 1, 90, 0),
            (MulF, 70, 71, 82),
            (AddF, 82, 74, 70),
            (Call0, 62, 0, 0),
            (StoreF, 1, 97, 0),
            (Return, 90, 0, 0),
        ]);
        let link = code.function(&[
            (StoreEnt, 64, 4, 0),
            (StoreV, 106, 7, 0),
            (StoreV, 109, 10, 0),
            (Call3, 60, 0, 0),
            (StoreEnt, 65, 4, 0),
            (StoreV, 118, 7, 0),
            (StoreV, 121, 10, 0),
            (Call3, 60, 0, 0),
            (Return, 0, 0, 0),
        ]);
        let inspect_random = code.function(&[
            (StoreF, 86, 115, 0),
            (StoreF, 87, 116, 0),
            (StoreF, 97, 117, 0),
            (Return, 115, 0, 0),
        ]);
        let inspect_log = code.function(&[(Return, 70, 0, 0)]);
        let mut vm = code.goal_vm();
        let stride = vm.stride as u32;
        for (at, value) in [
            (58, goal),
            (59, setorigin),
            (60, setsize),
            (62, random),
            (63, 0),
            (64, 2 * stride),
            (65, 3 * stride),
            (66, vm.image.field(b"flags").unwrap() as u32),
            (68, vm.image.field(b"goalentity").unwrap() as u32),
            (71, 10.0f32.to_bits()),
            (72, 2.0f32.to_bits()),
            (73, 3.0f32.to_bits()),
            (74, 4.0f32.to_bits()),
            (76, 4 * stride),
            (77, 5 * stride),
            (79, vm.image.field(b"touch").unwrap() as u32),
            (99, second_touch),
        ] {
            vm.image.globals[at] = value;
        }
        vm.image.globals[100..103].copy_from_slice(&[73.0f32, -5.0, 7.0].map(f32::to_bits));
        vm.image.globals[103..106].copy_from_slice(&[19.0f32, 23.0, 29.0].map(f32::to_bits));
        vm.image.globals[106..109].fill((-1.0f32).to_bits());
        vm.image.globals[109..112].fill(1.0f32.to_bits());
        vm.image.globals[112..115].copy_from_slice(&[0.0f32, 50.0, 8.0].map(f32::to_bits));
        vm.image.globals[118..121].fill((-16.0f32).to_bits());
        vm.image.globals[121..124].fill(16.0f32.to_bits());
        write_vector(&mut vm, 2, b"origin", Vec3([0.0, 0.0, 8.0]));
        write(&mut vm, 2, b"flags", 512.0f32.to_bits());
        write(&mut vm, 2, b"solid", 2.0f32.to_bits());
        write(&mut vm, 2, b"goalentity", 4 * stride);
        write(&mut vm, 2, b"yaw_speed", 360.0f32.to_bits());
        write_vector(&mut vm, 3, b"origin", Vec3([0.0, 0.0, 8.0]));
        write(&mut vm, 3, b"solid", 1.0f32.to_bits());
        write(&mut vm, 3, b"touch", first_touch);
        for target in [4, 5] {
            write_vector(&mut vm, target, b"origin", Vec3([50.0, 0.0, 8.0]));
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        install_motion_world(&mut runtime, false);
        let (mut request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[
                (2, 0.0, 0.0, 0),
                (3, 0.0, 0.0, 0),
                (4, 0.0, 0.0, 0),
                (5, 0.0, 0.0, 0),
            ],
        );
        request.context.trace_rules = rules;
        let mut host = load(runtime, vec![request]);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(link)),
            ThinkTime::Seconds(5.0),
        )
        .unwrap();
        let links = host.runtime.server.area.relinks;
        host.call_module(
            CallbackId(root),
            CallbackCall::Export {
                entity: ids[0],
                time: ThinkTime::Seconds(5.0),
                arguments: [0; 9],
                client: None,
                command: CommandArgs::default(),
            },
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC(
                [19.0f32, 23.0, 29.0].map(f32::to_bits)
            ))
        );
        let expected = Vec3([0.0, 0.0, 8.0]) + qa_movement::yaw_displacement(90.0, 10.0);
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize]
                .0
                .map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
        assert_eq!(
            host.runtime.server.entities.columns.angles[ids[0].slot as usize].0[1].to_bits(),
            90.0f32.to_bits()
        );
        assert_eq!(host.runtime.server.area.relinks - links, 2);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect_random)),
            ThinkTime::Seconds(6.0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                0x3e8f_191e,
                (((1714636915u32 & 0x7fff) as f32) / 32767.0).to_bits(),
                (((1957747793u32 & 0x7fff) as f32) / 32767.0).to_bits(),
            ]))
        );
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect_log)),
            ThinkTime::Seconds(6.0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                1234.0f32.to_bits(),
                10.0f32.to_bits(),
                2.0f32.to_bits()
            ]))
        );
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!((counts.rejected, counts.traps), (0, 0));
    }
}

fn push_fixture(
    code: &Code,
    mut vm: Vm,
    rules: RuleSetId,
    frame: u32,
    size: u32,
    link: u32,
    think: u32,
    nextthink: f32,
    blocked: u32,
    rider: Vec3,
    wall: Option<Vec3>,
) -> (Box<FrameHost>, Vec<EntityId>) {
    let stride = vm.stride as u32;
    vm.image.globals[80] = 40 * stride;
    vm.image.globals[81] = stride;
    vm.image.globals[82] = 3 * stride;
    vm.image.globals[83] = size;
    vm.image.globals[86] = 4 * stride;
    vm.image.globals[90] = vm.image.field(b"ltime").unwrap() as u32;
    vm.image.globals[91] = vm.image.field(b"blocked").unwrap() as u32;
    vm.image.globals[93] = 1.0f32.to_bits();
    vm.image.globals[94] = 2.0f32.to_bits();
    vm.image.globals[95] = 42.0f32.to_bits();
    vm.image.globals[97] = vm.image.field(b"absmin").unwrap() as u32;
    vm.image.globals[98] = vm.image.field(b"absmax").unwrap() as u32;
    vm.image.globals[100..103].fill((-1.0f32).to_bits());
    vm.image.globals[103..106].fill(1.0f32.to_bits());
    vm.image.globals[106..109].fill((-4.0f32).to_bits());
    vm.image.globals[109..112].fill(4.0f32.to_bits());
    let model = vm.insert_string(b"*1").unwrap() as u32;
    write(&mut vm, 40, b"model", model);
    write(&mut vm, 40, b"solid", 4.0f32.to_bits());
    write(&mut vm, 40, b"ltime", 1.0f32.to_bits());
    write(&mut vm, 40, b"blocked", blocked);
    write_vector(&mut vm, 1, b"origin", rider);
    write(&mut vm, 1, b"solid", 2.0f32.to_bits());
    write(&mut vm, 1, b"flags", 512.0f32.to_bits());
    write(&mut vm, 1, b"groundentity", 40 * stride);
    if let Some(wall) = wall {
        write_vector(&mut vm, 3, b"origin", wall);
        write(&mut vm, 3, b"solid", 2.0f32.to_bits());
    }
    let mut runtime = Runtime::load(1, []).unwrap();
    install_motion_models(&mut runtime, false, true);
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        rules,
        vm,
        (0..code.functions.len() as u32).collect(),
        frame,
        // request() takes milliseconds, not Hz: admit one provider frame at
        // tick100 rather than accumulating ten float local-time increments.
        100,
        &[
            (1, 3.0, 0.0, 0),
            (40, 7.0, nextthink, think),
            (3, 0.0, 0.0, 0),
            (4, 0.0, 0.0, 0),
        ],
    );
    // Reserved native edict 1 participates in PUSH even when the ordinary
    // QW provider scan excludes client edicts. Its Body is the existing row.
    let mut host = Box::new(load(runtime, vec![request]));
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(link)),
        ThinkTime::Seconds(0.0),
    )
    .unwrap();
    tick(&mut host, 0);
    (host, ids)
}

#[test]
fn qc_push_carries_reserved_native_riders_and_uses_local_deadlines_once_with_world_time() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let think = code.function(&[
            (LoadF, 34, 90, 60),
            (StoreF, 35, 61, 0),
            (AddF, 62, 48, 62),
            // Already past after this callback: QW PUSH still calls only once.
            (Address, 34, 54, 99),
            (StorePF, 93, 99, 0),
            (Return, 60, 0, 0),
        ]);
        let inspect = code.function(&[(Return, 60, 0, 0)]);
        let (size, link) = code.push_links();
        let mut vm = code.push_vm();
        write_vector(&mut vm, 40, b"velocity", Vec3([16.0, 0.0, 0.0]));
        let (mut host, ids) = push_fixture(
            &code,
            vm,
            rules,
            frame,
            size,
            link,
            think,
            1.0625,
            0,
            Vec3([0.0, 0.0, 6.0]),
            None,
        );
        tick(&mut host, 100);
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[1].slot as usize],
            Vec3([1.0, 0.0, 0.0]),
            "{rules:?} PUSH actor; {:?}",
            host.module_counts(ModuleId(1))
        );
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize],
            Vec3([1.0, 0.0, 6.0])
        );
        tick(&mut host, 200);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect)),
            ThinkTime::Seconds(9.0),
        )
        .unwrap();
        let world_time = if rules == RuleSetId::Quake {
            0.0f32
        } else {
            0.1f32
        };
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                1.0625f32.to_bits(),
                world_time.to_bits(),
                1.0f32.to_bits()
            ]))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    }
}

#[test]
fn qc_push_blocked_callback_observes_restored_poses_and_reuses_undo_after_a_trap() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for trap in [false, true] {
            let mut code = Code::new();
            // Blocked preserves time left by the prior impact/direct move.
            // NQ PushEntity's wall impact sets world time even without touch;
            // QW's direct passenger move leaves this StartFrame value intact.
            let frame = code.function(&[(StoreF, 95, 35, 0), (Return, 0, 0, 0)]);
            let blocked = code.function(&[
                (LoadV, 34, 53, 60),
                (LoadV, 36, 53, 63),
                (LoadF, 34, 90, 66),
                (StoreF, 35, 67, 0),
                (AddF, 68, 48, 68),
                (Address, 34, 90, 99),
                (StorePF, 94, 99, 0),
                (Return, 66, 0, 0),
            ]);
            let failing = code.function(&[(Call0, 84, 0, 0), (Call0, 85, 0, 0), (Return, 0, 0, 0)]);
            let repair = code.function(&[
                (Address, 80, 91, 99),
                (StorePFnc, 84, 99, 0),
                (Return, 0, 0, 0),
            ]);
            let inspect = code.function(&[(LoadF, 80, 90, 69), (Return, 66, 0, 0)]);
            let seen_poses = code.function(&[(Return, 60, 0, 0)]);
            let seen_rider = code.function(&[(Return, 63, 0, 0)]);
            let (size, link) = code.push_links();
            let mut vm = code.push_vm();
            vm.image.globals[84] = blocked;
            vm.image.globals[85] = u32::MAX;
            write_vector(&mut vm, 40, b"velocity", Vec3([16.0, 0.0, 0.0]));
            let (mut host, ids) = push_fixture(
                &code,
                vm,
                rules,
                frame,
                size,
                link,
                0,
                100.0,
                if trap { failing } else { blocked },
                Vec3([5.5, 0.0, 0.0]),
                Some(Vec3([7.5625, 0.0, 0.0])),
            );
            tick(&mut host, 100);
            assert_eq!(
                host.runtime.server.entities.columns.position[ids[1].slot as usize],
                Vec3::default(),
                "{rules:?} trap={trap} blocked actor; {:?}",
                host.module_counts(ModuleId(1))
            );
            assert_eq!(
                host.runtime.server.entities.columns.position[ids[0].slot as usize],
                Vec3([5.5, 0.0, 0.0])
            );
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().traps,
                u64::from(trap)
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(seen_poses)),
                ThinkTime::Seconds(9.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([0; 3]))
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(seen_rider)),
                ThinkTime::Seconds(9.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([5.5f32.to_bits(), 0, 0]))
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect)),
                ThinkTime::Seconds(9.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    1.0f32.to_bits(),
                    (if rules == RuleSetId::Quake {
                        0.0f32
                    } else {
                        42.0f32
                    })
                    .to_bits(),
                    1.0f32.to_bits()
                ])),
                "{rules:?} trap={trap} first blocked globals"
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(repair)),
                ThinkTime::Seconds(9.0),
            )
            .unwrap();
            tick(&mut host, 200);
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(inspect)),
                ThinkTime::Seconds(9.0),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    (if rules == RuleSetId::Quake {
                        (2.0f32 + 0.1f32) - 0.1f32
                    } else {
                        2.0f32
                    })
                    .to_bits(),
                    (if rules == RuleSetId::Quake {
                        0.1f32
                    } else {
                        42.0f32
                    })
                    .to_bits(),
                    2.0f32.to_bits()
                ])),
                "{rules:?} trap={trap} second blocked globals"
            );
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().traps,
                u64::from(trap)
            );
        }
    }
}

#[test]
fn qc_qw_push_replays_only_a_significant_think_snap_through_the_same_passenger_move() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for displacement in [1.0f32, 1.0 / 128.0] {
            let mut code = Code::new();
            let frame = code.function(&[(Return, 0, 0, 0)]);
            let think = code.function(&[
                (Address, 34, 53, 99),
                (StorePV, 112, 99, 0),
                (Return, 0, 0, 0),
            ]);
            let (size, link) = code.push_links();
            let mut vm = code.push_vm();
            vm.image.globals[112..115].copy_from_slice(&[displacement.to_bits(), 0, 0]);
            let (mut host, ids) = push_fixture(
                &code,
                vm,
                rules,
                frame,
                size,
                link,
                think,
                1.0625,
                0,
                Vec3([0.0, 0.0, 6.0]),
                None,
            );
            // Zero velocity advances only local time; any passenger move is the
            // QW post-think snap, not the first movement transaction.
            tick(&mut host, 100);
            assert_eq!(
                host.runtime.server.entities.columns.position[ids[1].slot as usize],
                Vec3([displacement, 0.0, 0.0])
            );
            let carried = if rules == RuleSetId::QuakeWorld && displacement > 1.0 / 64.0 {
                displacement
            } else {
                0.0
            };
            assert_eq!(
                host.runtime.server.entities.columns.position[ids[0].slot as usize],
                Vec3([carried, 0.0, 6.0])
            );
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!((counts.rejected, counts.traps), (0, 0));
        }
    }
}

#[test]
fn qc_invalid_dirty_row_does_not_suppress_a_later_due_think() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let malformed = code.function(&[
            (Address, 34, 56, 99),
            (StorePF, 50, 99, 0),
            (Address, 34, 86, 99),
            (StorePS, 87, 99, 0),
            (Call0, 87, 0, 0),
            (Return, 0, 0, 0),
        ]);
        let later = code.function(&[
            (StoreEnt, 34, 4, 0),
            (StoreV, 90, 7, 0),
            (Call2, 84, 0, 0),
            (Return, 0, 0, 0),
        ]);
        let set_origin = code.builtin(2);
        let mut fields = FIELDS.to_vec();
        fields.push((1, 40, b"classname"));
        let mut vm = code.vm_fields(&fields);
        vm.image.globals[84] = set_origin;
        vm.image.globals[86] = 40; // classname is read even for nonsolid rows.
        vm.image.globals[87] = u32::MAX;
        vm.image.globals[90..93].copy_from_slice(&[9.0f32.to_bits(), 0, 0]);
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            100,
            &[(40, 0.0, 0.05, malformed), (41, 0.0, 0.05, later)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 100);
        let columns = &host.runtime.server.entities.columns;
        // A stores an invalid classname and traps. B still reaches setorigin
        // service before ordinary return publication retries A's pending row.
        assert_eq!(
            columns.position[ids[1].slot as usize],
            Vec3([9.0, 0.0, 0.0])
        );
        assert_eq!(columns.appearance[ids[0].slot as usize].frame, 0);
        for id in ids {
            assert_eq!(
                columns.next_think[id.slot as usize],
                Some(ThinkTime::Seconds(0.0))
            );
        }
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!(counts.calls, 3);
        // B's successful service does not discard A's failed dirty import.
        // Its return still rejects that pending row, producing the second trap.
        assert_eq!((counts.rejected, counts.traps), (2, 2));
    }
}

#[test]
fn qc_due_think_selects_the_function_after_clearing_an_aliased_native_deadline() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let stale = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
        let inspect = code.function(&[(LoadF, 80, 54, 71), (Return, 70, 0, 0)]);
        let fields = FIELDS
            .iter()
            .map(|&(kind, offset, name)| (kind, if name == b"think" { 34 } else { offset }, name))
            .collect::<Vec<_>>();
        let mut vm = code.vm_fields(&fields);
        vm.image.globals[80] = 40 * vm.stride as u32;
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, _) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            10,
            &[(40, 0.0, f32::from_bits(stale), stale)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 100);
        // Clearing nextthink also clears the native function cell. Executing
        // the pre-clear cached stale callback would increment this observable.
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(inspect)),
            ThinkTime::Seconds(2.0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0; 3]))
        );
        tick(&mut host, 200);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
    }
}

#[test]
fn qc_push_callbacks_see_committed_native_attachments_and_blocked_rollback_pose() {
    use qa_core::primitives::{BodyAttachment, BodyFollow};

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for blocked in [false, true] {
            let mut code = Code::new();
            let frame = code.function(&[(Return, 0, 0, 0)]);
            let callback = code.function(&[
                (LoadV, 86, 53, 60),
                (LoadV, 86, 97, 63),
                (LoadV, 86, 98, 66),
                (Return, 60, 0, 0),
            ]);
            let inspect_seen = code.function(&[(Return, 60, 0, 0)]);
            let inspect_min = code.function(&[(Return, 63, 0, 0)]);
            let inspect_max = code.function(&[(Return, 66, 0, 0)]);
            let (size, link) = code.push_links();
            let mut vm = code.push_vm();
            write_vector(&mut vm, 40, b"velocity", Vec3([16.0, 0.0, 0.0]));
            let (mut host, ids) = push_fixture(
                &code,
                vm,
                rules,
                frame,
                size,
                link,
                if blocked { 0 } else { callback },
                if blocked { 100.0 } else { 1.0625 },
                if blocked { callback } else { 0 },
                if blocked {
                    Vec3([5.5, 0.0, 0.0])
                } else {
                    Vec3([0.0, 0.0, 6.0])
                },
                blocked.then_some(Vec3([7.5625, 0.0, 0.0])),
            );
            let pusher = ids[1];
            let child = ids[3];
            assert_eq!(
                host.runtime.server.area.bounds(child),
                Some(Bounds {
                    mins: Vec3([-2.0; 3]),
                    maxs: Vec3([2.0; 3])
                })
            );
            host.runtime
                .server
                .entities
                .attach(
                    child,
                    BodyAttachment {
                        anchor: pusher,
                        follow: BodyFollow::Translation,
                        offset: Vec3([0.0, 0.0, 6.0]),
                    },
                )
                .unwrap();
            if !blocked {
                // This native rider is also attached. Its own native PUSH
                // attempt must not make the callback observe two translations.
                host.runtime
                    .server
                    .entities
                    .attach(
                        ids[0],
                        BodyAttachment {
                            anchor: pusher,
                            follow: BodyFollow::Translation,
                            offset: Vec3([0.0, 0.0, 6.0]),
                        },
                    )
                    .unwrap();
            }
            // No manual transport: the actual provider must commit both the
            // accepted +1 pose and the restored pose before its callback.
            tick(&mut host, 100);
            let expected = Vec3([if blocked { 0.0 } else { 1.0 }, 0.0, 6.0]);
            assert_eq!(
                host.runtime.server.entities.columns.position[child.slot as usize],
                expected,
                "{rules:?} blocked={blocked} attached child; {:?}",
                host.module_counts(ModuleId(1))
            );
            if !blocked {
                assert_eq!(
                    host.runtime.server.entities.columns.position[ids[0].slot as usize],
                    expected
                );
            }
            let bounds = host.runtime.server.area.bounds(child).unwrap();
            assert_eq!(
                bounds,
                Bounds {
                    mins: expected + Vec3([-2.0; 3]),
                    maxs: expected + Vec3([2.0; 3])
                },
                "{rules:?} blocked={blocked} linked expansion"
            );
            for (inspect, value) in [
                (inspect_seen, expected),
                (inspect_min, bounds.mins),
                (inspect_max, bounds.maxs),
            ] {
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(inspect)),
                    ThinkTime::Seconds(9.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC(value.0.map(f32::to_bits))),
                    "{rules:?} blocked={blocked} captured native attachment fields"
                );
            }
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!((counts.rejected, counts.traps), (0, 0));
        }
    }
}

#[test]
fn qc_failed_think_binding_preparation_publishes_its_aliased_native_clear() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let fields = FIELDS
            .iter()
            .copied()
            .filter(|(_, _, name)| *name != b"think")
            .map(|(kind, offset, name)| (kind, if name == b"nextthink" { 0 } else { offset }, name))
            .collect::<Vec<_>>();
        let mut vm = code.vm_fields(&fields);
        write_vector(&mut vm, 40, b"origin", Vec3([0.05, 2.0, 3.0]));
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame],
            frame,
            10,
            &[(40, 0.0, 0.05, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 100);
        // The absent typed think cell rejects preparation after nextthink's
        // native clear. Its aliased origin still publishes in this same pass.
        let columns = &host.runtime.server.entities.columns;
        assert_eq!(
            columns.position[ids[0].slot as usize],
            Vec3([0.0, 2.0, 3.0])
        );
        assert_eq!(
            columns.next_think[ids[0].slot as usize],
            Some(ThinkTime::Seconds(0.0))
        );
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!((counts.rejected, counts.traps), (1, 0));
    }
}

#[test]
fn qc_rejected_think_preparation_still_publishes_its_aliased_native_clear() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[(Return, 0, 0, 0)]);
        let fields = FIELDS
            .iter()
            .map(|&(kind, offset, name)| {
                (kind, if name == b"nextthink" { 0 } else { offset }, name)
            })
            .collect::<Vec<_>>();
        let mut vm = code.vm_fields(&fields);
        write_vector(&mut vm, 40, b"origin", Vec3([0.05, 2.0, 3.0]));
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, ids) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![frame],
            frame,
            10,
            &[(40, 0.0, 0.05, u32::MAX)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 100);
        // The bad selector reaches the existing missing-function adapter. Its
        // admitted clear updates common state before any later explicit call.
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[0].slot as usize],
            Vec3([0.0, 2.0, 3.0])
        );
        assert_eq!(
            host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
            Some(ThinkTime::Seconds(0.0))
        );
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!((counts.rejected, counts.traps), (1, 1));
    }
}

fn client_push_vm(code: &Code, idle: u32, begin: u32) -> Vm {
    let mut fields = FIELDS.to_vec();
    fields.extend_from_slice(&[
        (2, 40, b"gravity"),
        (2, 41, b"waterlevel"),
        (2, 42, b"watertype"),
        (4, 43, b"groundentity"),
        (6, 44, b"touch"),
        (2, 45, b"ltime"),
        (6, 46, b"blocked"),
        (2, 47, b"team"),
        (1, 48, b"netname"),
    ]);
    let mut globals = GLOBALS.to_vec();
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
        globals.push((6, 140 + index as u16, name));
    }
    let parms = (1..=16)
        .map(|index| format!("parm{index}"))
        .collect::<Vec<_>>();
    for (index, name) in parms.iter().enumerate() {
        globals.push((2, 160 + index as u16, name.as_bytes()));
    }
    let bytes = quakec_program::program_with_definitions(
        &code.operations,
        &code.functions,
        &globals,
        &fields,
        49,
        192,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 1024,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    vm.image.globals[140..146].fill(idle);
    vm.image.globals[142] = begin;
    vm.image.globals[48] = 1.0f32.to_bits();
    vm.image.globals[53] = 0;
    vm.image.globals[57] = 36;
    vm
}

#[test]
fn qc_push_moves_disconnected_and_never_connected_native_client_bodies() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for connected in [false, true] {
            for blocked in [false, true] {
                let mut code = Code::new();
                let idle = code.function(&[(Return, 0, 0, 0)]);
                let touch = code.function(&[(AddF, 71, 48, 71), (Return, 0, 0, 0)]);
                let blocked_call = code.function(&[
                    (LoadV, 34, 53, 60),
                    (LoadV, 36, 53, 63),
                    (AddF, 70, 48, 70),
                    (Return, 0, 0, 0),
                ]);
                let begin = code.function(&[
                    (Address, 34, 53, 99),
                    (StorePV, 112, 99, 0),
                    (Address, 34, 120, 99),
                    (StorePF, 116, 99, 0),
                    (Address, 34, 121, 99),
                    (StorePF, 117, 99, 0),
                    (Address, 34, 57, 99),
                    (StorePF, 118, 99, 0),
                    (Address, 34, 119, 99),
                    (StorePEnt, 80, 99, 0),
                    (Address, 34, 123, 99),
                    (StorePFnc, 124, 99, 0),
                    (Return, 0, 0, 0),
                ]);
                let size = code.builtin(4);
                let link = code.function(&[
                    (StoreEnt, 80, 4, 0),
                    (StoreV, 106, 7, 0),
                    (StoreV, 109, 10, 0),
                    (Call3, 83, 0, 0),
                    (StoreEnt, 82, 4, 0),
                    (StoreV, 100, 7, 0),
                    (StoreV, 103, 10, 0),
                    (Call3, 83, 0, 0),
                    (Return, 0, 0, 0),
                ]);
                let link_rider = code.function(&[
                    (StoreEnt, 81, 4, 0),
                    (StoreV, 100, 7, 0),
                    (StoreV, 103, 10, 0),
                    (Call3, 83, 0, 0),
                    (Return, 0, 0, 0),
                ]);
                let inspect = code.function(&[(Return, 70, 0, 0)]);
                let inspect_rider = code.function(&[(LoadV, 81, 53, 75), (Return, 75, 0, 0)]);
                let mut vm = client_push_vm(&code, idle, begin);
                let stride = vm.stride as u32;
                let rider = if blocked {
                    Vec3([5.5, 0.0, 0.0])
                } else {
                    Vec3([0.0, 0.0, 6.0])
                };
                for (at, word) in [
                    (80, 40 * stride),
                    (81, stride),
                    (82, 3 * stride),
                    (83, size),
                    (116, 512.0f32.to_bits()),
                    (117, 2.0f32.to_bits()),
                    (118, 3.0f32.to_bits()),
                    (119, 43),
                    (120, 25),
                    (121, 24),
                    (123, 44),
                    (124, touch),
                ] {
                    vm.image.globals[at] = word;
                }
                vm.image.globals[100..103].fill((-1.0f32).to_bits());
                vm.image.globals[103..106].fill(1.0f32.to_bits());
                vm.image.globals[106..109].fill((-4.0f32).to_bits());
                vm.image.globals[109..112].fill(4.0f32.to_bits());
                vm.image.globals[112..115].copy_from_slice(&rider.0.map(f32::to_bits));
                let model = vm.insert_string(b"*1").unwrap() as u32;
                write(&mut vm, 40, b"model", model);
                write(&mut vm, 40, b"solid", 4.0f32.to_bits());
                write(&mut vm, 40, b"ltime", 1.0f32.to_bits());
                write(&mut vm, 40, b"blocked", blocked_call);
                write_vector(&mut vm, 40, b"velocity", Vec3([10.0, 0.0, 0.0]));
                if !connected {
                    write_vector(&mut vm, 1, b"origin", rider);
                    write_vector(&mut vm, 1, b"mins", Vec3([-1.0; 3]));
                    write_vector(&mut vm, 1, b"maxs", Vec3([1.0; 3]));
                    write(&mut vm, 1, b"solid", 2.0f32.to_bits());
                    write(&mut vm, 1, b"flags", 512.0f32.to_bits());
                    write(&mut vm, 1, b"movetype", 3.0f32.to_bits());
                    write(&mut vm, 1, b"groundentity", 40 * stride);
                    write(&mut vm, 1, b"touch", touch);
                }
                if blocked {
                    write_vector(&mut vm, 3, b"origin", Vec3([7.5625, 0.0, 0.0]));
                    write(&mut vm, 3, b"solid", 2.0f32.to_bits());
                }
                let mut runtime = Runtime::load(1, []).unwrap();
                install_motion_models(&mut runtime, false, true);
                let (request, rows) = request(
                    &mut runtime,
                    ModuleId(1),
                    rules,
                    vm,
                    (0..code.functions.len() as u32).collect(),
                    idle,
                    100,
                    &[(40, 7.0, 100.0, 0), (3, 0.0, 0.0, 0)],
                );
                let mut host = Box::new(load(runtime, vec![request]));
                let native = NativeEntity {
                    module: ModuleId(1),
                    slot: 1,
                };
                let old = if connected {
                    let client = host
                        .runtime
                        .server
                        .connect(
                            qa_session::clients::Connection::Bot,
                            ModuleId(1),
                            Some(native),
                        )
                        .unwrap();
                    host.connect_module_client(ModuleId(1), client, b"\\name\\rider")
                        .unwrap();
                    let entity = host.runtime.server.clients[client.0 as usize].entity;
                    host.call_module_export(
                        ModuleId(1),
                        Export::clocked(CallbackId(link_rider)),
                        ThinkTime::Seconds(0.0),
                    )
                    .unwrap();
                    let bounds = host.runtime.server.area.bounds(entity);
                    let links = host.runtime.server.area.relinks;
                    assert!(
                        host.client_string_command(client, rules, b"disconnect")
                            .unwrap()
                            .is_break()
                    );
                    assert!(host.runtime.server.entities.resolve(entity).is_none());
                    assert!(
                        host.runtime.server.clients[client.0 as usize]
                            .connection
                            .is_none()
                    );
                    let orphan = native_entity(&host, ModuleId(1), 1);
                    assert_ne!(orphan, entity);
                    assert_eq!(host.runtime.server.area.bounds(orphan), bounds);
                    assert_eq!(host.runtime.server.area.relinks, links);
                    Some(entity)
                } else {
                    assert!(
                        host.runtime
                            .server
                            .entities
                            .find_native(native, None)
                            .is_none()
                    );
                    None
                };
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(link)),
                    ThinkTime::Seconds(0.0),
                )
                .unwrap();
                tick(&mut host, 0);
                tick(&mut host, 100);
                let body = native_entity(&host, ModuleId(1), 1);
                if let Some(old) = old {
                    assert!(host.runtime.server.entities.resolve(old).is_none());
                    assert_ne!(body, old);
                }
                let expected = rider + Vec3([if blocked { 0.0 } else { 1.0 }, 0.0, 0.0]);
                assert_eq!(
                    host.runtime.server.entities.columns.position[body.slot as usize], expected,
                    "{rules:?} connected={connected} blocked={blocked}"
                );
                assert_eq!(
                    host.runtime.server.entities.columns.position[rows[0].slot as usize],
                    Vec3([if blocked { 0.0 } else { 1.0 }, 0.0, 0.0])
                );
                assert!(host.runtime.server.area.bounds(body).is_some());
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(inspect_rider)),
                    ThinkTime::Seconds(9.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC(expected.0.map(f32::to_bits)))
                );
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(inspect)),
                    ThinkTime::Seconds(9.0),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC([
                        (u8::from(blocked) as f32).to_bits(),
                        if blocked && rules == RuleSetId::Quake {
                            1.0f32.to_bits()
                        } else {
                            0
                        },
                        0
                    ]))
                );
                let counts = host.module_counts(ModuleId(1)).unwrap();
                assert_eq!((counts.rejected, counts.traps), (0, 0));
                assert_eq!(counts.unsupported_physics, 0);
            }
        }
    }
}

#[test]
fn qc_native_client_reconnect_reclaims_only_its_exact_orphan_body() {
    for other_module in [false, true] {
        let first_native = NativeEntity {
            module: ModuleId(1),
            slot: 1,
        };
        let second_native = NativeEntity {
            module: ModuleId(2),
            slot: 1,
        };
        let (mut host, first, unused) = qc_bot_namespace_host(second_native);
        let old = host.runtime.server.clients[first.0 as usize].entity;
        assert!(
            host.client_string_command(first, RuleSetId::Quake, b"disconnect")
                .unwrap()
                .is_break()
        );
        let orphan = native_entity(&host, ModuleId(1), 1);
        assert!(host.runtime.server.entities.resolve(old).is_none());
        assert_ne!(orphan, old);
        if other_module {
            assert!(
                host.client_string_command(unused, RuleSetId::Quake, b"disconnect")
                    .unwrap()
                    .is_break()
            );
        }
        let native = if other_module {
            second_native
        } else {
            first_native
        };
        let reconnected = host
            .runtime
            .server
            .connect(
                qa_session::clients::Connection::Bot,
                native.module,
                Some(native),
            )
            .unwrap();
        assert_eq!(reconnected, first);
        let replacement = host.runtime.server.clients[first.0 as usize].entity;
        host.connect_module_client(native.module, first, b"\\name\\replacement")
            .unwrap();
        assert_eq!(host.runtime.server.module_moves(first), Some(native));
        assert_eq!(native_entity(&host, native.module, 1), replacement);
        if other_module {
            assert_eq!(native_entity(&host, ModuleId(1), 1), orphan);
            assert!(host.runtime.server.entities.resolve(orphan).is_some());
        } else {
            assert!(host.runtime.server.entities.resolve(orphan).is_none());
        }
        assert!(host.runtime.server.entities.resolve(old).is_none());
        assert_eq!(host.module_counts(native.module).unwrap().rejected, 0);
    }
}

#[test]
fn qw_remote_output_overflow_retires_the_connection_and_retains_its_native_body() {
    use qa_core::{
        events::{EventRing, OutputTarget},
        loopback::Endpoint,
    };
    use qa_network::{
        channel::Channel,
        commands::{connection::Commands, packet::Protocol},
        ingress::{Connection, Route},
    };
    use qa_world::area::{LinkFlags, LinkIntent};

    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.server.events = EventRing::load(8, 8, 64, 256).unwrap();
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .unwrap();
    let (request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::QuakeWorld,
        qc_client_vm(),
        (0..8).collect(),
        7,
        100,
        &[],
    );
    let client = runtime
        .server
        .connect(qa_session::clients::Connection::Remote, ModuleId(1), None)
        .unwrap();
    let protocol = Protocol::QuakeWorld28;
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.set_qport(2000);
    runtime
        .bind_connection(
            client,
            Connection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: 0,
                    peer: Peer::Socket("127.0.0.1:2000".parse().unwrap()),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
            qa_app::snapshots::NativePlayerBinding::Supplied(0),
            None,
        )
        .unwrap();
    let mut host = Box::new(load(runtime, vec![request]));
    host.connect_module_client(ModuleId(1), client, b"\\name\\stalled")
        .unwrap();
    let old = host.runtime.server.clients[client.0 as usize].entity;
    let native = NativeEntity {
        module: ModuleId(1),
        slot: 1,
    };
    let server = &mut host.runtime.server;
    server.area.link(
        &server.entities,
        old,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit,
    );
    let bounds = server.area.bounds(old).unwrap();
    let body = server.entities.columns.body(old.slot as usize);
    let output = server.clients[client.0 as usize].output.unwrap();
    for value in 0..=server.events.capacity() {
        server
            .events
            .push_to(
                OutputTarget::Client(client),
                FrameEvent::Hud(HudEvent {
                    client,
                    change: HudChange::Collectibles(value as u64),
                }),
            )
            .unwrap();
        if server.events.needs_resync(output) {
            break;
        }
    }
    assert!(server.events.needs_resync(output));
    let counters = server.events.counters(output).unwrap();
    let overflow = counters.overflow;
    let retired = counters.retired_on_resync;
    let result = host.frame(
        &mut Source {
            time: EventTime::from_milliseconds(0),
            polls: 0,
        },
        true,
    );
    assert_eq!(result.output_drains, 1);
    assert_eq!(result.output.native_disconnected, 1);
    assert_eq!(result.output.native_overflow, overflow);
    assert_eq!(result.output.native_resync_retired, retired);
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.network.get(client, Endpoint::Server).is_none());
    assert!(host.runtime.server.events.counters(output).is_none());
    assert!(host.runtime.server.entities.resolve(old).is_none());
    let retained = host
        .runtime
        .server
        .entities
        .find_native(native, None)
        .unwrap();
    assert_ne!(retained, old);
    let retained_body = host
        .runtime
        .server
        .entities
        .columns
        .body(retained.slot as usize);
    assert_eq!(
        (
            retained_body.position,
            retained_body.velocity,
            retained_body.mins,
            retained_body.maxs
        ),
        (body.position, body.velocity, body.mins, body.maxs)
    );
    assert_eq!(host.runtime.server.area.bounds(retained), Some(bounds));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
}
