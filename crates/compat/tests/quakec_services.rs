#[path = "support/quakec_program.rs"]
mod quakec_program;

use qa_compat::{
    abi::{QUAKEC, QuakeCCalls, UnknownCalls},
    quakec::{Builtins, Layout, Trap, Vm},
    services::{
        CallContext, CallError, ENGINE_CALLS, EngineServices, ModelLoader, ModelLookup,
        ServiceStorage,
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
fn vm(capacity: usize) -> Vm {
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
        ],
        &[0, 1, 2, 5, 8, -16, 11, 13, 15],
        GLOBALS,
        FIELDS,
        36,
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
    mut model_loader: Option<&mut Models>,
    run: impl FnOnce(&mut QuakeCCalls<'_, '_>),
) {
    let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
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
    let config_count =
        qa_core::configstrings::ConfigLayout::for_rules(rules).map_or(512, |layout| layout.count);
    let mut storage =
        ServiceStorage::load_native(&[(ModuleId(1), config_count, rules)], 0, &cvars).unwrap();
    let mut geometry = CollisionStore::new();
    let mut tree = BrushTree::direct(0).unwrap();
    tree.models
        .push(qa_world::collision::brushes::ModelRoot::Leaf(0));
    let world = geometry
        .load_brushes(
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
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
        vfs: &vfs,
        model_loader: model_loader
            .as_deref_mut()
            .map(|loader| loader as &mut dyn ModelLoader),
        model_lookup: None,
        localization: &localization,
        registry: &registry,
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
            sound_rules: rules,
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
            calls.context.sound_rules = RuleSetId::Quake3;
            let consumer = calls
                .services
                .server
                .events
                .bind(qa_core::events::OutputTarget::Client(ClientId(0)))
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
            assert_eq!(event.spatial_rules, RuleSetId::Quake3);
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
    let mut vm = vm(if calls.context.native_rules == RuleSetId::QuakeWorld {
        capacity + 31
    } else {
        capacity
    });
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
    vm.seed_entities(&calls.services.server.entities, calls.context.module)
        .unwrap();
    vm
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
            let bits = |body: qa_core::primitives::Body| {
                [body.position, body.velocity, body.mins, body.maxs]
                    .map(|value| value.0.map(f32::to_bits))
            };
            assert_eq!(bits(after), bits(body));
            assert_eq!(calls.services.server.area.bounds(id), bounds);
            assert_eq!(
                vm.entities.read(u64::from(value), vm.stride).unwrap(),
                bytes
            );
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
fn spatial_queries_flush_storep_and_state_without_scanning_untouched_rows() {
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
        store_and_trace(calls, &mut vm, value, b"origin", &[12.0f32.to_bits(), 0, 0]).unwrap();
        assert_eq!(
            calls.services.server.entities.columns.position[id.slot as usize],
            Vec3([12.0, 0.0, 0.0])
        );
        calls.call(&mut vm, 16, 4).unwrap();
        assert_eq!(
            vm.project_entities(calls.services, calls.context),
            Err(CallError::Entity)
        );
        write(&mut vm, untouched, b"mins", &[0; 3]);
        vm.project_entities(calls.services, calls.context).unwrap();
        write(&mut vm, untouched, b"mins", &[4.0f32.to_bits(); 3]);
        vm.image.globals[34] = value;
        vm.image.globals[35] = 3.0f32.to_bits();
        vm.image.globals[28] = 37.0f32.to_bits();
        vm.image.globals[29] = 0x0100_0001;
        vm.image.globals[32] = 5;
        vm.call(calls, 4, 100, false).unwrap();
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
        assert_eq!(
            calls.services.server.entities.columns.position[second_id.slot as usize],
            Vec3([15.0, 0.0, 0.0])
        );
        write(&mut vm, second, b"origin", &[17.0f32.to_bits(), 0, 0]);
        // Native pointers can be unaligned. This single word touches both rows.
        vm.image.globals[28] = 0;
        vm.image.globals[31] = first + vm.stride as u32 - 2;
        vm.call(calls, 3, 100, false).unwrap();
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
            assert_eq!(
                store_and_trace(
                    calls,
                    &mut vm,
                    first,
                    b"origin",
                    &[(x as f32).to_bits(), 0, 0]
                ),
                Err(Trap::Builtin)
            );
        }
        write(&mut vm, second, b"mins", &[0; 3]);
        calls.call(&mut vm, 16, 4).unwrap();
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
fn native_staging_preserves_full_wire_capacity_with_one_or_dense_cues() {
    use qa_core::events::{FrameEvent, NativeReceipt, OutputSubmission, OutputTarget};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for cue_count in [1, NATIVE_CUE_CAPACITY] {
            with_services(rules, |calls| {
                let consumer = calls
                    .services
                    .server
                    .events
                    .bind(OutputTarget::Client(ClientId(0)))
                    .unwrap();
                let mut wire = vec![6; NATIVE_WIRE_BYTES]; // native svc_nop after the recognized prefix
                for cue in wire[..cue_count * NATIVE_CUE_MIN_WIRE_BYTES].chunks_exact_mut(3) {
                    cue.copy_from_slice(&[9, 0, 1]);
                }
                for &byte in &wire {
                    (ENGINE_CALLS.message_bits)(calls.services, ModuleId(1), u32::from(byte), 8)
                        .unwrap();
                }
                assert_eq!(
                    (ENGINE_CALLS.message_bits)(calls.services, ModuleId(1), 6, 8),
                    Err(CallError::Capacity)
                );
                let sequence = (ENGINE_CALLS.native_message)(
                    calls.services,
                    NativeDelivery {
                        module: ModuleId(1),
                        source: rules,
                        target: NativeTarget::Client(ClientId(0)),
                        origin: Vec3::default(),
                        reliable: true,
                        dupe_key: 0,
                        check_portals: false,
                    },
                )
                .unwrap()
                .unwrap();
                let ring = &mut calls.services.server.events;
                let mut batch = ring.batch(consumer).unwrap();
                let FrameEvent::Native(payload) = ring.next(&mut batch).unwrap().event else {
                    panic!("native payload")
                };
                let mut copied = vec![0; NATIVE_PAYLOAD_BYTES];
                let bytes = ring.texts.copy_payload(payload, &mut copied).unwrap();
                assert_eq!(&bytes[..NATIVE_WIRE_BYTES], wire);
                assert_eq!(
                    bytes.len(),
                    NATIVE_WIRE_BYTES + cue_count * NATIVE_CUE_CAPTURE_BYTES
                );
                for offset in [0, (cue_count - 1) * NATIVE_CUE_MIN_WIRE_BYTES] {
                    assert!(payload.cue(bytes, offset).is_some());
                }
                assert_eq!(ring.rejected_publications(), 0);
                assert!(ring.submit(
                    consumer,
                    sequence,
                    OutputSubmission::Reliable(NativeReceipt(71))
                ));
                assert_eq!(ring.acknowledge(consumer, NativeReceipt(70)), 0);
                assert!(ring.texts.copy_payload(payload, &mut copied).is_some());
                assert_eq!(ring.acknowledge(consumer, NativeReceipt(71)), 1);
                assert!(ring.texts.copy_payload(payload, &mut copied).is_none());
            });
        }
    }
}
