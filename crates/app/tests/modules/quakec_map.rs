use super::*;
use qa_app::{WorldCollision, map::NativeEntityText, modules::QuakeCMap};
use qa_formats::entities::EntitySyntax;
use qa_world::collision::{
    Contents,
    brushes::{BrushTree, CollisionLeaf, ModelRoot},
    surfaces::SurfaceTable,
};

type Op = (qa_formats::program::quakec::Opcode, i16, i16, i16);

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
    (1, 40, b"classname"),
    (2, 41, b"spawnflags"),
    (2, 42, b"light_lev"),
    (2, 43, b"marker"),
    (1, 44, b"message"),
    (4, 45, b"enemy"),
    (5, 46, b"field_ref"),
    (6, 47, b"callback"),
];

fn program(bodies: &[(&[u8], &[Op])], builtins: &[i32]) -> Vm {
    let mut operations = Vec::new();
    let mut entries = vec![0];
    let mut globals = vec![
        (4, 34, b"self".as_slice()),
        (2, 35, b"time".as_slice()),
        (4, 36, b"other".as_slice()),
        (2, 37, b"frametime".as_slice()),
        (1, 38, b"mapname".as_slice()),
        (2, 39, b"serverflags".as_slice()),
        (2, 48, b"coop".as_slice()),
        (2, 49, b"deathmatch".as_slice()),
    ];
    for (index, &(name, body)) in bodies.iter().enumerate() {
        entries.push(operations.len() as i32 + 1);
        operations.extend_from_slice(body);
        globals.push((6, 200 + index as u16, name));
    }
    entries.extend(builtins.iter().map(|&builtin| -builtin));
    let bytes =
        quakec_program::program_with_definitions(&operations, &entries, &globals, FIELDS, 64, 256);
    let mut image = Image::parse(&bytes, Some(5927)).unwrap();
    for (index, &(name, _)) in bodies.iter().enumerate() {
        image.functions[index + 1].name = image.names.find(name).unwrap();
    }
    // Referenced ev_field values are native global words, not definition offsets.
    image.globals[0] = 0x1020_3040;
    let mut vm = Vm::load(
        image,
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 8192,
            state_step: 0.1,
        },
    )
    .unwrap();
    for (at, word) in [
        (90, 43), // marker
        (91, 29), // frame
        (92, 44), // message
        (93, 30), // skin
        (94, 42), // light_lev
        (95, 45), // enemy
        (96, 46), // field_ref
        (97, 47), // callback
        (98, 6),  // angles
    ] {
        vm.image.globals[at] = word;
    }
    vm.image.globals[100] = 1.0f32.to_bits();
    vm.image.globals[101] = 9.0f32.to_bits();
    vm
}

fn host_for_map(
    rules: RuleSetId,
    vm: Vm,
    text: &[u8],
    commands: &str,
    rate: TickRate,
) -> FrameHost {
    let mut runtime = Runtime::load(1, []).unwrap();
    let bounds = Bounds {
        mins: Vec3([-512.0; 3]),
        maxs: Vec3([512.0; 3]),
    };
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: Some(Contents::EMPTY),
                    brushes: IndexSpan::default(),
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0); 2],
            },
            vec![bounds; 2],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    runtime.entity_sources.push(NativeEntityText {
        syntax: EntitySyntax::Quake,
        bytes: text.into(),
    });
    // The native world identity does not borrow a same-numbered common slot.
    runtime
        .server
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap();
    let module = ModuleId(1);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[anchor.slot as usize] =
        Some(NativeEntity { module, slot: 0 });
    let frame = vm.image.function(b"StartFrame").unwrap();
    let entries = (0..vm.image.functions.len() as u32).collect();
    let request = ModuleRequest {
        context: CallContext {
            module,
            native_rules: rules,
            trace_rules: RuleSetId::Quake3,
            sound_rules: RuleSetId::Quake2,
            server_frame: 0,
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
        rate,
        anchor,
        program: Program::quakec(
            vm,
            Some(QuakeCMap {
                world_path: "maps/mixed.bsp".into(),
                entity_source: 0,
            }),
        ),
        entries,
        frame: Export::clocked(CallbackId(frame)),
        prepare: vec![],
        initialize: vec![],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: qa_core::configstrings::ConfigLayout::for_rules(rules).count,
        files: 0,
        entity_source: Some((0, EntitySyntax::Quake)),
    };
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append(
            &format!("sv_maxclients 1\nsv_mintic 0\nsv_maxtic 1\n{commands}"),
            Context {
                source: rules,
                side: qa_console::catalog::Scope::Server,
                ..Context::default()
            },
        )
        .unwrap();
    console.execute_frame(&mut runtime);
    FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![request]).unwrap()
}

fn initialize(host: &mut FrameHost) {
    assert_eq!(
        host.module_state(ModuleId(1)),
        Some(qa_app::modules::State::Pending)
    );
    host.initialize_modules(Phase::Server);
}

fn probe(host: &mut FrameHost, callback: u32) -> [u32; 3] {
    host.call_module_export(
        ModuleId(1),
        Export::clocked(CallbackId(callback)),
        ThinkTime::Seconds(7.0),
    )
    .unwrap();
    let Some(ModuleResult::QuakeC(result)) = host.module_counts(ModuleId(1)).unwrap().last_result
    else {
        panic!("not a QuakeC result");
    };
    result
}

fn entities(host: &FrameHost) -> Vec<(i32, EntityId)> {
    host.runtime
        .server
        .entities
        .active()
        .filter_map(|entity| {
            let native = host.runtime.server.entities.columns.native_entity[entity.slot as usize]?;
            (native.module == ModuleId(1)).then_some((native.slot, entity))
        })
        .collect()
}

fn prints(host: &FrameHost) -> Vec<Vec<u8>> {
    let ring = &host.runtime.server.events;
    let mut batch = ring.batch(host.runtime.server.presentation).unwrap();
    let mut prints = vec![];
    while let Some(record) = ring.next(&mut batch) {
        if let FrameEvent::Print(print) = record.event {
            prints.push(ring.texts.get(print.text).unwrap().to_vec());
        }
    }
    prints
}

#[test]
fn map_callbacks_run_immediately_with_native_reuse_and_preserved_globals() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = program(
            &[
                (b"StartFrame", &[(Return, 1, 0, 0)]),
                (
                    b"worldspawn",
                    &[
                        (Call0, 110, 0, 0),
                        (StoreEnt, 1, 80, 0),
                        (StoreEnt, 80, 4, 0),
                        (Call1, 111, 0, 0),
                        (StoreEnt, 80, 36, 0),
                        (StoreF, 101, 35, 0),
                        (Return, 1, 0, 0),
                    ],
                ),
                (
                    b"entry",
                    &[
                        (StoreEnt, 34, 81, 0),
                        (StoreF, 35, 82, 0),
                        (StoreEnt, 36, 83, 0),
                        (StoreEnt, 34, 4, 0),
                        (StoreS, 112, 7, 0),
                        (Call2, 113, 0, 0),
                        (LoadF, 34, 90, 84),
                        (Address, 34, 91, 85),
                        (StorePF, 84, 85, 0),
                        (Return, 1, 0, 0),
                    ],
                ),
                (b"probe", &[(Return, 81, 0, 0)]),
            ],
            &[14, 15, 3],
        );
        let stride = vm.stride as u32;
        vm.image.globals[110] = 5;
        vm.image.globals[111] = 6;
        vm.image.globals[112] = vm.insert_string(b"*1").unwrap() as u32;
        vm.image.globals[113] = 7;
        let mut host = host_for_map(
            rules,
            vm,
            b"{ classname worldspawn } { classname entry marker 17 }",
            "",
            TickRate::fixed(100).unwrap(),
        );
        initialize(&mut host);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Running)
        );
        let first = if rules == RuleSetId::Quake { 2 } else { 33 };
        assert_eq!(
            probe(&mut host, 4),
            [first * stride, 9.0f32.to_bits(), first * stride]
        );
        let rows = entities(&host);
        assert_eq!(rows.len(), 2);
        let entry = rows
            .iter()
            .find(|(native, _)| *native == first as i32)
            .unwrap()
            .1;
        assert_eq!(
            host.runtime.server.entities.columns.appearance[entry.slot as usize].frame,
            17
        );
        assert_eq!(
            host.runtime.server.entities.columns.appearance[entry.slot as usize].models[0],
            ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 2)
        );
        let world = rows.iter().find(|(native, _)| *native == 0).unwrap().1;
        assert_ne!(world.slot, 0);
        assert_eq!(
            host.runtime.server.entities.columns.appearance[world.slot as usize].models[0],
            ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 1)
        );
        assert_eq!(
            host.model_precaches(),
            vec![(rules, b"maps/mixed.bsp".as_slice())]
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn typed_map_fields_keep_native_strings_aliases_and_nonlive_entity_references() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut vm = program(
            &[
                (b"StartFrame", &[(Return, 1, 0, 0)]),
                (b"worldspawn", &[(Return, 1, 0, 0)]),
                (
                    b"entry",
                    &[
                        (StoreEnt, 34, 85, 0),
                        (LoadS, 34, 92, 4),
                        (Call1, 110, 0, 0),
                        (LoadF, 34, 93, 80),
                        (LoadF, 34, 94, 81),
                        (LoadEnt, 34, 95, 82),
                        (Return, 1, 0, 0),
                    ],
                ),
                (b"probe", &[(Return, 80, 0, 0)]),
                (
                    b"refs",
                    &[
                        (LoadFld, 85, 96, 80),
                        (LoadFnc, 85, 97, 81),
                        (Return, 80, 0, 0),
                    ],
                ),
                (b"callback", &[(Return, 1, 0, 0)]),
            ],
            &[25],
        );
        let stride = vm.stride as u32;
        vm.image.globals[110] = 7;
        let mut host = host_for_map(
            rules,
            vm,
            b"{ classname worldspawn } { classname entry origin \"12 0 -3\" angle 75 light 2.75junk \"skin \" 7 enemy 60 field_ref origin callback callback message \"raw\x80\\nend\" _comment ignored unknown_field ignored }",
            "developer 1\n",
            TickRate::fixed(100).unwrap(),
        );
        initialize(&mut host);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Running)
        );
        assert_eq!(
            probe(&mut host, 4),
            [
                if rules == RuleSetId::Quake {
                    7.0f32.to_bits()
                } else {
                    0
                },
                2.75f32.to_bits(),
                60 * stride
            ]
        );
        assert_eq!(probe(&mut host, 5)[..2], [0x1020_3040, 6]);
        let first = if rules == RuleSetId::Quake { 2 } else { 33 };
        let entry = entities(&host)
            .into_iter()
            .find(|(native, _)| *native == first)
            .unwrap()
            .1;
        let columns = &host.runtime.server.entities.columns;
        assert_eq!(
            columns.position[entry.slot as usize],
            Vec3([12.0, 0.0, -3.0])
        );
        assert_eq!(columns.angles[entry.slot as usize], Vec3([0.0, 75.0, 0.0]));
        assert!(prints(&host).iter().any(|text| text == b"raw\x80\nend"));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn native_map_epoch_and_startframe_order_survive_the_platform_epoch() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let vm = program(
            &[
                (
                    b"StartFrame",
                    &[
                        (AddF, 70, 100, 70),
                        (StoreF, 35, 80, 0),
                        (StoreF, 37, 81, 0),
                        (StoreF, 70, 82, 0),
                        (Return, 80, 0, 0),
                    ],
                ),
                (
                    b"worldspawn",
                    &[
                        (StoreF, 70, 71, 0),
                        (StoreF, 35, 72, 0),
                        (StoreEnt, 36, 73, 0),
                        (Return, 1, 0, 0),
                    ],
                ),
                (b"probe", &[(Return, 71, 0, 0)]),
            ],
            &[],
        );
        let mut host = host_for_map(
            rules,
            vm,
            b"{ classname worldspawn }",
            "",
            TickRate::fixed(100).unwrap(),
        );
        initialize(&mut host);
        assert_eq!(
            probe(&mut host, 3),
            [
                if rules == RuleSetId::QuakeWorld {
                    1.0f32.to_bits()
                } else {
                    0
                },
                1.0f32.to_bits(),
                0
            ]
        );
        for milliseconds in [100_000, 100_100, 100_200] {
            host.frame(
                &mut Source {
                    time: EventTime::from_milliseconds(milliseconds),
                    polls: 0,
                },
                true,
            );
        }
        let Some(ModuleResult::QuakeC(result)) =
            host.module_counts(ModuleId(1)).unwrap().last_result
        else {
            panic!("missing frame result")
        };
        let expected_time = if rules == RuleSetId::Quake {
            1.1f32
        } else {
            (1.0f64 + 2.0 * f64::from(0.1f32)) as f32
        };
        assert_eq!(
            result,
            [
                expected_time.to_bits(),
                0.1f32.to_bits(),
                if rules == RuleSetId::Quake {
                    2.0f32.to_bits()
                } else {
                    3.0f32.to_bits()
                }
            ]
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn independently_queued_spans_keep_native_qw_float_narrowing() {
    for (rules, times) in [
        (RuleSetId::Quake, [0x3f80_0000, 0x3f81_948b]),
        (RuleSetId::QuakeWorld, [0x3f81_948b, 0x3f82_f1ac]),
    ] {
        let vm = program(
            &[
                (
                    b"StartFrame",
                    &[(StoreF, 35, 80, 0), (StoreF, 37, 81, 0), (Return, 80, 0, 0)],
                ),
                (b"worldspawn", &[(Return, 1, 0, 0)]),
            ],
            &[],
        );
        let mut host = host_for_map(
            rules,
            vm,
            b"{ classname worldspawn }",
            "",
            TickRate::FrameDriven,
        );
        initialize(&mut host);
        let mut now = 90_000_000_000u64;
        host.frame(
            &mut Source {
                time: EventTime(now),
                polls: 0,
            },
            true,
        );
        for ((span, frame), time) in [(12_345_678, 0x3c4a_4587), (10_654_622, 0x3c2e_90b9)]
            .into_iter()
            .zip(times)
        {
            now += span;
            host.frame(
                &mut Source {
                    time: EventTime(now),
                    polls: 0,
                },
                true,
            );
            let Some(ModuleResult::QuakeC(result)) =
                host.module_counts(ModuleId(1)).unwrap().last_result
            else {
                panic!("missing native frame result")
            };
            assert_eq!(result[..2], [time, frame]);
        }
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn nq_framedriven_duration_clamps_stalls_but_preserves_positive_override() {
    for (commands, duration, second_time) in [
        ("host_framerate 0\n", 0.1f32, 1.1f32),
        ("host_framerate 0.25\n", 0.25f32, 1.25f32),
    ] {
        let vm = program(
            &[
                (
                    b"StartFrame",
                    &[(StoreF, 35, 80, 0), (StoreF, 37, 81, 0), (Return, 80, 0, 0)],
                ),
                (b"worldspawn", &[(Return, 1, 0, 0)]),
            ],
            &[],
        );
        let mut host = host_for_map(
            RuleSetId::Quake,
            vm,
            b"{ classname worldspawn }",
            commands,
            TickRate::FrameDriven,
        );
        initialize(&mut host);
        for (milliseconds, expected_time) in
            [(1000, None), (1200, Some(1.0)), (1400, Some(second_time))]
        {
            host.frame(
                &mut Source {
                    time: EventTime::from_milliseconds(milliseconds),
                    polls: 0,
                },
                true,
            );
            if let Some(expected_time) = expected_time {
                let Some(ModuleResult::QuakeC(result)) =
                    host.module_counts(ModuleId(1)).unwrap().last_result
                else {
                    panic!("missing native frame result")
                };
                assert_eq!(result[..2], [expected_time.to_bits(), duration.to_bits()]);
            }
        }
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn map_inhibition_and_missing_classnames_free_rows_before_the_next_spawn() {
    for (rules, commands, flag, kept) in [
        (RuleSetId::Quake, "skill 0\n", 256, false),
        (RuleSetId::Quake, "skill 1\n", 256, true),
        (RuleSetId::Quake, "deathmatch 1\n", 2048, false),
        (
            RuleSetId::QuakeWorld,
            "deathmatch 0\nskill 0\n",
            2048,
            false,
        ),
        (RuleSetId::QuakeWorld, "deathmatch 0\nskill 0\n", 256, true),
    ] {
        let vm = program(
            &[
                (b"StartFrame", &[(Return, 1, 0, 0)]),
                (b"worldspawn", &[(Return, 1, 0, 0)]),
                (b"entry", &[(AddF, 70, 100, 70), (Return, 1, 0, 0)]),
                (b"probe", &[(Return, 70, 0, 0)]),
            ],
            &[],
        );
        let text = format!(
            "{{ classname worldspawn }} {{ classname entry spawnflags {flag} }} {{ marker 9 }} {{ classname missing }} {{ classname ENTRY }} {{ classname entry }}"
        );
        let mut host = host_for_map(
            rules,
            vm,
            text.as_bytes(),
            commands,
            TickRate::fixed(100).unwrap(),
        );
        initialize(&mut host);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Running)
        );
        assert_eq!(
            probe(&mut host, 4)[0],
            (if kept { 2.0f32 } else { 1.0f32 }).to_bits()
        );
        let first = if rules == RuleSetId::Quake { 2 } else { 33 };
        let mut native: Vec<_> = entities(&host)
            .into_iter()
            .map(|(native, _)| native)
            .collect();
        native.sort_unstable();
        assert_eq!(
            native,
            if kept {
                vec![0, first, first + 1]
            } else {
                vec![0, first]
            }
        );
        assert!(
            prints(&host)
                .iter()
                .any(|text| text.windows(5).any(|text| text == b"spawn"))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn nq_startup_assigns_only_the_selected_native_mode_global() {
    for (commands, expected, effective) in [
        (
            "coop 0\ndeathmatch 1\n",
            [17.0f32, 1.0f32],
            [0.0f32, 1.0f32],
        ),
        (
            "deathmatch 1\ncoop 1\n",
            [1.0f32, 29.0f32],
            [1.0f32, 0.0f32],
        ),
    ] {
        let mut vm = program(
            &[
                (b"StartFrame", &[(Return, 1, 0, 0)]),
                (
                    b"worldspawn",
                    &[(StoreF, 48, 80, 0), (StoreF, 49, 81, 0), (Return, 1, 0, 0)],
                ),
                (b"probe", &[(Return, 80, 0, 0)]),
            ],
            &[],
        );
        vm.image.globals[48] = 17.0f32.to_bits();
        vm.image.globals[49] = 29.0f32.to_bits();
        let mut host = host_for_map(
            RuleSetId::Quake,
            vm,
            b"{ classname worldspawn }",
            commands,
            TickRate::FrameDriven,
        );
        let context = Context {
            source: RuleSetId::Quake,
            side: qa_console::catalog::Scope::Server,
            role: qa_console::views::Role::Game,
            ..Context::default()
        };
        let selected = ["coop", "deathmatch"].map(|name| {
            let view = host.console.cvars.bind(name, context).unwrap();
            host.console.cvars.numeric(view).unwrap()
        });
        assert_eq!(selected, effective);
        initialize(&mut host);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Running)
        );
        assert_eq!(probe(&mut host, 3)[..2], expected.map(f32::to_bits));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn map_parse_or_spawn_failure_never_reports_running_or_executes_later_spawns() {
    for (text, unknown) in [
        (
            b"{ classname worldspawn } { classname broken } { classname entry }".as_slice(),
            Some(999),
        ),
        (
            b"{ classname worldspawn } { classname entry callback missing } { classname entry }"
                .as_slice(),
            None,
        ),
    ] {
        let mut vm = program(
            &[
                (b"StartFrame", &[(Return, 1, 0, 0)]),
                (b"worldspawn", &[(Return, 1, 0, 0)]),
                (b"broken", &[(Call0, 110, 0, 0), (Return, 1, 0, 0)]),
                (b"entry", &[(AddF, 70, 100, 70), (Return, 1, 0, 0)]),
            ],
            &[999],
        );
        vm.image.globals[110] = 5;
        let mut host = host_for_map(
            RuleSetId::Quake,
            vm,
            text,
            "",
            TickRate::fixed(100).unwrap(),
        );
        initialize(&mut host);
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(qa_app::modules::State::Failed)
        );
        assert_eq!(host.last_unknown_call(ModuleId(1)), unknown);
        if let Some(number) = unknown {
            assert!(
                prints(&host).iter().any(|text| text
                    == format!("module 1: unknown QuakeC builtin {number}\n").as_bytes())
            );
        }
        let calls = host.module_counts(ModuleId(1)).unwrap().calls;
        for milliseconds in [0, 100, 200] {
            host.frame(
                &mut Source {
                    time: EventTime::from_milliseconds(milliseconds),
                    polls: 0,
                },
                true,
            );
        }
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    }
}
