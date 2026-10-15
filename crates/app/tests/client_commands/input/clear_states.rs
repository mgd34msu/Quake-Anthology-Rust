use super::*;

const NEST: u32 = 80;
const AFTER_INNER: u32 = 84;
const AFTER_ROOT: u32 = 88;
const START: u32 = 900;
const UI_UP: u32 = 916;
const CG_UP: u32 = 928;
const INNER_DONE: u32 = 944;
const END: u32 = 960;

fn mark(ui: bool, offset: u32) -> Op {
    call(u32::from(ui), &[Value::Pointer(offset)], None)
}

fn customize(seat: usize, ui: bool, bodies: &mut Vec<Vec<Op>>, cross_module: bool) {
    use Value::*;
    if seat != 0 {
        return;
    }
    let key = if ui { 3 } else { 6 };
    let mut release = vec![mark(ui, if ui { UI_UP } else { CG_UP })];
    if ui {
        release.push(Op::IfEqual(
            Arg(0),
            Constant(97),
            vec![Op::IfEqual(
                Word(NEST),
                Constant(1),
                vec![
                    Op::Write(NEST, Constant(0)),
                    call(
                        41,
                        &[Constant(if cross_module {
                            key_catchers::CGAME
                        } else {
                            key_catchers::UI
                        })],
                        None,
                    ),
                    call(39, &[], None),
                    Op::Write(AFTER_INNER, Word(KEY_COUNT)),
                    mark(true, INNER_DONE),
                ],
            )],
        ));
    }
    let at = bodies[key].len() - 1;
    bodies[key].insert(at, Op::IfEqual(Arg(1), Constant(0), release));
    if ui {
        bodies[26] = vec![
            mark(true, START),
            call(39, &[], None),
            Op::Write(AFTER_ROOT, Word(KEY_COUNT)),
            mark(true, END),
            Op::Return(Constant(0)),
        ];
    }
}

fn seed_text(host: &mut FrameHost) {
    for (offset, bytes) in [
        (START, b"clear-start\0".as_slice()),
        (UI_UP, b"ui-up\0"),
        (CG_UP, b"cg-up\0"),
        (INNER_DONE, b"inner-done\0"),
        (END, b"clear-end\0"),
    ] {
        for (word, chunk) in bytes.chunks(4).enumerate() {
            let mut bytes = [0; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            for ui in [true, false] {
                write(
                    host,
                    0,
                    ui,
                    offset + word as u32 * 4,
                    u32::from_le_bytes(bytes),
                );
            }
        }
    }
}

fn nested_clear(native: bool, cgame_native: bool, cross_module: bool) {
    let mut host = fixture_roles(
        native,
        cgame_native,
        |seat, ui, bodies| {
            customize(seat, ui, bodies, cross_module);
            vec![]
        },
        |_| {},
    );
    seed_text(&mut host);
    let mut source = Source::default();
    catch(&mut host, 0, true, key_catchers::UI);
    catch(&mut host, 1, true, key_catchers::UI);
    frame(
        &mut host,
        &mut source,
        vec![
            key(DeviceId::Keyboard(10), 4, true, false),
            key(DeviceId::Keyboard(10), 5, true, false),
            key(DeviceId::Keyboard(10), 6, true, false),
            key(DeviceId::Keyboard(11), 4, true, false),
        ],
        vec![],
    );
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), 3);
    write(&mut host, 0, true, NEST, 1);
    if cross_module {
        // Cgame changes the catcher for the next key; it has no ClearStates
        // import. The nested UI clear therefore dispatches CG then UI again.
        write(&mut host, 0, false, SWITCH_KEY, 98);
        write(&mut host, 0, false, SWITCH_DOWN, 0);
        write(&mut host, 0, false, SWITCH_MASK, key_catchers::UI);
    }
    let polls = source.polls;
    assert_eq!(invoke(&mut host, 0, true, 26, &[]), 0);
    assert_eq!(source.polls, polls, "synthetic releases do not poll the OS");
    let expected = if cross_module { 5 } else { 6 };
    assert_eq!(read(&mut host, 0, true, AFTER_INNER), expected);
    assert_eq!(read(&mut host, 0, true, AFTER_ROOT), expected);
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), expected);
    assert_eq!(
        read(&mut host, 0, false, KEY_COUNT),
        u32::from(cross_module)
    );
    assert_eq!(read(&mut host, 0, true, LAST_KEY), 99);
    assert_eq!(read(&mut host, 0, true, LAST_DOWN), 0);
    assert_eq!(read(&mut host, 0, true, HELD_KEY), 0);
    for key in [97, 98, 99] {
        assert_eq!(invoke(&mut host, 0, true, 23, &[key]), 0);
    }
    assert_eq!(invoke(&mut host, 1, true, 23, &[97]), 1);
    assert_eq!(read(&mut host, 1, true, KEY_COUNT), 1);
    let prints = presented_prints(&host);
    let texts: Vec<_> = prints.iter().map(|(_, _, text)| text.as_slice()).collect();
    assert_eq!(
        texts,
        [
            b"clear-start".as_slice(),
            b"ui-up",
            if cross_module { b"cg-up" } else { b"ui-up" },
            b"ui-up",
            b"inner-done",
            b"clear-end",
        ]
    );
    assert_eq!(host.module_counts(module(0, true)).unwrap().traps, 0);
    assert_eq!(host.module_counts(module(0, false)).unwrap().traps, 0);
}

pub(super) fn run(native: bool) {
    nested_clear(native, native, false);
    nested_clear(native, native, true);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    nested_clear(native, !native, true);
    clear_during_initialization(native);
    scene_survives_parent_resume(native);
    failed_release_finishes_cleanup(native);
    println!("synchronous clear-state fixtures passed: native={native}");
}

fn failed_release_finishes_cleanup(native: bool) {
    let mut host = fixture(native);
    let mut source = Source::default();
    catch(&mut host, 0, true, key_catchers::UI);
    catch(&mut host, 1, true, key_catchers::UI);
    frame(
        &mut host,
        &mut source,
        vec![
            key(DeviceId::Keyboard(10), 4, true, false),
            key(DeviceId::Keyboard(10), 5, true, false),
            key(DeviceId::Keyboard(10), 6, true, false),
        ],
        vec![],
    );
    write(&mut host, 0, true, FAIL, 1);
    assert!(
        host.call_module_export(
            module(0, true),
            Export::words(CallbackId(26), &[]),
            ThinkTime::Milliseconds(9),
        )
        .is_err()
    );
    for control in [4, 5, 6] {
        assert!(!host.runtime.input.is_down(SeatId::FIRST, control));
    }
    assert_eq!(
        host.module_state(module(0, true)),
        Some(qa_app::modules::State::Failed)
    );
    let provider = host.module_counts(ModuleId(5)).unwrap().calls;
    let result = frame(
        &mut host,
        &mut source,
        vec![key(DeviceId::Keyboard(11), 4, true, false)],
        vec![],
    );
    assert!(result.server_ticks > 0);
    assert!(host.module_counts(ModuleId(5)).unwrap().calls > provider);
    assert_eq!(read(&mut host, 1, true, KEY_COUNT), 1);
    assert_eq!(read(&mut host, 1, true, HELD_KEY), 1);
    assert_eq!(host.module_counts(module(1, true)).unwrap().traps, 0);
}

fn add_light(ui: bool, radius: f32) -> Op {
    use Value::*;
    call(
        if ui { 24 } else { 43 },
        &[
            Pointer(192),
            Constant(radius.to_bits()),
            Constant(1.0f32.to_bits()),
            Constant(0),
            Constant(0),
        ],
        None,
    )
}

fn scene_survives_parent_resume(native: bool) {
    use Value::*;
    let mut host = fixture_with(native, |seat, ui, bodies| {
        if seat != 0 {
            return;
        }
        if ui {
            bodies[26] = vec![
                add_light(true, 10.0),
                call(39, &[], None),
                add_light(true, 40.0),
                call(25, &[Pointer(256)], None),
                Op::Return(Constant(0)),
            ];
        } else {
            let at = bodies[6].len() - 1;
            bodies[6].insert(
                at,
                Op::IfEqual(
                    Arg(1),
                    Constant(0),
                    vec![
                        add_light(false, 20.0),
                        call(44, &[Pointer(256)], None),
                        call(40, &[], None),
                        add_light(false, 30.0),
                    ],
                ),
            );
        }
    });
    for ui in [true, false] {
        for at in (256..624).step_by(4) {
            write(&mut host, 0, ui, at, 0);
        }
        for (at, value) in [(8, 64), (12, 64), (76, 1)] {
            write(&mut host, 0, ui, 256 + at, value);
        }
        for (at, value) in [(16, 90.0f32), (20, 75.0), (36, 1.0), (52, 1.0), (68, 1.0)] {
            write(&mut host, 0, ui, 256 + at, value.to_bits());
        }
        for at in [192, 196, 200] {
            write(&mut host, 0, ui, at, 0);
        }
    }
    let profile = qa_app::model_assets::ModelRequest::native("", RuleSetId::Quake3, None).profile();
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    let mut source = Source::default();
    catch(&mut host, 0, false, key_catchers::CGAME);
    frame(
        &mut host,
        &mut source,
        vec![key(DeviceId::Keyboard(10), 5, true, false)],
        vec![],
    );
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits::default(),
            qa_render::Assets::load().unwrap(),
            vec![],
        )
        .unwrap()
        .0,
    );
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(invoke(&mut host, 0, true, 26, &[]), 0);
    let scene = host.runtime.scene.as_mut().unwrap();
    let packet = scene.finish_frame().unwrap();
    let views: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            qa_render::scene::Command::View(view) => Some(view),
            _ => None,
        })
        .collect();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].scene.lights.count, 1);
    assert_eq!(views[1].scene.lights.count, 2);
    assert_eq!(packet.lights(views[0].scene.lights)[0].radius, 20.0);
    let lights = views[1].scene.lights;
    assert_eq!(packet.lights(lights)[0].radius, 30.0);
    assert_eq!(packet.lights(lights)[1].radius, 40.0);
    assert_eq!(packet.rejected, 0);
    assert!(scene.recycle(packet).is_ok());
}

fn clear_during_initialization(native: bool) {
    use Value::*;
    struct HeldOnly;
    impl qa_input::Target for HeldOnly {
        fn key(&mut self, _: SeatId, _: u16, _: bool, _: bool) -> bool {
            true
        }
        fn character(&mut self, _: SeatId, _: char) {}
        fn command(
            &mut self,
            _: SeatId,
            _: EventTime,
            _: Option<qa_core::sys_events::KeySource>,
            _: Option<qa_core::primitives::CommandProducer>,
            _: &str,
        ) -> bool {
            false
        }
        fn cancel_commands(
            &mut self,
            _: qa_core::sys_events::KeySource,
            _: Option<qa_core::primitives::CommandProducer>,
        ) {
        }
    }
    let mut host = fixture_config(
        native,
        |seat, ui, bodies| {
            if seat == 0 && ui {
                bodies[31] = vec![
                    call(41, &[Constant(key_catchers::UI)], None),
                    call(39, &[], None),
                    Op::Write(AFTER_ROOT, Word(KEY_COUNT)),
                    Op::Return(Constant(0)),
                ];
                vec![Export::words(CallbackId(31), &[])]
            } else {
                vec![]
            }
        },
        |host| {
            assert_eq!(
                host.module_state(module(0, true)),
                Some(qa_app::modules::State::Pending)
            );
            host.runtime.input.dispatch(
                SysEvent {
                    time: EventTime::from_milliseconds(5),
                    kind: key(DeviceId::Keyboard(10), 4, true, false),
                },
                &mut HeldOnly,
            );
        },
    );
    assert_eq!(
        host.module_state(module(0, true)),
        Some(qa_app::modules::State::Running)
    );
    assert_eq!(read(&mut host, 0, true, AFTER_ROOT), 1);
    assert_eq!(read(&mut host, 0, true, KEY_COUNT), 1);
    assert_eq!(read(&mut host, 0, true, LAST_KEY), 97);
    assert_eq!(read(&mut host, 0, true, LAST_DOWN), 0);
    assert_eq!(read(&mut host, 0, true, HELD_KEY), 0);
}
