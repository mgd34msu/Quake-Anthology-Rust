#[path = "support/quakec_program.rs"]
mod quakec_program;

use qa_compat::{
    abi::{QUAKEC, QuakeCCalls, UnknownCalls},
    quakec::{Builtins, Layout, Trap, Vm},
    services::{
        CallContext, CallError, ENGINE_CALLS, EngineServices, ModelLoader, ModelLookup,
        ServiceStorage, SignonCursor,
    },
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{names::NameTable, primitives::*, sys_events::EventTime};
use qa_formats::program::quakec::{Image, Opcode};
use qa_world::{
    collision::{CollisionStore, brushes::BrushTree, surfaces::SurfaceTable},
    entities::AllocationPolicy,
};

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
    (4, 36, b"groundentity"),
];
const GLOBALS: &[(u16, u16, &[u8])] = &[
    (4, 34, b"self"),
    (2, 35, b"time"),
    (2, 60, b"trace_allsolid"),
    (2, 61, b"trace_startsolid"),
    (2, 62, b"trace_fraction"),
    (2, 63, b"trace_inwater"),
    (2, 64, b"trace_inopen"),
    (2, 65, b"trace_plane_dist"),
    (3, 66, b"trace_endpos"),
    (3, 69, b"trace_plane_normal"),
    (4, 72, b"trace_ent"),
];
fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|value| value.0.map(f32::to_bits))
}
fn vm(capacity: usize) -> Vm {
    vm_with_definitions(capacity, GLOBALS, FIELDS)
}
fn vm_with_definitions(
    capacity: usize,
    globals: &[(u16, u16, &[u8])],
    fields: &[(u16, u16, &[u8])],
) -> Vm {
    let bytes = quakec_program::program_with_definitions(
        &[
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePV, 28, 31, 0),
            (Opcode::Call4, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePF, 28, 31, 0),
            (Opcode::Call4, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::State, 28, 29, 0),
            (Opcode::Call4, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePF, 28, 31, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePV, 28, 31, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::State, 28, 29, 0),
            (Opcode::Return, 1, 0, 0),
            (Opcode::StorePV, 28, 31, 0),
            (Opcode::Call1, 32, 0, 0),
            (Opcode::Return, 1, 0, 0),
        ],
        &[0, 1, 2, 5, 8, -16, 11, 13, 15, 17, -41],
        globals,
        fields,
        fields
            .iter()
            .map(|(kind, offset, _)| usize::from(*offset) + if *kind == 3 { 3 } else { 1 })
            .max()
            .unwrap_or(37)
            .max(37),
        96,
    );
    Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: capacity,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
        },
    )
    .unwrap()
}
fn parameters(vm: &mut Vm, parameters: &[&[u32]]) {
    for (index, words) in parameters.iter().enumerate() {
        vm.image.globals[4 + index * 3..4 + index * 3 + words.len()].copy_from_slice(words);
    }
}
fn write(vm: &mut Vm, value: u32, name: &[u8], words: &[u32]) {
    let field = vm.image.field(name).unwrap() as u32;
    let at = vm.field_address(value, field, words.len()).unwrap();
    for (index, word) in words.iter().enumerate() {
        vm.entities
            .write_word(at + index as u64 * 4, *word as i32)
            .unwrap();
    }
}
fn read(vm: &Vm, value: u32, name: &[u8], width: usize) -> Vec<u32> {
    let at = vm
        .field_address(value, vm.image.field(name).unwrap() as u32, width)
        .unwrap();
    (0..width)
        .map(|index| vm.entities.read_word(at + index as u64 * 4).unwrap() as u32)
        .collect()
}
fn store_and_trace(
    calls: &mut QuakeCCalls<'_, '_>,
    vm: &mut Vm,
    value: u32,
    name: &[u8],
    words: &[u32],
) -> Result<[u32; 3], Trap> {
    vm.image.globals[28..28 + words.len()].copy_from_slice(words);
    vm.image.globals[31] = vm
        .field_address(value, vm.image.field(name).unwrap() as u32, words.len())
        .unwrap() as u32;
    vm.image.globals[32] = 5;
    vm.call(calls, if words.len() == 3 { 2 } else { 3 }, 100, false)
}
fn entity(calls: &QuakeCCalls<'_, '_>, vm: &Vm, value: u32) -> EntityId {
    let native = NativeEntity {
        module: calls.context.module,
        slot: vm.entity_number(value).unwrap() as i32,
    };
    calls
        .services
        .server
        .entities
        .active()
        .find(|id| {
            calls.services.server.entities.columns.native_entity[id.slot as usize] == Some(native)
        })
        .unwrap()
}
fn with_services(rules: RuleSetId, run: impl FnOnce(&mut QuakeCCalls<'_, '_>)) {
    with_model_services(rules, None, run);
}
fn with_model_services(
    rules: RuleSetId,
    model_loader: Option<&mut Models>,
    run: impl FnOnce(&mut QuakeCCalls<'_, '_>),
) {
    with_collision_services(rules, model_loader, &[], run);
}
fn with_collision_services(
    rules: RuleSetId,
    mut model_loader: Option<&mut Models>,
    world_boxes: &[Bounds],
    run: impl FnOnce(&mut QuakeCCalls<'_, '_>),
) {
    let mut names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
    let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
    let mut server = qa_session::clients::Server::load(1, 32, 1, 1, 1, 1).unwrap();
    let mut cvars = Cvars::new().unwrap();
    let console = Context {
        source: rules,
        ..Context::default()
    };
    let max_clients = cvars.bind("sv_maxclients", console).unwrap();
    cvars.force_write(max_clients, "1").unwrap();
    let mut input = qa_input::Input::load();
    let mut commands = CommandBuffer::new();
    let vfs = qa_content::vfs::Vfs::default();
    let localization = qa_content::localization::Catalog::default();
    let config_count = qa_core::configstrings::ConfigLayout::for_rules(rules).count;
    let mut storage =
        ServiceStorage::load_native(&[(ModuleId(1), config_count, rules)], 0, &cvars).unwrap();
    let mut geometry = CollisionStore::new();
    let mut planes = Vec::new();
    let mut brushes = Vec::new();
    for bounds in world_boxes {
        let first = planes.len() as u32;
        for axis in 0..3 {
            let mut normal = [0.0; 3];
            normal[axis] = 1.0;
            planes.push(Plane::oriented(Vec3(normal), bounds.maxs.0[axis]));
            normal[axis] = -1.0;
            planes.push(Plane::oriented(Vec3(normal), -bounds.mins.0[axis]));
        }
        brushes.push(qa_world::collision::brushes::Brush {
            sides: IndexSpan { first, count: 6 },
            contents: qa_world::collision::Contents::SOLID,
        });
    }
    let mut tree = BrushTree::direct(brushes.len()).unwrap();
    tree.models
        .push(qa_world::collision::brushes::ModelRoot::Leaf(0));
    let world = geometry
        .load_brushes(
            planes,
            brushes,
            SurfaceTable::flags(vec![SurfaceFlags::default(); world_boxes.len() * 6]),
            tree,
            vec![
                Bounds {
                    mins: Vec3([-100.0; 3]),
                    maxs: Vec3([100.0; 3]),
                },
                Bounds {
                    mins: Vec3([-4.0, -8.0, -12.0]),
                    maxs: Vec3([8.0, 16.0, 24.0]),
                },
            ],
        )
        .unwrap();
    let mut scratch = geometry.scratch();
    let developer = cvars.find("developer").unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut services = EngineServices {
        server: &mut server,
        input: &mut input,
        cvars: &mut cvars,
        commands: &mut commands,
        client_commands: None,
        vfs: &vfs,
        model_loader: model_loader
            .as_deref_mut()
            .map(|loader| loader as &mut dyn ModelLoader),
        model_lookup: None,
        localization: &localization,
        registry: &registry,
        names: &mut names,
        storage: &mut storage,
        geometry: &geometry,
        world: Some((world, 0)),
        visibility: None,
        portals: None,
        scratch: &mut scratch,
    };
    let mut calls = QuakeCCalls {
        services: &mut services,
        table: &QUAKEC,
        context: CallContext {
            native_rules: rules,
            trace_rules: rules,
            sound_policy: qa_gameplay::rules::sound_policy(rules),
            module: ModuleId(1),
            clock: ThinkTime::Seconds(3.0),
            server_frame: 0,
            console,
            allocation: AllocationPolicy::for_rules(rules, 0),
            link_order: qa_gameplay::rules::link_order(rules),
        },
        platform_time: EventTime(0),
        developer,
        unknown: &mut unknown,
    };
    run(&mut calls);
}

#[derive(Default)]
struct Models {
    loaded: Vec<(Vec<u8>, RuleSetId)>,
    fail: bool,
}
impl ModelLookup for Models {
    fn bounds(&self, path: &[u8], source: RuleSetId) -> Option<Bounds> {
        (!self.fail
            && self
                .loaded
                .iter()
                .any(|(name, rules)| name == path && *rules == source))
        .then_some(Bounds {
            mins: Vec3([-16.0; 3]),
            maxs: Vec3([16.0; 3]),
        })
    }
}
impl ModelLoader for Models {
    fn precache(
        &mut self,
        _: &qa_content::vfs::Vfs,
        path: &[u8],
        source: RuleSetId,
    ) -> Result<Bounds, CallError> {
        self.loaded.push((path.to_vec(), source));
        if self.fail {
            Err(CallError::File)
        } else {
            self.bounds(path, source).ok_or(CallError::Geometry)
        }
    }
}

#[test]
fn cold_lightstyle_builtin_uses_shared_rows_raw_strings_and_native_index_conversion() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = vm(1);
            let layout = qa_core::configstrings::ConfigLayout::for_rules(rules);
            let return_words = [0xa55a_1234, 0x1122_3344, 0x5566_7788];
            vm.image.globals[1..4].copy_from_slice(&return_words);
            let text = vm.insert_string(b"aM\x80\xffz").unwrap() as u32;
            for (style, ordinal) in [(-0.5f32, 0usize), (31.9, 31), (63.9, 63)] {
                parameters(&mut vm, &[&[style.to_bits()], &[text]]);
                calls.call(&mut vm, 35, 2).unwrap();
                assert_eq!(vm.image.globals[1..4], return_words);
                assert_eq!(
                    calls
                        .services
                        .storage
                        .configstring(calls.context.module, layout.style_index(ordinal).unwrap())
                        .unwrap()
                        .0,
                    b"aM\x80\xffz",
                );
            }
            let index = layout.style_index(31).unwrap();
            let revision = calls
                .services
                .storage
                .configstring(calls.context.module, index)
                .unwrap()
                .1;
            for style in [-1.0f32, 64.0, f32::INFINITY, f32::NAN] {
                parameters(&mut vm, &[&[style.to_bits()], &[text]]);
                assert_eq!(calls.call(&mut vm, 35, 2), Err(Trap::Builtin));
                assert_eq!(
                    calls
                        .services
                        .storage
                        .configstring(calls.context.module, index)
                        .unwrap(),
                    (b"aM\x80\xffz".as_slice(), revision)
                );
            }
            let long = vm.insert_string(&[b'm'; 64]).unwrap() as u32;
            parameters(&mut vm, &[&[31.0f32.to_bits()], &[long]]);
            assert_eq!(calls.call(&mut vm, 35, 2), Err(Trap::Builtin));
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, index)
                    .unwrap(),
                (b"aM\x80\xffz".as_slice(), revision)
            );
            assert_eq!(calls.call(&mut vm, 35, 1), Err(Trap::Builtin));
            parameters(&mut vm, &[&[31.0f32.to_bits()], &[0]]);
            calls.call(&mut vm, 35, 2).unwrap();
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, index)
                    .unwrap()
                    .0,
                b""
            );
            assert!(calls.services.server.events.is_empty());
            assert_eq!(calls.unknown.calls, 0);
            assert_eq!(
                calls.services.storage.resources(calls.context.module),
                Some(layout.resources)
            );
        });
    }
}

#[test]
fn precaches_preserve_words_exact_names_and_native_loading_rules() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut models = Models::default();
        with_model_services(rules, Some(&mut models), |calls| {
            let mut vm = vm(4);
            let first = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
            let second = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
            let capital = vm.insert_string(b"progs/Test.mdl").unwrap() as u32;
            assert_ne!(first, second);
            for (builtin, word) in [(20, first), (75, second), (20, capital)] {
                parameters(&mut vm, &[&[word]]);
                calls.call(&mut vm, builtin, 1).unwrap();
                assert_eq!(vm.image.globals[1], word);
            }
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, 1)
                    .unwrap()
                    .0,
                b"progs/test.mdl"
            );
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, 2)
                    .unwrap()
                    .0,
                b"progs/Test.mdl"
            );
            let sound = vm.insert_string(b"misc/ready.wav").unwrap() as u32;
            parameters(&mut vm, &[&[sound]]);
            calls.call(&mut vm, 19, 1).unwrap();
            calls.call(&mut vm, 76, 1).unwrap();
            assert_eq!(vm.image.globals[1], sound);
            assert_eq!(
                calls
                    .services
                    .storage
                    .configstring(calls.context.module, 257)
                    .unwrap()
                    .0,
                b"misc/ready.wav"
            );
            vm.active = true;
            for builtin in [19, 20, 75, 76] {
                assert_eq!(calls.call(&mut vm, builtin, 1), Err(Trap::Builtin));
            }
            for builtin in [68, 77] {
                calls.call(&mut vm, builtin, 1).unwrap();
                assert_eq!(vm.image.globals[1], sound);
            }
        });
        assert_eq!(
            models.loaded.len(),
            if rules == RuleSetId::Quake { 2 } else { 0 }
        );
        assert!(models.loaded.iter().all(|(_, source)| *source == rules));
    }
}

#[test]
fn precache_empty_overflow_and_failed_native_load_have_original_side_effects() {
    let mut models = Models {
        fail: true,
        ..Models::default()
    };
    with_model_services(RuleSetId::Quake, Some(&mut models), |calls| {
        let mut vm = vm(4);
        let name = vm.insert_string(b"progs/missing.mdl").unwrap() as u32;
        parameters(&mut vm, &[&[name]]);
        assert_eq!(calls.call(&mut vm, 20, 1), Err(Trap::Builtin));
        assert_eq!(
            calls
                .services
                .storage
                .configstring(calls.context.module, 1)
                .unwrap()
                .0,
            b"progs/missing.mdl"
        );
        // Native registration already happened; repeat registration does not
        // retry Mod_ForName or replace the failed module's ordinal.
        calls.call(&mut vm, 20, 1).unwrap();
        for name in [b"".as_slice(), b" space", b"\tpath"] {
            let word = vm.insert_string(name).unwrap() as u32;
            parameters(&mut vm, &[&[word]]);
            assert_eq!(calls.call(&mut vm, 19, 1), Err(Trap::Builtin));
        }
        let sounds = calls
            .services
            .storage
            .resources(calls.context.module)
            .unwrap()[1];
        for ordinal in 1..256 {
            let path = format!("s{ordinal}.wav");
            assert_eq!(
                (ENGINE_CALLS.resource_index)(
                    calls.services,
                    calls.context.module,
                    sounds,
                    path.as_bytes()
                )
                .unwrap(),
                ordinal
            );
        }
        let overflow = vm.insert_string(b"extra.wav").unwrap() as u32;
        parameters(&mut vm, &[&[overflow]]);
        assert_eq!(calls.call(&mut vm, 19, 1), Err(Trap::Builtin));
    });
    assert_eq!(models.loaded.len(), 1);
}

#[test]
fn setmodel_only_uses_precached_ordinals_and_native_bounds_policy() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut models = Models::default();
        with_model_services(rules, Some(&mut models), |calls| {
            let mut vm = seeded_vm(calls, 4);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            let id = entity(calls, &vm, value);
            let initial = Bounds {
                mins: Vec3([-2.0; 3]),
                maxs: Vec3([3.0; 3]),
            };
            write(&mut vm, value, b"mins", &initial.mins.0.map(f32::to_bits));
            write(&mut vm, value, b"maxs", &initial.maxs.0.map(f32::to_bits));
            let name = vm.insert_string(b"progs/test.mdl").unwrap() as u32;
            parameters(&mut vm, &[&[value], &[name]]);
            assert_eq!(calls.call(&mut vm, 3, 2), Err(Trap::Builtin));
            assert_eq!(read(&vm, value, b"modelindex", 1), [0]);
            parameters(&mut vm, &[&[name]]);
            calls.call(&mut vm, 20, 1).unwrap();
            parameters(&mut vm, &[&[value], &[name]]);
            calls.call(&mut vm, 3, 2).unwrap();
            assert_eq!(read(&vm, value, b"model", 1), [name]);
            assert_eq!(read(&vm, value, b"modelindex", 1), [1.0f32.to_bits()]);
            let expected = if rules == RuleSetId::Quake {
                Vec3([-16.0; 3])
            } else {
                initial.mins
            };
            assert_eq!(read(&vm, value, b"mins", 3), expected.0.map(f32::to_bits));
            assert_eq!(
                calls.services.server.entities.columns.appearance[id.slot as usize].models[0],
                ModelRef::native(ResourceNamespace::Module(calls.context.module), 1)
            );
            assert_eq!(
                calls.services.server.area.bounds(id).is_some(),
                rules == RuleSetId::Quake
            );
            parameters(&mut vm, &[&[value], &[0]]);
            calls.call(&mut vm, 3, 2).unwrap();
            assert_eq!(
                read(&vm, value, b"mins", 3),
                if rules == RuleSetId::Quake {
                    [0; 3]
                } else {
                    initial.mins.0.map(f32::to_bits)
                }
            );
            let inline = vm.insert_string(b"*1").unwrap() as u32;
            parameters(&mut vm, &[&[inline]]);
            calls.call(&mut vm, 20, 1).unwrap();
            parameters(&mut vm, &[&[value], &[inline]]);
            calls.call(&mut vm, 3, 2).unwrap();
            assert_eq!(
                read(&vm, value, b"mins", 3),
                [-4.0f32, -8.0, -12.0].map(f32::to_bits)
            );
            assert_eq!(
                read(&vm, value, b"size", 3),
                [12.0f32, 24.0, 36.0].map(f32::to_bits)
            );
            assert!(calls.services.server.area.bounds(id).is_some());
        });
    }
}

#[test]
fn qc_sound_uses_registered_bank_ring_native_origin_and_independent_sound_rules() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 4);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            let id = entity(calls, &vm, value);
            write(
                &mut vm,
                value,
                b"origin",
                &[11.0f32, 22.0, 33.0].map(f32::to_bits),
            );
            write(
                &mut vm,
                value,
                b"mins",
                &[-2.0f32, -4.0, -6.0].map(f32::to_bits),
            );
            write(
                &mut vm,
                value,
                b"maxs",
                &[6.0f32, 12.0, 18.0].map(f32::to_bits),
            );
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            let name = vm.insert_string(b"misc/test.wav").unwrap() as u32;
            parameters(&mut vm, &[&[name]]);
            calls.call(&mut vm, 19, 1).unwrap();
            let rate = std::num::NonZeroU32::new(44100).unwrap();
            let bank = std::sync::Arc::new(
                qa_audio::Bank::load(
                    rate,
                    vec![(
                        "sound/misc/test.wav".into(),
                        Pcm {
                            rate,
                            channels: PcmChannels::Mono,
                            samples: vec![1000; 4],
                            loop_start: None,
                        },
                    )],
                )
                .unwrap(),
            );
            let sound = bank.find("sound/misc/test.wav").unwrap();
            let sounds = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[1];
            calls
                .services
                .storage
                .bind_sound_bank(calls.context.module, sounds, bank)
                .unwrap();
            calls.context.sound_policy = qa_gameplay::rules::sound_policy(RuleSetId::Quake3);
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Remote,
                    ModuleId::default(),
                    None,
                )
                .unwrap();
            let consumer = calls.services.server.clients[client.0 as usize]
                .output
                .unwrap();
            let channel = if rules == RuleSetId::QuakeWorld {
                9.0f32
            } else {
                1.0f32
            };
            parameters(
                &mut vm,
                &[
                    &[value],
                    &[channel.to_bits()],
                    &[name],
                    &[0.501f32.to_bits()],
                    &[1.0f32.to_bits()],
                ],
            );
            calls.call(&mut vm, 8, 5).unwrap();
            let mut batch = calls.services.server.events.batch(consumer).unwrap();
            let record = calls.services.server.events.next(&mut batch).unwrap();
            let qa_core::events::FrameEvent::Sound(event) = record.event else {
                panic!("sound record")
            };
            assert_eq!(event.sound, sound);
            assert_eq!(event.entity, Some(SoundEmitter::Entity(id)));
            assert_eq!(
                event.spatial,
                qa_gameplay::rules::sound_policy(RuleSetId::Quake3)
            );
            assert_eq!(
                event.position,
                if rules == RuleSetId::Quake {
                    Vec3([13.0, 26.0, 39.0])
                } else {
                    Vec3([11.0, 22.0, 33.0])
                }
            );
            assert_eq!(event.volume.to_bits(), (127.0f32 / 255.0).to_bits());
            assert_eq!(event.channel, 1);
            let delivery = record.delivery.unwrap();
            assert_eq!(delivery.source, rules);
            assert_eq!(delivery.target, NativeTarget::All);
            assert_eq!(delivery.reliable, rules == RuleSetId::QuakeWorld);
            assert!(calls.services.server.area.bounds(id).is_none());
            if rules == RuleSetId::QuakeWorld {
                let phs = calls
                    .services
                    .cvars
                    .bind("sv_phs", calls.context.console)
                    .unwrap();
                for (enabled, target) in [("1", NativeTarget::Hearing), ("0", NativeTarget::All)] {
                    calls.services.cvars.force_write(phs, enabled).unwrap();
                    parameters(
                        &mut vm,
                        &[
                            &[value],
                            &[1.0f32.to_bits()],
                            &[name],
                            &[1.0f32.to_bits()],
                            &[1.0f32.to_bits()],
                        ],
                    );
                    calls.call(&mut vm, 8, 5).unwrap();
                    let mut batch = calls.services.server.events.batch(consumer).unwrap();
                    let records =
                        std::iter::from_fn(|| calls.services.server.events.next(&mut batch))
                            .collect::<Vec<_>>();
                    let delivery = records.last().unwrap().delivery.unwrap();
                    assert_eq!(delivery.target, target);
                    assert!(!delivery.reliable);
                }
            }
        });
    }
}
fn seeded_vm(calls: &mut QuakeCCalls<'_, '_>, capacity: usize) -> Vm {
    let vm = vm(if calls.context.native_rules == RuleSetId::QuakeWorld {
        capacity + 31
    } else {
        capacity
    });
    seed_vm(calls, vm)
}
fn seed_vm(calls: &mut QuakeCCalls<'_, '_>, mut vm: Vm) -> Vm {
    let anchor = calls
        .services
        .server
        .entities
        .allocate(
            calls.context.clock,
            calls.context.module,
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    calls.services.server.entities.columns.native_entity[anchor.slot as usize] =
        Some(NativeEntity {
            module: calls.context.module,
            slot: 0,
        });
    vm.seed_entities(
        &calls.services.server.entities,
        calls.context.module,
        calls.context.native_rules,
        calls
            .services
            .storage
            .max_clients(calls.services.cvars, calls.context.native_rules),
    )
    .unwrap();
    vm
}

fn named_vm(calls: &mut QuakeCCalls<'_, '_>, defined: &[&[u8]]) -> Vm {
    let mut fields = FIELDS.to_vec();
    fields.extend(
        [b"classname".as_slice(), b"target", b"targetname"]
            .into_iter()
            .enumerate()
            .filter(|(_, name)| defined.contains(name))
            .map(|(index, name)| (1, 37 + index as u16, name)),
    );
    let bytes = quakec_program::program_with_definitions(
        &[(Opcode::StorePS, 28, 31, 0), (Opcode::Return, 1, 0, 0)],
        &[0, 1],
        GLOBALS,
        &fields,
        40,
        96,
    );
    let vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: if calls.context.native_rules == RuleSetId::QuakeWorld {
                36
            } else {
                5
            },
            header_bytes: 16,
            extra_string_bytes: 1024,
            state_step: 0.1,
        },
    )
    .unwrap();
    let (names, bytes) = vm.name_history_reservation().unwrap();
    calls.services.names.reserve(names, bytes).unwrap();
    seed_vm(calls, vm)
}

#[test]
fn native_names_publish_exact_raw_bytes_and_dirty_target_changes_to_one_index() {
    use qa_core::names::NameMatch;
    use qa_world::targets::TargetIndex;
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            calls.context.console.source = RuleSetId::Quake2Rerelease;
            calls.context.trace_rules = RuleSetId::Quake3;
            let mut vm = named_vm(calls, &[b"classname", b"target", b"targetname"]);
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let class = vm.insert_string(b"Monster\xff").unwrap() as u32;
            let target = vm.insert_string(b"door\\path\xfe").unwrap() as u32;
            let name = vm.insert_string(b"Door\xff").unwrap() as u32;
            for (field, word) in [
                (b"classname".as_slice(), class),
                (b"target", target),
                (b"targetname", name),
            ] {
                write(&mut vm, value, field, &[word]);
            }
            let capacity = (
                calls.services.names.capacity(),
                calls.services.names.byte_capacity(),
            );
            vm.project_entities(calls.services, calls.context).unwrap();
            let slot = id.slot as usize;
            let class_id = calls.services.names.find(b"Monster\xff").unwrap();
            let target_id = calls.services.names.find(b"door\\path\xfe").unwrap();
            let upper = calls.services.names.find(b"Door\xff").unwrap();
            assert_eq!(
                calls.services.server.entities.columns.classname[slot],
                Some(class_id)
            );
            assert_eq!(
                calls.services.server.entities.columns.target[slot],
                Some(target_id)
            );
            assert_eq!(
                calls.services.server.entities.columns.targetname(slot),
                Some(upper)
            );
            assert_eq!(
                calls.services.names.get(target_id),
                Some(b"door\\path\xfe".as_slice())
            );
            let mut targets = TargetIndex::new(&calls.services.server.entities);
            assert!(targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert_eq!(
                targets
                    .find(upper, NameMatch::Exact, calls.services.names)
                    .collect::<Vec<_>>(),
                [id]
            );

            let lower_word = vm.insert_string(b"door\xff").unwrap() as u32;
            vm.image.globals[28] = lower_word;
            vm.image.globals[31] = vm
                .field_address(value, vm.image.field(b"targetname").unwrap() as u32, 1)
                .unwrap() as u32;
            vm.call(calls, 1, 100, false).unwrap();
            // StorePS writes the VM row; only its existing publication imports it.
            assert_eq!(
                calls.services.server.entities.columns.targetname(slot),
                Some(upper)
            );
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            let lower = calls.services.names.find(b"door\xff").unwrap();
            assert_ne!(upper, lower);
            assert!(targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert!(
                targets
                    .find(upper, NameMatch::Exact, calls.services.names)
                    .next()
                    .is_none()
            );
            assert_eq!(
                targets
                    .find(lower, NameMatch::Exact, calls.services.names)
                    .collect::<Vec<_>>(),
                [id]
            );
            assert_eq!(
                targets
                    .find(upper, NameMatch::Folded, calls.services.names)
                    .collect::<Vec<_>>(),
                [id]
            );
            vm.project_entities(calls.services, calls.context).unwrap();
            assert!(!targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert_eq!(
                (
                    calls.services.names.capacity(),
                    calls.services.names.byte_capacity()
                ),
                capacity
            );

            vm.remove_entity(calls.services, calls.context, value)
                .unwrap();
            assert!(targets.refresh(&mut calls.services.server.entities, calls.services.names));
            assert!(
                targets
                    .find(lower, NameMatch::Exact, calls.services.names)
                    .next()
                    .is_none()
            );
            calls.context.clock = ThinkTime::Seconds(4.0);
            let reused = vm.spawn_entity(calls.services, calls.context).unwrap();
            assert_eq!(reused, value);
            let next = entity(calls, &vm, reused);
            assert_ne!(next, id);
            let slot = next.slot as usize;
            assert_eq!(calls.services.server.entities.columns.classname[slot], None);
            assert_eq!(calls.services.server.entities.columns.target[slot], None);
            assert_eq!(
                calls.services.server.entities.columns.targetname(slot),
                None
            );
        });
    }
}

#[test]
fn native_name_definitions_preserve_absence_and_explicit_empty_offsets() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for present in [false, true] {
            with_services(rules, |calls| {
                let fields: &[&[u8]] = if present {
                    &[b"classname", b"target", b"targetname"]
                } else {
                    &[]
                };
                let mut vm = named_vm(calls, fields);
                let value = vm.spawn_entity(calls.services, calls.context).unwrap();
                let id = entity(calls, &vm, value);
                vm.project_entities(calls.services, calls.context).unwrap();
                let slot = id.slot as usize;
                let expected = present.then_some(NameId(0));
                assert_eq!(
                    calls.services.server.entities.columns.classname[slot],
                    expected
                );
                assert_eq!(
                    calls.services.server.entities.columns.target[slot],
                    expected
                );
                assert_eq!(
                    calls.services.server.entities.columns.targetname(slot),
                    expected
                );
                if present {
                    let empty = vm.insert_string(b"").unwrap() as u32;
                    assert_ne!(empty, 0);
                    for field in [b"classname".as_slice(), b"target", b"targetname"] {
                        write(&mut vm, value, field, &[empty]);
                    }
                    vm.project_entities(calls.services, calls.context).unwrap();
                    assert_eq!(
                        calls.services.server.entities.columns.classname[slot],
                        expected
                    );
                    assert_eq!(
                        calls.services.server.entities.columns.target[slot],
                        expected
                    );
                    assert_eq!(
                        calls.services.server.entities.columns.targetname(slot),
                        expected
                    );
                }
            });
        }
    }
}

#[test]
fn rejected_native_name_import_preserves_prior_common_names_and_skips_retired_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = named_vm(calls, &[b"classname", b"target", b"targetname"]);
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        let name = vm.insert_string(b"live").unwrap() as u32;
        for field in [b"classname".as_slice(), b"target", b"targetname"] {
            write(&mut vm, value, field, &[name]);
        }
        vm.project_entities(calls.services, calls.context).unwrap();
        let slot = id.slot as usize;
        let before = (
            calls.services.server.entities.columns.classname[slot],
            calls.services.server.entities.columns.target[slot],
            calls.services.server.entities.columns.targetname(slot),
        );
        for bad in [u32::MAX, vm.strings.len() as u32] {
            write(&mut vm, value, b"targetname", &[bad]);
            assert_eq!(
                vm.project_entities(calls.services, calls.context),
                Err(CallError::Memory)
            );
            assert_eq!(
                (
                    calls.services.server.entities.columns.classname[slot],
                    calls.services.server.entities.columns.target[slot],
                    calls.services.server.entities.columns.targetname(slot)
                ),
                before
            );
        }
        let unterminated = vm.strings.len() - 1;
        vm.strings.write(unterminated as u64, &[0xff]).unwrap();
        write(&mut vm, value, b"targetname", &[unterminated as u32]);
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Memory)
        );
        assert_eq!(
            calls.services.server.entities.columns.targetname(slot),
            before.2
        );
        write(&mut vm, value, b"targetname", &[name]);
        let next = vm.insert_string(b"not-yet-published").unwrap() as u32;
        let mut counter = 0;
        loop {
            let text = format!("history-{counter}");
            if calls.services.names.intern(text.as_bytes()).is_err() {
                break;
            }
            counter += 1;
        }
        write(&mut vm, value, b"targetname", &[next]);
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Capacity)
        );
        assert_eq!(
            (
                calls.services.server.entities.columns.classname[slot],
                calls.services.server.entities.columns.target[slot],
                calls.services.server.entities.columns.targetname(slot)
            ),
            before
        );
        assert_eq!(
            calls.services.names.get(before.0.unwrap()),
            Some(b"live".as_slice())
        );
        // ED_Free need not inspect target strings; the dead ordinal cannot
        // cause a later full-export scan to read malformed name offsets.
        write(&mut vm, value, b"targetname", &[u32::MAX]);
        vm.remove_entity(calls.services, calls.context, value)
            .unwrap();
        vm.project_entities(calls.services, calls.context).unwrap();
    });
}

#[test]
fn two_native_modules_share_exact_names_without_sharing_native_string_offsets() {
    with_services(RuleSetId::Quake, |calls| {
        let mut first = named_vm(calls, &[b"classname", b"target", b"targetname"]);
        let a = first.spawn_entity(calls.services, calls.context).unwrap();
        let a_id = entity(calls, &first, a);
        let a_word = first.insert_string(b"shared\xff").unwrap() as u32;
        write(&mut first, a, b"classname", &[a_word]);
        first
            .project_entities(calls.services, calls.context)
            .unwrap();
        let shared = calls.services.server.entities.columns.classname[a_id.slot as usize].unwrap();
        let before = calls.services.names.get(shared).unwrap().to_vec();

        calls.context.module = ModuleId(2);
        let mut second = named_vm(calls, &[b"classname", b"target", b"targetname"]);
        let b = second.spawn_entity(calls.services, calls.context).unwrap();
        let b_id = entity(calls, &second, b);
        second
            .insert_string(b"padding changes the native offset")
            .unwrap();
        let b_word = second.insert_string(b"shared\xff").unwrap() as u32;
        assert_ne!(a_word, b_word);
        write(&mut second, b, b"classname", &[b_word]);
        let different = second.insert_string(b"SHARED\xff").unwrap() as u32;
        write(&mut second, b, b"target", &[different]);
        second
            .project_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.classname[b_id.slot as usize],
            Some(shared)
        );
        let variant = calls.services.server.entities.columns.target[b_id.slot as usize].unwrap();
        assert_ne!(variant, shared);
        assert_eq!(
            calls.services.names.folded(variant),
            calls.services.names.folded(shared)
        );
        assert_eq!(calls.services.names.get(shared), Some(before.as_slice()));
        assert_eq!(read(&first, a, b"classname", 1), [a_word]);
        assert_eq!(read(&second, b, b"classname", 1), [b_word]);
    });
}

fn find(
    calls: &mut QuakeCCalls<'_, '_>,
    vm: &mut Vm,
    start: u32,
    field: u32,
    text: i32,
) -> Result<u32, Trap> {
    parameters(vm, &[&[start], &[field], &[text as u32]]);
    calls.call(vm, 18, 3)?;
    Ok(vm.image.globals[1])
}

#[test]
fn find_includes_frozen_native_clients_without_common_bindings() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let max_clients = calls
                .services
                .cvars
                .bind("sv_maxclients", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(max_clients, "3").unwrap();
            let mut vm = seeded_vm(calls, 8);
            let clients = if rules == RuleSetId::QuakeWorld {
                32
            } else {
                3
            };
            let stride = vm.stride as u32;
            let field = vm.image.field(b"model").unwrap() as u32;
            let live = calls.services.server.entities.len();
            let relinks = calls.services.server.area.relinks;
            for changed in ["0", "63"] {
                calls
                    .services
                    .cvars
                    .force_write(max_clients, changed)
                    .unwrap();
                for ordinal in 0..clients {
                    assert_eq!(
                        find(calls, &mut vm, ordinal * stride, field, 0),
                        Ok((ordinal + 1) * stride)
                    );
                }
                assert_eq!(find(calls, &mut vm, clients * stride, field, 0), Ok(0));
            }
            let mut cursor = 1;
            assert!(
                vm.physics_entity(&calls.services.server.entities, &mut cursor)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(calls.services.server.entities.len(), live);
            assert_eq!(calls.services.server.area.relinks, relinks);
        });
    }
}

#[test]
fn find_uses_native_order_exact_bytes_and_raw_field_words() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let first = if rules == RuleSetId::QuakeWorld {
                33
            } else {
                2
            };
            let mut mapped = Vec::new();
            for ordinal in [first + 2, first, first + 1] {
                let id = calls
                    .services
                    .server
                    .entities
                    .allocate(
                        calls.context.clock,
                        calls.context.module,
                        AllocationPolicy::EDICT,
                    )
                    .unwrap()
                    .id;
                calls.services.server.entities.columns.native_entity[id.slot as usize] =
                    Some(NativeEntity {
                        module: calls.context.module,
                        slot: ordinal,
                    });
                mapped.push(id);
            }
            let mut vm = seeded_vm(calls, 8);
            let stride = vm.stride as u32;
            let stored = vm.insert_string(b"Door\x80").unwrap();
            let matching = vm.insert_string(b"Door\x80").unwrap();
            let folded = vm.insert_string(b"door\x80").unwrap();
            assert_ne!(stored, matching);
            // The declared float field is deliberately used as the original
            // raw string-word operand, without a second field-type gate.
            let field = vm.image.field(b"skin").unwrap() as u32;
            for ordinal in first..first + 3 {
                write(&mut vm, ordinal as u32 * stride, b"skin", &[stored as u32]);
            }
            assert!(mapped[0].slot < mapped[1].slot);
            assert_eq!(
                find(calls, &mut vm, 0, field, matching),
                Ok(first as u32 * stride)
            );
            assert_eq!(
                find(calls, &mut vm, first as u32 * stride, field, matching),
                Ok((first + 1) as u32 * stride)
            );
            assert_eq!(
                find(calls, &mut vm, (first + 2) as u32 * stride, field, matching),
                Ok(0)
            );
            assert_eq!(find(calls, &mut vm, 0, field, folded), Ok(0));
        });
    }
}

#[test]
fn find_reads_live_vm_string_stores_without_entity_publication() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let native = vm.image.globals[1];
        let id = entity(calls, &vm, native);
        let field = vm.image.field(b"model").unwrap() as u32;
        let text = vm.insert_string(b"live\x80").unwrap();
        assert_eq!(find(calls, &mut vm, 0, field, text), Ok(0));
        vm.image.statements[11].opcode = Opcode::StorePS;
        vm.image.globals[28] = text as u32;
        vm.image.globals[31] = vm.field_address(native, field, 1).unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        let relinks = calls.services.server.area.relinks;
        let before = body_bits(
            calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize),
        );
        parameters(&mut vm, &[&[0], &[field], &[text as u32]]);
        let mut expected_globals = vm.image.globals.to_vec();
        expected_globals[1] = native;
        calls.call(&mut vm, 18, 3).unwrap();
        assert_eq!(&*vm.image.globals, expected_globals);
        assert_eq!(
            body_bits(
                calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize)
            ),
            before
        );
        assert_eq!(calls.services.server.area.relinks, relinks);
    });
}

#[test]
fn find_skips_freed_generations_and_never_uses_a_foreign_replacement() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            calls.context.clock = ThinkTime::Seconds(1.0);
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let first = vm.image.globals[1];
            let old = entity(calls, &vm, first);
            calls.call(&mut vm, 14, 0).unwrap();
            let second = vm.image.globals[1];
            let field = vm.image.field(b"model").unwrap() as u32;
            let text = vm.insert_string(b"match").unwrap();
            write(&mut vm, first, b"model", &[text as u32]);
            write(&mut vm, second, b"model", &[text as u32]);
            parameters(&mut vm, &[&[first]]);
            calls.call(&mut vm, 15, 1).unwrap();
            // Stale native bytes must not be read after canonical release.
            write(&mut vm, first, b"model", &[u32::MAX]);
            assert_eq!(find(calls, &mut vm, first, field, text), Ok(second));
            assert_eq!(find(calls, &mut vm, 0, field, text), Ok(second));
            let foreign = calls
                .services
                .server
                .entities
                .allocate(calls.context.clock, ModuleId(9), AllocationPolicy::EDICT)
                .unwrap()
                .id;
            assert_eq!(foreign.slot, old.slot);
            calls.services.server.entities.columns.native_entity[foreign.slot as usize] =
                Some(NativeEntity {
                    module: ModuleId(9),
                    slot: vm.entity_number(first).unwrap() as i32,
                });
            write(&mut vm, first, b"model", &[text as u32]);
            assert_eq!(find(calls, &mut vm, 0, field, text), Ok(second));
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.image.globals[1], first);
            let current = entity(calls, &vm, first);
            assert_ne!(current, old);
            assert!(calls.services.server.entities.resolve(old).is_none());
            assert!(calls.services.server.entities.resolve(foreign).is_some());
            write(&mut vm, first, b"model", &[text as u32]);
            assert_eq!(find(calls, &mut vm, 0, field, text), Ok(first));
        });
    }
}

#[test]
fn find_validates_only_executed_native_fields_and_strings() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        calls.call(&mut vm, 14, 0).unwrap();
        let last = vm.image.globals[1];
        let field = vm.image.field(b"model").unwrap() as u32;
        let text = vm.insert_string(b"first").unwrap();
        write(&mut vm, first, b"model", &[text as u32]);
        write(&mut vm, last, b"model", &[u32::MAX]);
        assert_eq!(find(calls, &mut vm, 0, field, text), Ok(first));
        assert_eq!(find(calls, &mut vm, first, field, text), Err(Trap::Builtin));
        assert_eq!(find(calls, &mut vm, last, u32::MAX, text), Ok(0));
        for invalid in [
            1,
            last + vm.stride as u32,
            vm.entities.len() as u32,
            u32::MAX,
        ] {
            assert_eq!(
                find(calls, &mut vm, invalid, field, text),
                Err(Trap::Builtin)
            );
        }
        for invalid in [vm.image.entityfields as u32, u32::MAX] {
            assert_eq!(find(calls, &mut vm, 0, invalid, text), Err(Trap::Builtin));
        }
        assert_eq!(find(calls, &mut vm, last, field, -1), Err(Trap::String));
        assert_eq!(calls.call(&mut vm, 18, 2), Err(Trap::Builtin));
        let mut unseeded = vm_with_definitions(8, GLOBALS, FIELDS);
        assert_eq!(find(calls, &mut unseeded, 0, field, 0), Err(Trap::Builtin));
    });
}

#[test]
fn spawn_remove_use_native_ordinals_and_independent_common_generations() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let first = vm.image.globals[1];
            let first_ordinal = if rules == RuleSetId::QuakeWorld {
                33
            } else {
                2
            };
            assert_eq!(vm.entity_number(first), Ok(first_ordinal));
            let old = entity(calls, &vm, first);
            assert_ne!(old.slot, first_ordinal);
            write(&mut vm, first, b"skin", &[19.0f32.to_bits()]);
            parameters(&mut vm, &[&[first]]);
            calls.call(&mut vm, 15, 1).unwrap();
            assert!(calls.services.server.entities.resolve(old).is_none());
            assert_eq!(read(&vm, first, b"nextthink", 1), vec![(-1.0f32).to_bits()]);
            assert_eq!(read(&vm, first, b"skin", 1), vec![0]);
            calls.context.clock = ThinkTime::Seconds(3.5);
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(first_ordinal + 1));
            calls.context.clock = ThinkTime::Seconds(3.500_001);
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.image.globals[1], first);
            let new = entity(calls, &vm, first);
            assert_ne!(new, old);
            assert_eq!(read(&vm, first, b"nextthink", 1), vec![0]);
            assert_eq!(read(&vm, first, b"skin", 1), vec![0]);
            assert_eq!(
                calls.services.server.entities.columns.owner[new.slot as usize],
                calls.context.module
            );
        });
    }
}

#[test]
fn quakeworld_reserves_all_32_native_clients_despite_maxclients_one() {
    with_services(RuleSetId::QuakeWorld, |calls| {
        let client = calls
            .services
            .server
            .entities
            .allocate(
                calls.context.clock,
                calls.context.module,
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        calls.services.server.entities.columns.native_entity[client.slot as usize] =
            Some(NativeEntity {
                module: calls.context.module,
                slot: 32,
            });
        assert!(calls.services.server.entities.set_never_free(client, true));
        let mut vm = seeded_vm(calls, 4);
        let client_value = 32 * vm.stride as u32;
        write(&mut vm, client_value, b"skin", &[19.0f32.to_bits()]);
        let before = vm
            .entities
            .read(u64::from(client_value), vm.stride)
            .unwrap()
            .to_vec();
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(33));
        assert!(calls.services.server.entities.resolve(client).is_some());
        assert_eq!(
            vm.entities
                .read(u64::from(client_value), vm.stride)
                .unwrap(),
            before
        );
        parameters(&mut vm, &[&[client_value]]);
        assert_eq!(calls.call(&mut vm, 15, 1), Err(Trap::Builtin));
        assert!(calls.services.server.entities.resolve(client).is_some());
        assert_eq!(
            vm.entities
                .read(u64::from(client_value), vm.stride)
                .unwrap(),
            before
        );
    });
}

#[test]
fn native_client_reservations_are_frozen_before_spawn_and_reuse() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let max_clients = calls
                .services
                .cvars
                .bind("sv_maxclients", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(max_clients, "3").unwrap();
            let reserved = if rules == RuleSetId::Quake { 3 } else { 32 };
            let client = calls
                .services
                .server
                .entities
                .allocate(
                    calls.context.clock,
                    calls.context.module,
                    AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            calls.services.server.entities.columns.native_entity[client.slot as usize] =
                Some(NativeEntity {
                    module: calls.context.module,
                    slot: reserved,
                });
            let mut vm = seeded_vm(calls, 8);
            let client_value = reserved as u32 * vm.stride as u32;
            write(&mut vm, client_value, b"skin", &[19.0f32.to_bits()]);
            let client_bytes = vm
                .entities
                .read(u64::from(client_value), vm.stride)
                .unwrap()
                .to_vec();
            for (value, ordinal) in [("0", reserved + 1), ("7", reserved + 2)] {
                calls
                    .services
                    .cvars
                    .force_write(max_clients, value)
                    .unwrap();
                calls.call(&mut vm, 14, 0).unwrap();
                assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(ordinal as u32));
            }
            let freed_value = vm.image.globals[1];
            let old = entity(calls, &vm, freed_value);
            parameters(&mut vm, &[&[freed_value]]);
            calls.call(&mut vm, 15, 1).unwrap();
            assert!(calls.services.server.entities.resolve(old).is_none());
            calls.services.cvars.force_write(max_clients, "0").unwrap();
            calls.context.clock = ThinkTime::Seconds(3.500_001);
            calls.call(&mut vm, 14, 0).unwrap();
            assert_eq!(vm.image.globals[1], freed_value);
            let current = entity(calls, &vm, freed_value);
            assert_ne!(current, old);
            assert_eq!(
                calls.services.server.entities.columns.native_entity[current.slot as usize],
                Some(NativeEntity {
                    module: calls.context.module,
                    slot: reserved + 2,
                })
            );
            assert!(calls.services.server.entities.resolve(client).is_some());
            assert_eq!(
                vm.entities
                    .read(u64::from(client_value), vm.stride)
                    .unwrap(),
                client_bytes
            );
            parameters(&mut vm, &[&[client_value]]);
            assert_eq!(calls.call(&mut vm, 15, 1), Err(Trap::Builtin));
            assert!(calls.services.server.entities.resolve(client).is_some());
        });
    }
}

#[test]
fn changed_client_reservation_reseed_is_rejected_without_reconfiguration() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        assert_eq!(
            vm.seed_entities(
                &calls.services.server.entities,
                calls.context.module,
                RuleSetId::Quake,
                4,
            ),
            Err(CallError::Entity)
        );
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.entity_number(vm.image.globals[1]), Ok(2));
    });
}

#[test]
fn rejected_protected_remove_preserves_lifetime_area_and_native_bytes() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            let id = entity(calls, &vm, value);
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            let mins = [-2.0f32; 3].map(f32::to_bits);
            let maxs = [2.0f32; 3].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &mins, &maxs]);
            calls.call(&mut vm, 4, 3).unwrap();
            let origin = [10.0f32, 0.0, 0.0].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &origin]);
            calls.call(&mut vm, 2, 2).unwrap();
            assert!(calls.services.server.entities.set_never_free(id, true));
            let body = calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize);
            let bounds = calls.services.server.area.bounds(id);
            let bytes = vm
                .entities
                .read(u64::from(value), vm.stride)
                .unwrap()
                .to_vec();
            parameters(&mut vm, &[&[value]]);
            assert_eq!(calls.call(&mut vm, 15, 1), Err(Trap::Builtin));
            assert!(calls.services.server.entities.resolve(id).is_some());
            let after = calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize);
            assert_eq!(body_bits(after), body_bits(body));
            assert_eq!(calls.services.server.area.bounds(id), bounds);
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
        });
    }
}

#[test]
fn quakec_fly_flag_stays_visible_and_raw_at_shared_publication() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            write(&mut vm, value, b"flags", &[1.0f32.to_bits()]);
            write(&mut vm, value, b"modelindex", &[7.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            let columns = &calls.services.server.entities.columns;
            assert_eq!(columns.flags[id.slot as usize], 1);
            assert!(!columns.appearance[id.slot as usize].snapshot_excluded);
            assert_eq!(read(&vm, value, b"flags", 1), [1.0f32.to_bits()]);
        });
    }
}

#[test]
fn explicit_pose_size_and_item_link_use_shared_bounds_without_implicit_relinks() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        let id = entity(calls, &vm, value);
        write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
        write(&mut vm, value, b"flags", &[256.0f32.to_bits()]);
        let mins = [-2.0f32, -3.0, -4.0].map(f32::to_bits);
        let maxs = [2.0f32, 3.0, 4.0].map(f32::to_bits);
        parameters(&mut vm, &[&[value], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        let origin = [20.0f32, 10.0, -0.0].map(f32::to_bits);
        parameters(&mut vm, &[&[value], &origin]);
        calls.call(&mut vm, 2, 2).unwrap();
        let bounds = calls.services.server.area.bounds(id).unwrap();
        assert_eq!(
            bounds,
            Bounds {
                mins: Vec3([3.0, -8.0, -4.0]),
                maxs: Vec3([37.0, 28.0, 4.0])
            }
        );
        assert_eq!(
            read(&vm, value, b"absmin", 3),
            bounds.mins.0.map(f32::to_bits)
        );
        assert_eq!(
            read(&vm, value, b"size", 3),
            [4.0f32, 6.0, 8.0].map(f32::to_bits)
        );
        let relinks = calls.services.server.area.relinks;
        calls.call(&mut vm, 2, 2).unwrap();
        assert_eq!(calls.services.server.area.relinks, relinks + 1);
        write(&mut vm, value, b"origin", &[30.0f32.to_bits(), 0, 0]);
        write(&mut vm, value, b"modelindex", &[7.0f32.to_bits()]);
        write(&mut vm, value, b"effects", &[3.0f32.to_bits()]);
        vm.project_entities(calls.services, calls.context).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([30.0, 0.0, 0.0])
        );
        assert_eq!(calls.services.server.area.bounds(id), Some(bounds));
        let appearance = calls.services.server.entities.columns.appearance[id.slot as usize];
        assert_eq!(
            appearance.models[0],
            ModelRef::native(ResourceNamespace::Module(calls.context.module), 7)
        );
        assert_eq!(appearance.native.effects, 3);
        parameters(&mut vm, &[&[value], &maxs, &mins]);
        assert_eq!(calls.call(&mut vm, 4, 3), Err(Trap::Builtin));
        assert_eq!(read(&vm, value, b"mins", 3), mins);
        assert_eq!(calls.services.server.area.bounds(id), Some(bounds));
    });
}

#[test]
fn trace_uses_shared_collision_and_native_pass_and_result_offsets() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            let mins = [-2.0f32; 3].map(f32::to_bits);
            let maxs = [2.0f32; 3].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &mins, &maxs]);
            calls.call(&mut vm, 4, 3).unwrap();
            let origin = [10.0f32, 0.0, 0.0].map(f32::to_bits);
            parameters(&mut vm, &[&[value], &origin]);
            calls.call(&mut vm, 2, 2).unwrap();
            let start = [0u32; 3];
            let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
            parameters(&mut vm, &[&start, &end, &[0], &[0]]);
            calls.call(&mut vm, 16, 4).unwrap();
            assert!(f32::from_bits(vm.image.globals[62]) < 1.0);
            assert_eq!(vm.image.globals[72], value);
            assert_eq!(f32::from_bits(vm.image.globals[69]), -1.0);
            let before = f32::from_bits(vm.image.globals[62]);
            let id = entity(calls, &vm, value);
            let linked_bounds = calls.services.server.area.bounds(id);
            // A field store immediately preceding a trace must affect the
            // native narrow phase without inventing a relink.
            store_and_trace(calls, &mut vm, value, b"origin", &[12.0f32.to_bits(), 0, 0]).unwrap();
            assert!(f32::from_bits(vm.image.globals[62]) > before);
            assert_eq!(calls.services.server.area.bounds(id), linked_bounds);
            let box_fraction = vm.image.globals[62];
            write(&mut vm, value, b"angles", &[f32::NAN.to_bits(); 3]);
            calls.call(&mut vm, 16, 4).unwrap();
            assert_eq!(vm.image.globals[62], box_fraction);
            assert_eq!(calls.services.server.area.bounds(id), linked_bounds);
            // Full publication still validates the presentation angles.
            assert_eq!(
                vm.project_entities(calls.services, calls.context),
                Err(CallError::Entity)
            );
            write(&mut vm, value, b"angles", &[0; 3]);
            store_and_trace(calls, &mut vm, value, b"solid", &[0]).unwrap();
            assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
            assert_eq!(calls.services.server.area.bounds(id), linked_bounds);
            parameters(&mut vm, &[&start, &end, &[0], &[value]]);
            store_and_trace(calls, &mut vm, value, b"solid", &[2.0f32.to_bits()]).unwrap();
            assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
            assert_eq!(vm.image.globals[72], 0);
            parameters(&mut vm, &[&start, &end, &[1.0f32.to_bits()], &[0]]);
            calls.call(&mut vm, 16, 4).unwrap();
            assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
            parameters(&mut vm, &[&start, &end, &[0], &[1]]);
            assert_eq!(calls.call(&mut vm, 16, 4), Err(Trap::Builtin));
        });
    }
}

fn spawn_box(
    calls: &mut QuakeCCalls<'_, '_>,
    vm: &mut Vm,
    origin: [f32; 3],
    mins: [f32; 3],
    maxs: [f32; 3],
) -> u32 {
    calls.call(vm, 14, 0).unwrap();
    let value = vm.image.globals[1];
    write(vm, value, b"solid", &[2.0f32.to_bits()]);
    parameters(
        vm,
        &[&[value], &mins.map(f32::to_bits), &maxs.map(f32::to_bits)],
    );
    calls.call(vm, 4, 3).unwrap();
    parameters(vm, &[&[value], &origin.map(f32::to_bits)]);
    calls.call(vm, 2, 2).unwrap();
    value
}

fn solid_order(calls: &QuakeCCalls<'_, '_>) -> Vec<EntityId> {
    calls
        .services
        .server
        .area
        .query(
            &calls.services.server.entities,
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
            qa_world::area::LinkFlags::SOLID,
        )
        .map(|linked| linked.id)
        .collect()
}

#[test]
fn droptofloor_uses_live_box_and_caller_rules_over_a_shared_brush_world() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -512.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_collision_services(native, None, &[floor], |calls| {
            let mut vm = seeded_vm(calls, 8);
            let value = spawn_box(calls, &mut vm, [0.0, 0.0, 40.0], [-1.0; 3], [1.0; 3]);
            let id = entity(calls, &vm, value);
            for (trace_rules, epsilon) in
                [(RuleSetId::Quake, 0.03125f32), (RuleSetId::Quake3, 0.125)]
            {
                // The input is live, while the cached body still sits at40.
                write(
                    &mut vm,
                    value,
                    b"origin",
                    &[0.0f32, 0.0, 32.0].map(f32::to_bits),
                );
                write(&mut vm, value, b"flags", &[3.75f32.to_bits()]);
                write(&mut vm, value, b"groundentity", &[value]);
                vm.image.globals[34] = value;
                calls.context.trace_rules = trace_rules;
                calls.context.console.source = RuleSetId::Quake2;
                let trace_globals = vm.image.globals[60..73].to_vec();
                calls.call(&mut vm, 34, 0).unwrap();
                assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
                assert_eq!(
                    read(&vm, value, b"origin", 3),
                    [0.0f32, 0.0, 1.0 + epsilon].map(f32::to_bits)
                );
                assert_eq!(read(&vm, value, b"flags", 1), [515.0f32.to_bits()]);
                assert_eq!(read(&vm, value, b"groundentity", 1), [0]);
                assert_eq!(&vm.image.globals[60..73], trace_globals);
                assert_eq!(
                    calls.services.server.entities.columns.position[id.slot as usize],
                    Vec3([0.0, 0.0, 1.0 + epsilon])
                );
                // Relink precedes the flags write. The actual post-Think
                // dirty importer must then see the final native flags too.
                assert_eq!(
                    calls.services.server.entities.columns.flags[id.slot as usize],
                    3
                );
                let linked = calls.services.server.area.bounds(id);
                vm.project_dirty_entities(calls.services, calls.context)
                    .unwrap();
                assert_eq!(
                    calls.services.server.entities.columns.flags[id.slot as usize],
                    515
                );
                assert_eq!(calls.services.server.area.bounds(id), linked);
            }
        });
    }
}

#[test]
fn droptofloor_sets_linked_native_ground_and_reinserts_an_unchanged_pose() {
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(native, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let floor = spawn_box(
                calls,
                &mut vm,
                [0.0; 3],
                [-16.0, -16.0, -8.0],
                [16.0, 16.0, 0.0],
            );
            let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
            vm.image.globals[34] = value;
            calls.call(&mut vm, 34, 0).unwrap();
            assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
            assert_eq!(read(&vm, value, b"groundentity", 1), [floor]);
            assert_eq!(
                read(&vm, value, b"origin", 3),
                [0.0f32, 0.0, 1.03125].map(f32::to_bits)
            );
            let peer = spawn_box(calls, &mut vm, [8.0, 0.0, 1.03125], [-1.0; 3], [1.0; 3]);
            let value_id = entity(calls, &vm, value);
            let peer_id = entity(calls, &vm, peer);
            let order = solid_order(calls);
            assert!(
                order.iter().position(|id| *id == value_id).unwrap()
                    < order.iter().position(|id| *id == peer_id).unwrap()
            );
            let pose = read(&vm, value, b"origin", 3);
            let bounds = calls.services.server.area.bounds(value_id);
            let peer_bytes = vm
                .entities
                .read(u64::from(peer), vm.stride)
                .unwrap()
                .to_vec();
            vm.image.globals[34] = value;
            calls.call(&mut vm, 34, 0).unwrap();
            assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
            assert_eq!(read(&vm, value, b"origin", 3), pose);
            assert_eq!(calls.services.server.area.bounds(value_id), bounds);
            let order = solid_order(calls);
            assert!(
                order.iter().position(|id| *id == value_id).unwrap()
                    > order.iter().position(|id| *id == peer_id).unwrap()
            );
            assert_eq!(
                vm.entities.read(u64::from(peer), vm.stride).unwrap(),
                peer_bytes
            );
        });
    }
}

#[test]
fn droptofloor_miss_and_allsolid_preserve_native_row_common_body_and_links() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -512.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for boxes in [&[][..], &[floor][..]] {
            with_collision_services(native, None, boxes, |calls| {
                let mut vm = seeded_vm(calls, 8);
                let value = spawn_box(calls, &mut vm, [0.0, 0.0, -20.0], [-1.0; 3], [1.0; 3]);
                write(&mut vm, value, b"flags", &[81.5f32.to_bits()]);
                write(&mut vm, value, b"groundentity", &[value]);
                vm.image.globals[34] = value;
                let id = entity(calls, &vm, value);
                let bytes = vm
                    .entities
                    .read(u64::from(value), vm.stride)
                    .unwrap()
                    .to_vec();
                let body = calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize);
                let bounds = calls.services.server.area.bounds(id);
                let order = solid_order(calls);
                calls.call(&mut vm, 34, 0).unwrap();
                assert_eq!(vm.image.globals[1], 0.0f32.to_bits());
                assert_eq!(
                    vm.entities.read(u64::from(value), vm.stride).unwrap(),
                    bytes
                );
                assert_eq!(
                    body_bits(
                        calls
                            .services
                            .server
                            .entities
                            .columns
                            .body(id.slot as usize)
                    ),
                    body_bits(body)
                );
                assert_eq!(calls.services.server.area.bounds(id), bounds);
                assert_eq!(solid_order(calls), order);
            });
        }
    }
}

#[test]
fn droptofloor_accepts_startsolid_when_trace_reaches_an_actual_floor() {
    let boxes = [
        Bounds {
            mins: Vec3([-100.0, -100.0, -512.0]),
            maxs: Vec3([100.0, 100.0, 0.0]),
        },
        Bounds {
            mins: Vec3([-100.0, -100.0, 31.0]),
            maxs: Vec3([100.0, 100.0, 33.0]),
        },
    ];
    for native in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_collision_services(native, None, &boxes, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
            vm.image.globals[34] = value;
            let (rules, entity_rules) =
                qa_world::collision::trace_policy(calls.context.trace_rules);
            let mut query = qa_world::collision::TraceQuery::point(
                Vec3([0.0, 0.0, 32.0]),
                Vec3([0.0, 0.0, -224.0]),
                rules,
                entity_rules.with_quake_kind(Some(qa_world::collision::QuakeTraceKind::Normal)),
            );
            query.mins = Vec3([-1.0; 3]);
            query.maxs = Vec3([1.0; 3]);
            query.pass = CollisionOwner::Lifetime(entity(calls, &vm, value));
            let trace = (ENGINE_CALLS.trace)(calls.services, query, None).unwrap();
            assert!(trace.start_solid && !trace.all_solid && trace.fraction < 1.0);
            calls.call(&mut vm, 34, 0).unwrap();
            assert_eq!(vm.image.globals[1], 1.0f32.to_bits());
            assert_eq!(
                read(&vm, value, b"origin", 3),
                trace.end.0.map(f32::to_bits)
            );
            assert_eq!(read(&vm, value, b"groundentity", 1), [0]);
        });
    }
}

#[test]
fn droptofloor_rejects_only_executed_arity_or_required_field_paths_without_partial_relink() {
    let floor = Bounds {
        mins: Vec3([-100.0, -100.0, -512.0]),
        maxs: Vec3([100.0, 100.0, 0.0]),
    };
    for missing in [
        b"groundentity".as_slice(),
        b"origin",
        b"mins",
        b"maxs",
        b"flags",
        b"absmin",
        b"absmax",
        b"angles",
        b"solid",
        b"owner",
    ] {
        with_collision_services(RuleSetId::Quake, None, &[floor], |calls| {
            let fields = FIELDS
                .iter()
                .copied()
                .map(|mut definition| {
                    if definition.2 == missing {
                        definition.0 = 1;
                    }
                    definition
                })
                .collect::<Vec<_>>();
            let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
            // Loading unrelated bytecode and the ordinary spawn path still
            // work. Only the builtin's executed field path rejects.
            assert!(vm.call(calls, 1, 100, false).is_ok());
            calls.call(&mut vm, 14, 0).unwrap();
            let value = vm.image.globals[1];
            write(
                &mut vm,
                value,
                b"origin",
                &[0.0f32, 0.0, 32.0].map(f32::to_bits),
            );
            write(&mut vm, value, b"mins", &[-1.0f32; 3].map(f32::to_bits));
            write(&mut vm, value, b"maxs", &[1.0f32; 3].map(f32::to_bits));
            write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
            vm.image.globals[34] = value;
            let id = entity(calls, &vm, value);
            let bytes = vm
                .entities
                .read(u64::from(value), vm.stride)
                .unwrap()
                .to_vec();
            let body = calls
                .services
                .server
                .entities
                .columns
                .body(id.slot as usize);
            assert_eq!(calls.call(&mut vm, 34, 0), Err(Trap::Builtin));
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
            assert_eq!(
                body_bits(
                    calls
                        .services
                        .server
                        .entities
                        .columns
                        .body(id.slot as usize)
                ),
                body_bits(body)
            );
            assert_eq!(calls.services.server.area.bounds(id), None);
        });
    }
    with_services(RuleSetId::Quake, |calls| {
        let fields = FIELDS
            .iter()
            .copied()
            .filter(|field| field.2 != b"groundentity")
            .collect::<Vec<_>>();
        let mut vm = seed_vm(calls, vm_with_definitions(8, GLOBALS, &fields));
        let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
        vm.image.globals[34] = value;
        calls.call(&mut vm, 34, 0).unwrap(); // Miss never reads groundentity.
        assert_eq!(vm.image.globals[1], 0);
        let bytes = vm
            .entities
            .read(u64::from(value), vm.stride)
            .unwrap()
            .to_vec();
        assert_eq!(calls.call(&mut vm, 34, 1), Err(Trap::Builtin));
        assert_eq!(
            vm.entities.read(u64::from(value), vm.stride).unwrap(),
            bytes
        );
    });
    with_services(RuleSetId::Quake, |calls| {
        let globals = GLOBALS
            .iter()
            .copied()
            .map(|mut definition| {
                if definition.2 == b"self" {
                    definition.0 = 2;
                }
                definition
            })
            .collect::<Vec<_>>();
        let mut vm = seed_vm(calls, vm_with_definitions(8, &globals, FIELDS));
        assert!(vm.call(calls, 1, 100, false).is_ok());
        assert_eq!(calls.call(&mut vm, 34, 0), Err(Trap::Builtin));
    });
}

#[test]
fn droptofloor_finishes_native_candidate_validation_before_mutating_self() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let floor = spawn_box(
            calls,
            &mut vm,
            [0.0; 3],
            [-16.0, -16.0, -8.0],
            [16.0, 16.0, 0.0],
        );
        let value = spawn_box(calls, &mut vm, [0.0, 0.0, 32.0], [-1.0; 3], [1.0; 3]);
        // This selected floor's live owner is malformed after its old area
        // membership was published. The view captures the boundary error.
        write(&mut vm, floor, b"owner", &[1]);
        vm.image.globals[34] = value;
        let bytes = vm
            .entities
            .read(u64::from(value), vm.stride)
            .unwrap()
            .to_vec();
        let id = entity(calls, &vm, value);
        let body = calls
            .services
            .server
            .entities
            .columns
            .body(id.slot as usize);
        let bounds = calls.services.server.area.bounds(id);
        let order = solid_order(calls);
        assert_eq!(calls.call(&mut vm, 34, 0), Err(Trap::Builtin));
        assert_eq!(
            vm.entities.read(u64::from(value), vm.stride).unwrap(),
            bytes
        );
        assert_eq!(
            body_bits(
                calls
                    .services
                    .server
                    .entities
                    .columns
                    .body(id.slot as usize)
            ),
            body_bits(body)
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        assert_eq!(solid_order(calls), order);
    });
}

#[test]
fn quakec_trace_role_controls_epsilon_and_linked_merge_independently_of_console_source() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let front = vm.image.globals[1];
        write(&mut vm, front, b"solid", &[2.0f32.to_bits()]);
        let mins = [-1.0f32; 3].map(f32::to_bits);
        let maxs = [1.0f32; 3].map(f32::to_bits);
        parameters(&mut vm, &[&[front], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        let origin = [12.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&[front], &origin]);
        calls.call(&mut vm, 2, 2).unwrap();
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        let mut fractions = [0; 2];
        for (index, trace_rules) in [RuleSetId::Quake, RuleSetId::Quake3]
            .into_iter()
            .enumerate()
        {
            calls.context.trace_rules = trace_rules;
            for console_source in [RuleSetId::Quake, RuleSetId::Quake3] {
                calls.context.console.source = console_source;
                parameters(&mut vm, &[&start, &end, &[0], &[0]]);
                calls.call(&mut vm, 16, 4).unwrap();
                let epsilon = if index == 0 { 0.03125 } else { 0.125 };
                let expected = ((11.0 - epsilon) / 20.0) as f32;
                // Box-hull recursion composes near-side fractions. Its native
                // f32 rounding can differ from this single-plane estimate.
                assert!((f32::from_bits(vm.image.globals[62]) - expected).abs() <= f32::EPSILON);
                assert_eq!(vm.image.globals[72], front);
                if console_source == RuleSetId::Quake {
                    fractions[index] = vm.image.globals[62];
                } else {
                    assert_eq!(vm.image.globals[62], fractions[index]);
                }
            }
        }
        assert!(f32::from_bits(fractions[1]) < f32::from_bits(fractions[0]));

        calls.call(&mut vm, 14, 0).unwrap();
        let containing = vm.image.globals[1];
        write(&mut vm, containing, b"solid", &[2.0f32.to_bits()]);
        parameters(&mut vm, &[&[containing], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        parameters(&mut vm, &[&[containing], &start]);
        calls.call(&mut vm, 2, 2).unwrap();
        for (index, trace_rules) in [RuleSetId::Quake, RuleSetId::Quake3]
            .into_iter()
            .enumerate()
        {
            calls.context.trace_rules = trace_rules;
            for console_source in [RuleSetId::QuakeWorld, RuleSetId::Quake2Rerelease] {
                calls.context.console.source = console_source;
                parameters(&mut vm, &[&start, &end, &[0], &[0]]);
                calls.call(&mut vm, 16, 4).unwrap();
                assert_eq!(vm.image.globals[61], 1.0f32.to_bits());
                assert_eq!(vm.image.globals[60], 0);
                assert_eq!(
                    vm.image.globals[62],
                    if index == 0 {
                        1.0f32.to_bits()
                    } else {
                        fractions[1]
                    }
                );
                assert_eq!(
                    vm.image.globals[72],
                    if index == 0 { containing } else { front }
                );
            }
        }
        let slot = entity(calls, &vm, containing).slot as usize;
        assert_eq!(
            calls.services.server.entities.columns.appearance[slot].source,
            RuleSetId::Quake
        );
    });
}

#[test]
fn quakec_native_print_signature_does_not_follow_the_console_view() {
    use qa_core::events::{FrameEvent, OutputSubmission};
    for native_rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(native_rules, |calls| {
            let mut vm = seeded_vm(calls, 4);
            let text = vm.insert_string(b"native ABI unchanged\xff").unwrap() as u32;
            for console_source in [RuleSetId::Quake3, RuleSetId::Quake2Rerelease] {
                calls.context.console.source = console_source;
                if native_rules == RuleSetId::QuakeWorld {
                    parameters(&mut vm, &[&[3.0f32.to_bits()], &[text]]);
                    calls.call(&mut vm, 23, 2).unwrap();
                } else {
                    parameters(&mut vm, &[&[text]]);
                    calls.call(&mut vm, 23, 1).unwrap();
                }
                let ring = &mut calls.services.server.events;
                let consumer = calls.services.server.presentation;
                let mut batch = ring.batch(consumer).unwrap();
                let record = ring.next(&mut batch).unwrap();
                let FrameEvent::Print(print) = record.event else {
                    panic!("expected print")
                };
                assert_eq!(
                    ring.texts.get(print.text),
                    Some(&b"native ABI unchanged\xff"[..])
                );
                assert_eq!(
                    print.kind,
                    if native_rules == RuleSetId::QuakeWorld {
                        PrintKind::Chat
                    } else {
                        PrintKind::Console
                    }
                );
                assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
            }
        });
    }
}

#[test]
fn spatial_queries_read_storep_without_publishing_unrelated_dirty_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        let id = entity(calls, &vm, value);
        calls.call(&mut vm, 14, 0).unwrap();
        let untouched = vm.image.globals[1];
        // Direct host edits are synchronized by the export scan. They must not
        // make every spatial query inspect a different, untouched ABI row.
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        // Executed StorePV makes this unlinked row malformed. Trace must not
        // validate its bounds merely because it has a dirty publication mark.
        store_and_trace(calls, &mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]).unwrap();
        assert_eq!(vm.image.globals[62], 1.0f32.to_bits());
        store_and_trace(calls, &mut vm, value, b"origin", &[12.0f32.to_bits(), 0, 0]).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3::default()
        );
        calls.call(&mut vm, 16, 4).unwrap();
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Entity)
        );
        write(&mut vm, untouched, b"mins", &[0; 3]);
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([12.0, 0.0, 0.0])
        );
        vm.project_entities(calls.services, calls.context).unwrap();
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        vm.image.globals[34] = value;
        vm.image.globals[35] = 3.0f32.to_bits();
        vm.image.globals[28] = 37.0f32.to_bits();
        vm.image.globals[29] = 0x0100_0001;
        vm.image.globals[32] = 5;
        vm.call(calls, 4, 100, false).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.appearance[id.slot as usize].frame,
            37
        );
        assert_eq!(read(&vm, value, b"think", 1), [0x0100_0001]);
        assert_eq!(
            read(&vm, value, b"nextthink", 1),
            [((3.0f64 + 0.1) as f32).to_bits()]
        );
        vm.image.globals[28] = 41.0f32.to_bits();
        assert_eq!(vm.call(calls, 8, 2, false), Err(Trap::Budget));
        calls.call(&mut vm, 16, 4).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.appearance[id.slot as usize].frame,
            41
        );
    });
}

#[test]
fn pointer_stores_mark_crossed_rows_and_flush_the_current_reused_lifetime() {
    with_services(RuleSetId::Quake, |calls| {
        calls.context.clock = ThinkTime::Seconds(1.0);
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        let old = entity(calls, &vm, first);
        calls.call(&mut vm, 14, 0).unwrap();
        let second = vm.image.globals[1];
        let second_id = entity(calls, &vm, second);
        assert_eq!(second as usize, first as usize + vm.stride);
        write(&mut vm, second, b"origin", &[15.0f32.to_bits(), 0, 0]);
        vm.image.globals[28..31].fill(0);
        vm.image.globals[31] = first + vm.stride as u32 - 4;
        vm.image.globals[32] = 5;
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        vm.call(calls, 2, 100, false).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[second_id.slot as usize],
            Vec3([15.0, 0.0, 0.0])
        );
        write(&mut vm, second, b"origin", &[17.0f32.to_bits(), 0, 0]);
        // Native pointers can be unaligned. This single word touches both rows.
        vm.image.globals[28] = 0;
        vm.image.globals[31] = first + vm.stride as u32 - 2;
        vm.call(calls, 3, 100, false).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[second_id.slot as usize],
            Vec3([17.0, 0.0, 0.0])
        );
        vm.image.globals[28] = 11.0f32.to_bits();
        vm.image.globals[31] = vm.field_address(first, 0, 1).unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        parameters(&mut vm, &[&[first]]);
        calls.call(&mut vm, 15, 1).unwrap();
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.image.globals[1], first);
        let new = entity(calls, &vm, first);
        assert_ne!(new, old);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        calls.call(&mut vm, 16, 4).unwrap();
        assert!(calls.services.server.entities.resolve(old).is_none());
        assert_eq!(
            calls.services.server.entities.columns.position[new.slot as usize],
            Vec3([0.0; 3])
        );
    });
}

#[test]
fn rejected_dirty_import_keeps_bounded_membership_until_it_can_retry() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 4);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        let first_id = entity(calls, &vm, first);
        calls.call(&mut vm, 14, 0).unwrap();
        let second = vm.image.globals[1];
        let second_id = entity(calls, &vm, second);
        write(&mut vm, second, b"mins", &[4.0f32.to_bits(); 3]);
        vm.image.globals[28] = 0;
        vm.image.globals[31] = vm.field_address(first, 0, 1).unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        vm.image.globals[28] = 9.0f32.to_bits();
        vm.image.globals[31] = vm
            .field_address(second, vm.image.field(b"frame").unwrap() as u32, 1)
            .unwrap() as u32;
        vm.call(calls, 6, 100, false).unwrap();
        let start = [0; 3];
        let end = [20.0f32, 0.0, 0.0].map(f32::to_bits);
        parameters(&mut vm, &[&start, &end, &[0], &[0]]);
        for x in 1..=16 {
            store_and_trace(
                calls,
                &mut vm,
                first,
                b"origin",
                &[(x as f32).to_bits(), 0, 0],
            )
            .unwrap();
            assert_eq!(
                vm.project_dirty_entities(calls.services, calls.context),
                Err(CallError::Entity)
            );
        }
        write(&mut vm, second, b"mins", &[0; 3]);
        calls.call(&mut vm, 16, 4).unwrap();
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[first_id.slot as usize],
            Vec3([16.0, 0.0, 0.0])
        );
        assert_eq!(
            calls.services.server.entities.columns.appearance[second_id.slot as usize].frame,
            9
        );
    });
}

#[test]
fn full_qw_reuses_exact_last_ordinal_and_stale_free_writes_do_not_relink() {
    with_services(RuleSetId::QuakeWorld, |calls| {
        let mut vm = seeded_vm(calls, 4);
        calls.call(&mut vm, 14, 0).unwrap();
        let first = vm.image.globals[1];
        let first_id = entity(calls, &vm, first);
        calls.call(&mut vm, 14, 0).unwrap();
        let last = vm.image.globals[1];
        let last_id = entity(calls, &vm, last);
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.image.globals[1], last);
        assert!(calls.services.server.entities.resolve(last_id).is_none());
        assert!(calls.services.server.entities.resolve(first_id).is_some());
        let replacement = entity(calls, &vm, last);
        assert_eq!(replacement.slot, last_id.slot);
        assert_eq!(replacement.generation, last_id.generation + 1);
        parameters(&mut vm, &[&[last]]);
        calls.call(&mut vm, 15, 1).unwrap();
        let relinks = calls.services.server.area.relinks;
        let origin = [9.0f32, 8.0, 7.0].map(f32::to_bits);
        parameters(&mut vm, &[&[last], &origin]);
        calls.call(&mut vm, 2, 2).unwrap();
        assert_eq!(read(&vm, last, b"origin", 3), origin);
        assert_eq!(calls.services.server.area.relinks, relinks);
        assert!(
            calls
                .services
                .server
                .entities
                .resolve(replacement)
                .is_none()
        );
    });
}

#[test]
fn grace_reuse_and_invalid_operand_traps_do_not_create_lifetimes() {
    with_services(RuleSetId::Quake, |calls| {
        calls.context.clock = ThinkTime::Seconds(1.0);
        let mut vm = seeded_vm(calls, 4);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        parameters(&mut vm, &[&[value]]);
        calls.call(&mut vm, 15, 1).unwrap();
        calls.call(&mut vm, 14, 0).unwrap();
        assert_eq!(vm.image.globals[1], value);
        let live = calls.services.server.entities.active().count();
        let origin = [0u32; 3];
        for invalid in [1u32, vm.entities.len() as u32, u32::MAX] {
            parameters(&mut vm, &[&[invalid], &origin]);
            assert_eq!(calls.call(&mut vm, 2, 2), Err(Trap::Builtin));
        }
        assert_eq!(calls.call(&mut vm, 2, 1), Err(Trap::Builtin));
        assert_eq!(calls.services.server.entities.active().count(), live);
    });
}

#[test]
fn vm_rejects_zero_stride_before_entity_number_or_builtin_resolution() {
    let bytes = quakec_program::program(&[(Opcode::Return, 1, 0, 0)], &[0, 1]);
    let mut image = Image::parse(&bytes, Some(5927)).unwrap();
    image.entityfields = 0;
    assert!(matches!(
        Vm::load(
            image,
            Layout {
                entities: 1,
                header_bytes: 0,
                extra_string_bytes: 0,
                state_step: 0.1
            }
        ),
        Err(Trap::Memory)
    ));
}

#[test]
fn native_staging_dense_sounds_reject_the_whole_group_without_resync() {
    use qa_core::events::{FrameEvent, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let sound = bind_native_message_sound(calls);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let mut wire = vec![6; NATIVE_WIRE_BYTES];
            for cue in wire[..(NATIVE_WIRE_BYTES / 3) * 3].chunks_exact_mut(3) {
                cue.copy_from_slice(&[9, 0, 1]);
            }
            stage_native_message(calls, &wire);
            assert_eq!(
                (ENGINE_CALLS.message_bits)(calls.services, ModuleId(1), 6, 8),
                Err(CallError::Capacity)
            );
            let before = calls.services.server.events.sequence_range();
            let counters = calls.services.server.events.counters(consumer).unwrap();
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(rules),
                    calls.context.sound_policy
                ),
                Err(CallError::Capacity)
            );
            let ring = &calls.services.server.events;
            assert_eq!(ring.sequence_range(), before);
            assert_eq!(ring.counters(consumer), Some(counters));
            assert!(!ring.needs_resync(consumer));
            assert_eq!(ring.rejected_publications(), 1);
            // The failed emission consumed staging and cannot retry a prefix.
            assert_eq!(
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(rules),
                    calls.context.sound_policy
                )
                .unwrap(),
                None
            );
            stage_native_message(calls, &[9, 0, 1]);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            let ring = &calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            assert!(
                matches!(ring.next(&mut batch).unwrap().event, FrameEvent::Sound(e) if e.sound == sound && e.native_fields.unwrap().flags == 0)
            );
            assert!(ring.next(&mut batch).is_none());
        });
    }
}
fn native_message_delivery(rules: RuleSetId) -> NativeDelivery {
    NativeDelivery {
        module: ModuleId(1),
        source: rules,
        target: NativeTarget::All,
        origin: Vec3::default(),
        reliable: true,
        dupe_key: 0,
        component: 0,
        emission: None,
        check_portals: false,
    }
}
fn stage_native_message(calls: &mut QuakeCCalls<'_, '_>, bytes: &[u8]) {
    for &byte in bytes {
        (ENGINE_CALLS.message_bits)(calls.services, ModuleId(1), u32::from(byte), 8).unwrap();
    }
}
fn bind_native_message_sound(calls: &mut QuakeCCalls<'_, '_>) -> SoundId {
    let rate = std::num::NonZeroU32::new(44100).unwrap();
    let bank = std::sync::Arc::new(
        qa_audio::Bank::load(
            rate,
            vec![(
                "sound/test.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![100; 4],
                    loop_start: None,
                },
            )],
        )
        .unwrap(),
    );
    let sound = bank.find("sound/test.wav").unwrap();
    let sounds = calls.services.storage.resources(ModuleId(1)).unwrap()[1];
    (ENGINE_CALLS.configstring)(
        calls.services,
        ModuleId(1),
        sounds.first as usize + 1,
        b"test.wav",
    )
    .unwrap();
    calls
        .services
        .storage
        .bind_sound_bank(ModuleId(1), sounds, bank)
        .unwrap();
    sound
}
#[test]
fn native_mixed_print_and_sound_components_use_one_ring_in_source_order() {
    use qa_core::events::{FrameEvent, OutputSubmission, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let sound = bind_native_message_sound(calls);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let bytes = [10, 2, b'p', 0, 9, 3, 1, 255, 64, 9, 0, 1, 10, 2, b'q', 0];
            for _ in 0..2 {
                stage_native_message(calls, &bytes);
                (ENGINE_CALLS.native_message)(
                    calls.services,
                    native_message_delivery(rules),
                    calls.context.sound_policy,
                )
                .unwrap();
            }
            // Later config mutation cannot replace the captured PCM handle.
            let sounds = calls.services.storage.resources(ModuleId(1)).unwrap()[1];
            (ENGINE_CALLS.configstring)(
                calls.services,
                ModuleId(1),
                sounds.first as usize + 1,
                b"missing.wav",
            )
            .unwrap();
            let ring = &mut calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            for _ in 0..2 {
                for (component, expected_flags, expected_text) in [
                    (0, None, Some(&bytes[..4])),
                    (4, Some(3), None),
                    (9, Some(0), None),
                    (12, None, Some(&bytes[12..])),
                ] {
                    let record = ring.next(&mut batch).unwrap();
                    assert_eq!(record.delivery.unwrap().component, component);
                    match record.event {
                        FrameEvent::Sound(event) => {
                            assert_eq!(event.sound, sound);
                            assert_eq!(event.native_fields.unwrap().flags, expected_flags.unwrap());
                            assert_eq!(event.spatial, calls.context.sound_policy);
                        }
                        FrameEvent::Print(print) => {
                            let qa_core::primitives::PrintText::Native { wire, text, .. } =
                                print.text
                            else {
                                panic!("native print must retain the original wire pages")
                            };
                            let mut scratch = [0; 64];
                            let bytes = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                            assert_eq!(bytes, expected_text.unwrap());
                            assert_eq!(&bytes[text.range()], &expected_text.unwrap()[2..3]);
                            assert_eq!((print.kind, print.level), (PrintKind::Console, 2));
                        }
                        _ => panic!("unexpected component"),
                    }
                    assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
                }
            }
            assert!(ring.next(&mut batch).is_none());
            assert!(ring.is_empty());
            assert_eq!(ring.rejected_publications(), 0);
        });
    }
}

#[test]
fn native_print_retains_long_rr_text_and_opaque_classic_continuation_once() {
    use qa_core::events::{FrameEvent, OutputTarget};
    use qa_core::primitives::PrintText;
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let mut bytes = vec![10, 0];
            bytes.extend(std::iter::repeat_n(
                b'x',
                if rules == RuleSetId::Quake2 {
                    2050
                } else {
                    9000
                },
            ));
            bytes.push(0);
            bytes.extend_from_slice(b"\x0fnext\0");
            stage_native_message(calls, &bytes);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            let ring = &calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            let record = ring.next(&mut batch).unwrap();
            let FrameEvent::Print(print) = record.event else {
                panic!("typed native print")
            };
            let PrintText::Native {
                wire,
                text,
                consumed,
            } = print.text
            else {
                panic!("one raw owner")
            };
            let mut scratch = vec![0; bytes.len()];
            let raw = ring.texts.copy_payload(wire, &mut scratch).unwrap();
            if rules == RuleSetId::Quake2 {
                assert_eq!(raw, bytes);
                assert_eq!(consumed, 2049);
                assert_eq!(text.count, 2047);
                assert!(
                    ring.next(&mut batch).is_none(),
                    "unknown tail must not be independently sent after an oversized prefix"
                );
            } else {
                assert_eq!(raw, &bytes[..9003]);
                assert_eq!(consumed, 9003);
                assert_eq!(text.count, 9000);
                let FrameEvent::Print(after) = ring.next(&mut batch).unwrap().event else {
                    panic!("later RR print")
                };
                let PrintText::Native { wire, text, .. } = after.text else {
                    panic!("typed next")
                };
                let mut scratch = [0; 16];
                let raw = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                assert_eq!(&raw[text.range()], b"next");
                assert!(ring.next(&mut batch).is_none());
            }
            assert_eq!((print.kind, print.level), (PrintKind::Notify, 0));
            assert_eq!(ring.texts.truncated(), 0);
        });
    }
    with_services(RuleSetId::Quake2, |calls| {
        let consumer = calls
            .services
            .server
            .events
            .bind(OutputTarget::Client(ClientId(0)))
            .unwrap();
        let bytes = b"\x0a\x01a\x80\xff\0opaque";
        stage_native_message(calls, bytes);
        (ENGINE_CALLS.native_message)(
            calls.services,
            native_message_delivery(RuleSetId::Quake2),
            calls.context.sound_policy,
        )
        .unwrap();
        let ring = &calls.services.server.events;
        let mut batch = ring.batch(consumer).unwrap();
        let FrameEvent::Print(print) = ring.next(&mut batch).unwrap().event else {
            panic!("print")
        };
        let PrintText::Native {
            wire,
            text,
            consumed,
        } = print.text
        else {
            panic!("raw native print")
        };
        let mut scratch = [0; 64];
        let raw = ring.texts.copy_payload(wire, &mut scratch).unwrap();
        assert_eq!(raw, bytes);
        assert_eq!(&raw[text.range()], b"a\x80");
        assert_eq!(consumed, 5);
        assert!(ring.next(&mut batch).is_none());
    });
}

#[test]
fn native_sound_captures_the_emitter_lifetime_before_slot_reuse() {
    use qa_core::events::{FrameEvent, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let sound = bind_native_message_sound(calls);
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let old = calls
                .services
                .server
                .entities
                .allocate(
                    ThinkTime::Seconds(0.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            let native = NativeEntity {
                module: ModuleId(1),
                slot: 7,
            };
            calls.services.server.entities.columns.native_entity[old.slot as usize] = Some(native);
            stage_native_message(calls, &[9, 8, 1, 58, 0]);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            assert!(
                calls
                    .services
                    .server
                    .entities
                    .release(old, ThinkTime::Seconds(1.0))
            );
            let replacement = calls
                .services
                .server
                .entities
                .allocate(
                    ThinkTime::Seconds(1000.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT,
                )
                .unwrap()
                .id;
            assert_eq!(replacement.slot, old.slot);
            assert_ne!(replacement.generation, old.generation);
            calls.services.server.entities.columns.native_entity[replacement.slot as usize] =
                Some(native);
            let ring = &calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            let FrameEvent::Sound(event) = ring.next(&mut batch).unwrap().event else {
                panic!("missing sound")
            };
            assert_eq!(event.sound, sound);
            assert_eq!(event.entity, Some(SoundEmitter::Entity(old)));
            assert_eq!(event.native_fields.unwrap().entity, 7);
            assert_eq!(event.source_ordinal, Some(1));
            assert!(ring.next(&mut batch).is_none());
        });
    }
}

#[test]
fn unresolved_native_sound_preserves_sibling_print_runs() {
    use qa_core::events::{FrameEvent, OutputSubmission, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        with_services(rules, |calls| {
            let consumer = calls
                .services
                .server
                .events
                .bind(OutputTarget::Client(ClientId(0)))
                .unwrap();
            let bytes = [10, 2, b'p', 0, 9, 0, 1, 10, 2, b'q', 0];
            stage_native_message(calls, &bytes);
            (ENGINE_CALLS.native_message)(
                calls.services,
                native_message_delivery(rules),
                calls.context.sound_policy,
            )
            .unwrap();
            let ring = &mut calls.services.server.events;
            let mut batch = ring.batch(consumer).unwrap();
            for (component, expected) in [(0, &bytes[..4]), (7, &bytes[7..])] {
                let record = ring.next(&mut batch).unwrap();
                assert_eq!(record.delivery.unwrap().component, component);
                let FrameEvent::Print(print) = record.event else {
                    panic!("unresolved sound must not publish")
                };
                let qa_core::primitives::PrintText::Native { wire, text, .. } = print.text else {
                    panic!("native print storage")
                };
                let mut scratch = [0; 32];
                let bytes = ring.texts.copy_payload(wire, &mut scratch).unwrap();
                assert_eq!(bytes, expected);
                assert_eq!(&bytes[text.range()], &expected[2..3]);
                assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
            }
            assert!(ring.next(&mut batch).is_none());
            assert!(ring.is_empty());
            assert!(!ring.needs_resync(consumer));
            assert_eq!(ring.rejected_publications(), 1);
        });
    }
}

#[test]
fn point_contents_reads_live_candidates_without_flushing_unrelated_dirty_rows() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        calls.call(&mut vm, 14, 0).unwrap();
        let value = vm.image.globals[1];
        let id = entity(calls, &vm, value);
        write(&mut vm, value, b"solid", &[2.0f32.to_bits()]);
        let mins = [-2.0f32; 3].map(f32::to_bits);
        let maxs = [2.0f32; 3].map(f32::to_bits);
        parameters(&mut vm, &[&[value], &mins, &maxs]);
        calls.call(&mut vm, 4, 3).unwrap();
        let bounds = calls.services.server.area.bounds(id);
        // A different unlinked, untouched row is malformed but not scanned.
        calls.call(&mut vm, 14, 0).unwrap();
        let untouched = vm.image.globals[1];
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        parameters(&mut vm, &[&[0; 3]]);
        vm.image.globals[28..31].copy_from_slice(&[12.0f32.to_bits(), 0, 0]);
        vm.image.globals[31] = vm
            .field_address(value, vm.image.field(b"origin").unwrap() as u32, 3)
            .unwrap() as u32;
        vm.image.globals[32] = 10;
        vm.call(calls, 9, 100, false).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3::default()
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // Original Q1 pointcontents remains world-only despite the live row.
        assert_eq!(vm.image.globals[1], (-1.0f32).to_bits());
        // Publication retains its existing explicit boundary; the query did
        // not consume the dirty mark or change the area membership.
        vm.project_dirty_entities(calls.services, calls.context)
            .unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([12.0, 0.0, 0.0])
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // A real bytecode store makes an unrelated unlinked row malformed.
        // Stock PointContents does not inspect or publish any entity row.
        vm.image.globals[28..31].copy_from_slice(&[4.0f32.to_bits(); 3]);
        vm.image.globals[31] = vm
            .field_address(untouched, vm.image.field(b"mins").unwrap() as u32, 3)
            .unwrap() as u32;
        vm.call(calls, 9, 100, false).unwrap();
        assert_eq!(vm.image.globals[1], (-1.0f32).to_bits());
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // A combined caller selects linked contents independently of the QC
        // source. Raw host/native bytes have no dirty mark: the candidate-local
        // view must decode this linked row without importing unrelated rows.
        calls.context.trace_rules = RuleSetId::Quake2;
        write(&mut vm, value, b"origin", &[0; 3]);
        write(&mut vm, value, b"mins", &[4.0f32.to_bits(); 3]);
        parameters(&mut vm, &[&[0; 3]]);
        assert_eq!(calls.call(&mut vm, 41, 1), Err(Trap::Builtin));
        assert_eq!(
            calls.services.server.entities.columns.mins[id.slot as usize],
            Vec3([-2.0; 3])
        );
        assert_eq!(calls.services.server.area.bounds(id), bounds);
        // The caller's linked-content policy uses the old solid chain even
        // after the QC row becomes nonsolid. Bounds remain executed fields.
        write(&mut vm, value, b"solid", &[0]);
        write(&mut vm, value, b"owner", &[1]);
        assert_eq!(calls.call(&mut vm, 41, 1), Err(Trap::Builtin));
        write(&mut vm, value, b"mins", &[(-2.0f32).to_bits(); 3]);
        // Native temporary boxes do not use angles or owner for contents.
        write(&mut vm, value, b"angles", &[f32::NAN.to_bits(); 3]);
        calls.call(&mut vm, 41, 1).unwrap();
        assert_eq!(vm.image.globals[1], (-1.0f32).to_bits());
        assert_eq!(calls.services.server.area.bounds(id), bounds);
    });
}

#[test]
fn makestatic_copies_native_signon_values_then_frees_the_original_lifetime() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let mut vm = seeded_vm(calls, 8);
            let anchor = entity(calls, &vm, 0);
            calls
                .services
                .storage
                .bind_lifetime(calls.context.module, anchor)
                .unwrap();
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let models = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[0];
            let model = (ENGINE_CALLS.resource_index)(
                calls.services,
                calls.context.module,
                models,
                b"progs/flame.mdl",
            )
            .unwrap();
            let name = vm.insert_string(b"progs/flame.mdl").unwrap() as u32;
            write(&mut vm, value, b"model", &[name]);
            write(&mut vm, value, b"modelindex", &[99.0f32.to_bits()]);
            write(
                &mut vm,
                value,
                b"origin",
                &[-0.2f32, -0.999, 4096.2].map(f32::to_bits),
            );
            write(
                &mut vm,
                value,
                b"angles",
                &[1.9f32, 180.0, -1.9].map(f32::to_bits),
            );
            write(&mut vm, value, b"frame", &[258.9f32.to_bits()]);
            write(&mut vm, value, b"skin", &[(-1.9f32).to_bits()]);
            write(&mut vm, value, b"colormap", &[513.9f32.to_bits()]);
            write(&mut vm, value, b"effects", &[999.0f32.to_bits()]);
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Remote,
                    calls.context.module,
                    None,
                )
                .unwrap();
            let consumer = calls.services.server.clients[client.0 as usize]
                .output
                .unwrap();
            parameters(&mut vm, &[&[value]]);
            calls.call(&mut vm, 69, 1).unwrap();
            assert!(calls.services.server.entities.resolve(id).is_none());
            assert!(calls.services.server.area.bounds(id).is_none());
            assert_eq!(read(&vm, value, b"model", 1), [0]);
            assert_eq!(read(&vm, value, b"nextthink", 1), [(-1.0f32).to_bits()]);
            assert_eq!(read(&vm, value, b"origin", 3), [0; 3]);
            let rate = std::num::NonZeroU32::new(44100).unwrap();
            let bank = std::sync::Arc::new(qa_audio::Bank::load(rate, Vec::new()).unwrap());
            let sounds = calls
                .services
                .storage
                .resources(calls.context.module)
                .unwrap()[1];
            calls
                .services
                .storage
                .bind_sound_bank(calls.context.module, sounds, bank)
                .unwrap();
            assert_eq!(
                calls.services.storage.publish_signon(
                    calls.services.server,
                    calls.context,
                    client,
                    &mut SignonCursor::default(),
                ),
                Ok(1)
            );
            let mut batch = calls.services.server.events.batch(consumer).unwrap();
            let record = calls.services.server.events.next(&mut batch).unwrap();
            let qa_core::events::FrameEvent::StaticEntity(record) = record.event else {
                panic!("static entity");
            };
            assert_eq!(record.position, Vec3([-0.125, -0.875, -4095.875]));
            assert_eq!(
                record.angles,
                if rules == RuleSetId::Quake {
                    Vec3([0.0, -180.0, 0.0])
                } else {
                    Vec3([1.40625, -180.0, -1.40625])
                }
            );
            assert_eq!(
                record.appearance.models[0],
                ModelRef::native(ResourceNamespace::Module(calls.context.module), model)
            );
            assert_eq!(
                (
                    record.appearance.frame,
                    record.appearance.skin,
                    record.appearance.native.colormap,
                    record.appearance.native.effects
                ),
                (2, 255, 1, 0)
            );
            calls.context.clock = ThinkTime::Seconds(4.0);
            let reused = vm.spawn_entity(calls.services, calls.context).unwrap();
            assert_eq!(reused, value);
            assert_ne!(entity(calls, &vm, reused), id);
            assert_eq!(record.position, Vec3([-0.125, -0.875, -4095.875]));
        });
    }
}

#[test]
fn makestatic_rejects_protected_and_malformed_removal_without_retained_ghosts() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let anchor = entity(calls, &vm, 0);
        calls
            .services
            .storage
            .bind_lifetime(calls.context.module, anchor)
            .unwrap();
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        calls.services.server.entities.set_never_free(id, true);
        let bytes = vm
            .entities
            .read(u64::from(value), vm.stride)
            .unwrap()
            .to_vec();
        assert_eq!(
            vm.make_static(calls.services, calls.context, value),
            Err(CallError::Entity)
        );
        assert_eq!(
            vm.make_static(calls.services, calls.context, 0),
            Err(CallError::Entity)
        );
        assert_eq!(
            vm.entities.read(u64::from(value), vm.stride).unwrap(),
            bytes
        );
        assert!(calls.services.server.entities.resolve(id).is_some());
        let rate = std::num::NonZeroU32::new(44100).unwrap();
        let bank = std::sync::Arc::new(qa_audio::Bank::load(rate, Vec::new()).unwrap());
        let sounds = calls
            .services
            .storage
            .resources(calls.context.module)
            .unwrap()[1];
        calls
            .services
            .storage
            .bind_sound_bank(calls.context.module, sounds, bank)
            .unwrap();
        let client = calls
            .services
            .server
            .connect(
                qa_session::clients::Connection::Remote,
                ModuleId::default(),
                None,
            )
            .unwrap();
        assert_eq!(
            calls.services.storage.publish_signon(
                calls.services.server,
                calls.context,
                client,
                &mut SignonCursor::default(),
            ),
            Ok(0)
        );
    });
    for field in [
        b"model".as_slice(),
        b"frame",
        b"skin",
        b"colormap",
        b"nextthink",
        b"origin",
        b"angles",
        b"solid",
        b"takedamage",
        b"modelindex",
    ] {
        with_services(RuleSetId::Quake, |calls| {
            let definitions: Vec<_> = FIELDS
                .iter()
                .copied()
                .filter(|(_, _, name)| *name != field)
                .collect();
            let image = vm_with_definitions(8, GLOBALS, &definitions);
            let mut vm = seed_vm(calls, image);
            let anchor = entity(calls, &vm, 0);
            calls
                .services
                .storage
                .bind_lifetime(calls.context.module, anchor)
                .unwrap();
            let value = vm.spawn_entity(calls.services, calls.context).unwrap();
            let id = entity(calls, &vm, value);
            let bytes = vm
                .entities
                .read(u64::from(value), vm.stride)
                .unwrap()
                .to_vec();
            assert_eq!(
                vm.make_static(calls.services, calls.context, value),
                Err(CallError::Memory),
                "{field:?}"
            );
            assert!(calls.services.server.entities.resolve(id).is_some());
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
        });
    }
}

#[test]
fn makestatic_capacity_drop_still_releases_the_source_edict() {
    with_services(RuleSetId::Quake, |calls| {
        let mut vm = seeded_vm(calls, 8);
        let anchor = entity(calls, &vm, 0);
        calls
            .services
            .storage
            .bind_lifetime(calls.context.module, anchor)
            .unwrap();
        for _ in 0..qa_compat::services::SIGNON_BYTES / 14 {
            (ENGINE_CALLS.static_entity)(calls.services, calls.context, EntityRecord::default())
                .unwrap();
        }
        let value = vm.spawn_entity(calls.services, calls.context).unwrap();
        let id = entity(calls, &vm, value);
        assert_eq!(vm.make_static(calls.services, calls.context, value), Ok(()));
        assert!(calls.services.server.entities.resolve(id).is_none());
        assert_eq!(calls.services.server.events.rejected_publications(), 1);
    });
}

#[test]
fn native_weapon_deadlines_publish_for_the_owning_client_with_independent_roles() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_services(rules, |calls| {
            let max_clients = calls
                .services
                .cvars
                .bind("sv_maxclients", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(max_clients, "2").unwrap();
            let client = calls
                .services
                .server
                .connect(
                    qa_session::clients::Connection::Local,
                    calls.context.module,
                    Some(NativeEntity {
                        module: calls.context.module,
                        slot: 2,
                    }),
                )
                .unwrap();
            let mut fields = FIELDS.to_vec();
            fields.push((2, 37, b"attack_finished"));
            let image = vm_with_definitions(40, GLOBALS, &fields);
            let mut vm = seed_vm(calls, image);
            let value = (2 * vm.stride) as u32;
            let canonical = calls.services.server.clients[client.0 as usize].entity;
            assert_eq!(entity(calls, &vm, value), canonical);
            assert_ne!(canonical.slot, 2);
            calls.context.console.source = RuleSetId::Quake2;
            calls.context.trace_rules = RuleSetId::Quake3;
            let player = &mut calls.services.server.clients[client.0 as usize].player;
            player.movement_rules = RuleSetId::Quake3;
            player.trace_rules = RuleSetId::Quake2Rerelease;
            assert_eq!(player.weapon_state, None);

            for bits in [
                0,
                (-3.25f32).to_bits(),
                0x8000_0000,
                0x3f80_0001,
                0xffc0_1234,
            ] {
                write(&mut vm, value, b"attack_finished", &[bits]);
                vm.project_entities(calls.services, calls.context).unwrap();
                let player = &calls.services.server.clients[client.0 as usize].player;
                let state = player.weapon_state.unwrap();
                assert_eq!(state.source, rules);
                assert_eq!(
                    state.namespace,
                    ResourceNamespace::Module(calls.context.module)
                );
                let WeaponReadiness::DeadlineSeconds(deadline) = state.readiness else {
                    panic!("native QC seconds");
                };
                assert_eq!(deadline.to_bits(), bits);
                assert_eq!(
                    (player.movement_rules, player.trace_rules),
                    (RuleSetId::Quake3, RuleSetId::Quake2Rerelease)
                );
            }

            // Exercise real StorePF dirty publication rather than direct importer setup.
            vm.image.globals[28] = 7.125f32.to_bits();
            vm.image.globals[31] = vm.field_address(value, 37, 1).unwrap() as u32;
            vm.call(calls, 6, 100, false).unwrap();
            vm.project_dirty_entities(calls.services, calls.context)
                .unwrap();
            assert_eq!(
                calls.services.server.clients[client.0 as usize]
                    .player
                    .weapon_state,
                Some(WeaponState {
                    source: rules,
                    namespace: ResourceNamespace::Module(calls.context.module),
                    readiness: WeaponReadiness::DeadlineSeconds(7.125),
                })
            );

            let other = vm.spawn_entity(calls.services, calls.context).unwrap();
            write(&mut vm, other, b"attack_finished", &[99.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                calls.services.server.clients[client.0 as usize]
                    .player
                    .weapon_state
                    .unwrap()
                    .readiness,
                WeaponReadiness::DeadlineSeconds(7.125)
            );
            calls.services.server.clients[client.0 as usize].module = ModuleId(17);
            write(&mut vm, value, b"attack_finished", &[123.0f32.to_bits()]);
            vm.project_entities(calls.services, calls.context).unwrap();
            assert_eq!(
                calls.services.server.clients[client.0 as usize]
                    .player
                    .weapon_state
                    .unwrap()
                    .readiness,
                WeaponReadiness::DeadlineSeconds(7.125)
            );
        });
    }
}

#[test]
fn a_missing_native_weapon_deadline_does_not_fabricate_zero() {
    with_services(RuleSetId::Quake, |calls| {
        let client = calls
            .services
            .server
            .connect(
                qa_session::clients::Connection::Local,
                calls.context.module,
                Some(NativeEntity {
                    module: calls.context.module,
                    slot: 1,
                }),
            )
            .unwrap();
        let mut vm = seeded_vm(calls, 8);
        assert_eq!(
            calls.services.server.clients[client.0 as usize]
                .player
                .weapon_state,
            None
        );
        vm.project_entities(calls.services, calls.context).unwrap();
        assert_eq!(
            calls.services.server.clients[client.0 as usize]
                .player
                .weapon_state,
            None
        );
    });
}
