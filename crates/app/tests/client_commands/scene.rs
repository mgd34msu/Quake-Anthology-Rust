#[path = "../../../render/tests/support/world_surfaces.rs"]
mod mark_world;
use super::*;
use qa_app::model_assets::{ModelAssets, ModelRequest};
use qa_compat::services::{ModelLoader, ModelLookup};
use qa_render::material::world_load::{
    LoadedWorld, LoadedWorldPresentation, WorldPresentationProfile,
};
use qa_render::scene::Command;

fn admit_assets(host: &mut FrameHost) -> qa_render::cpu::CpuBackend {
    let backend = qa_render::cpu::CpuBackend::load_with_assets(
        320,
        200,
        host.runtime.model_assets.as_ref().unwrap().assets(),
    )
    .unwrap();
    host.publish_model_admission(backend.admitted_assets().unwrap())
        .unwrap();
    backend
}

const CALLS: &[(u8, &[u32])] = &[
    (39, &[16]),
    (45, &[32]),
    (87, &[1, 3, 64, 2]),
    (44, &[256]),
    (
        46,
        &[
            (-10.0f32).to_bits(),
            0,
            100.0f32.to_bits(),
            30.0f32.to_bits(),
            0,
            0,
            1.0f32.to_bits(),
            1.0f32.to_bits(),
            1,
        ],
    ),
    (40, &[]),
    (36, &[224]),
    (87, &[1, 3, 65528, 2]),
    (45, &[0]),
    (87, &[0, 3, u32::MAX, 2]),
];

const MARK_CALL: (u8, &[u32]) = (27, &[4, 640, 688, 16, 32, 8, 940]);
fn scene_call(command: usize) -> (u8, &'static [u32]) {
    if command == 20 {
        MARK_CALL
    } else {
        CALLS[command]
    }
}

fn data() -> [u8; 1024] {
    let mut data = [0; 1024];
    data[16..27].copy_from_slice(b"ui/pic.tga\0");
    data[224..239].copy_from_slice(b"maps/probe.bsp\0");
    for (index, value) in [0.5f32, 0.25, 1., 0.75].into_iter().enumerate() {
        data[32 + index * 4..36 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    for index in 0..6 {
        let first = 64 + index * 24;
        for (axis, value) in [8.0, index as f32, (index % 3) as f32]
            .into_iter()
            .enumerate()
        {
            data[first + axis * 4..first + axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[first + 20..first + 24].copy_from_slice(&[index as u8, 20, 30, 255]);
    }
    for (at, value) in [(8, 160i32), (12, 200), (72, -12345), (76, 1)] {
        data[256 + at..260 + at].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(16, 95.0f32), (20, 75.), (36, 1.), (52, 1.), (68, 1.)] {
        data[256 + at..260 + at].copy_from_slice(&value.to_le_bytes());
    }
    data[336..368].fill(0xa5);
    for row in 0..8 {
        data[368 + row * 32 + 1] = 0xe0 + row as u8;
    }
    for (index, point) in [[0f32, -2., -2.], [0., 2., -2.], [0., 2., 2.], [0., -2., 2.]]
        .into_iter()
        .enumerate()
    {
        for (axis, coordinate) in point.into_iter().enumerate() {
            let at = 640 + index * 12 + axis * 4;
            data[at..at + 4].copy_from_slice(&coordinate.to_le_bytes());
        }
    }
    data[688..692].copy_from_slice(&2f32.to_le_bytes());
    data[880..896].copy_from_slice(b"models/test.md3\0");
    data[900..917].copy_from_slice(b"models/test.skin\0");
    data[920..923].copy_from_slice(b"*1\0");
    for (offset, value) in [
        (28, 1.0f32),
        (44, 1.),
        (60, 1.),
        (68, 8.),
        (100, 0.25),
        (120, 0.125),
        (124, 0.5),
        (128, 1.25),
        (132, 2.),
        (136, 45.),
    ] {
        data[700 + offset..704 + offset].copy_from_slice(&value.to_le_bytes());
    }
    data[780..784].copy_from_slice(&1i32.to_le_bytes());
    data[812..816].copy_from_slice(&1i32.to_le_bytes());
    data[816..820].copy_from_slice(&[201, 150, 90, 255]);
    data
}

fn pointer_argument(command: usize, index: usize) -> bool {
    matches!(
        (command, index),
        (0 | 1 | 3 | 6 | 8 | 12 | 13, 0) | (2 | 7 | 9, 2) | (20, 1 | 2 | 4 | 6)
    )
}
fn ordinal(ui: bool, value: u8) -> u8 {
    if !ui {
        return value;
    }
    match value {
        39 => 20,
        45 => 26,
        87 => 23,
        44 => 25,
        46 => 27,
        40 => 21,
        37 => 18,
        38 => 19,
        41 => 22,
        other => other,
    }
}
fn qvm_scene(ui: bool) -> Program {
    use Opcode::*;
    let mut operations = vec![(Enter, 96)];
    let mut branches = vec![];
    for command in 0..26 {
        operations.extend([(Local, 104), (Load4, 0), (Const, command), (Eq, 0)]);
        branches.push(operations.len() - 1);
    }
    operations.extend([(Const, 0), (Leave, 96)]);
    for (command, branch) in branches.into_iter().enumerate() {
        operations[branch].1 = operations.len() as i32;
        if matches!(command, 12 | 13 | 16 | 19) {
            let (number, address) = match command {
                12 => (37, 880),
                16 => (38, 900),
                19 => (37, 920),
                _ => (41, 700),
            };
            operations.extend([
                (Const, address),
                (Arg, 8),
                (Const, -i32::from(ordinal(ui, number)) - 1),
                (Call, 0),
            ]);
        } else if matches!(command, 14 | 15 | 17 | 18) {
            operations.extend([
                (
                    Const,
                    match command {
                        14 => 708,
                        17 => 808,
                        18 => 812,
                        _ => 700,
                    },
                ),
                (Local, 108),
                (Load4, 0),
                (Store4, 0),
                (Const, 0),
            ]);
        } else if command >= 21 {
            operations.extend([
                (
                    Const,
                    match command {
                        21 => 940,
                        22 => 944,
                        23 => 32,
                        24 => 36,
                        _ => 40,
                    },
                ),
                (Load4, 0),
            ]);
        } else if command >= CALLS.len() && command != 20 {
            for (at, word) in if command == 10 {
                [(64, 900.0f32.to_bits()), (328, 777)]
            } else {
                [(332, 0), (332, 0)]
            } {
                operations.extend([(Const, at), (Const, word as i32), (Store4, 0)]);
            }
            operations.push((Const, 0));
        } else {
            let (number, words) = scene_call(command);
            let ordinal = ordinal(ui, number);
            for (index, &word) in words.iter().enumerate() {
                operations.extend([(Const, word as i32), (Arg, 8 + index as i32 * 4)]);
            }
            operations.extend([(Const, -i32::from(ordinal) - 1), (Call, 0)]);
        }
        operations.push((Leave, 96));
    }
    Program::Qvm {
        vm: Box::new(service_program::program(&operations, &data())),
        imports: if ui {
            &qa_compat::abi::Q3_UI
        } else {
            &Q3_CLIENT
        },
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_scene(ui: bool) -> Program {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 8, 8192, 4);
    put(&mut file, 392 + 16, 8192, 4);
    put(&mut file, 208, 12288, 4);
    file.resize(8704, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = Vm::map_image(
        image,
        &(0..26)
            .map(|command| NamedExport {
                name: b"GetGameAPI",
                command: Some(command),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let callback = vm.import_callback();
    let source = base + 0x2400;
    let mut code = vec![];
    for command in 0..26usize {
        code.extend([0x83, 0xf9, command as u8, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        code.extend([0x48, 0x83, 0xec, 0x58]);
        if matches!(command, 12 | 13 | 16 | 19) {
            let (number, address) = match command {
                12 => (37, 880),
                16 => (38, 900),
                19 => (37, 920),
                _ => (41, 700),
            };
            code.extend([0x48, 0xba]);
            code.extend((source + address).to_le_bytes());
            code.extend([0xb9, ordinal(ui, number), 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
        } else if matches!(command, 14 | 15 | 17 | 18) {
            code.extend([0x48, 0xb8]);
            code.extend(
                (source
                    + match command {
                        14 => 708,
                        17 => 808,
                        18 => 812,
                        _ => 700,
                    })
                .to_le_bytes(),
            );
            code.extend([0x89, 0x10, 0x31, 0xc0]);
        } else if command >= 21 {
            code.extend([0x48, 0xb8]);
            code.extend(
                (source
                    + match command {
                        21 => 940,
                        22 => 944,
                        23 => 32,
                        24 => 36,
                        _ => 40,
                    })
                .to_le_bytes(),
            );
            code.extend([0x8b, 0x00]);
        } else if command >= CALLS.len() && command != 20 {
            for (at, word) in if command == 10 {
                [(64, 900.0f32.to_bits()), (328, 777)]
            } else {
                [(332, 0), (332, 0)]
            } {
                code.extend([0x48, 0xb8]);
                code.extend((source + at).to_le_bytes());
                code.extend([0xc7, 0x00]);
                code.extend(word.to_le_bytes());
            }
            code.extend([0x31, 0xc0]);
        } else {
            let (number, arguments) = scene_call(command);
            let ordinal = ordinal(ui, number);
            for (index, &argument) in arguments.iter().enumerate() {
                let value = if pointer_argument(command, index) && argument != 0 {
                    source + u64::from(argument)
                } else {
                    u64::from(argument)
                };
                if index < 3 {
                    code.extend([[0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]][index]);
                    code.extend(value.to_le_bytes());
                } else {
                    code.extend([0x48, 0xb8]);
                    code.extend(value.to_le_bytes());
                    code.extend([0x48, 0x89, 0x44, 0x24, (0x20 + (index - 3) * 8) as u8]);
                }
            }
            code.extend([0xb9, ordinal, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
        }
        code.extend([0x48, 0x83, 0xc4, 0x58, 0xc3]);
        let end = code.len();
        code[branch..branch + 4].copy_from_slice(&((end - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0xc00);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    vm.process.memory_mut().unwrap()[0x2400..0x2800].copy_from_slice(&data());
    Program::Native {
        vm: Box::new(vm),
        imports: if ui {
            &qa_compat::abi::Q3_UI
        } else {
            &Q3_CLIENT
        },
    }
}

pub(super) fn submissions(native: bool, ui: bool) {
    if !ui {
        mark_fragments(native);
    }
    let mut fixture = model_support::Fixture::new(&[(
        "ui/pic.tga",
        model_support::tga(2, 2, &[[80, 30, 10, 255]; 4]),
    )]);
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = qa_render::Assets::load().unwrap();
    let world = model_support::world(&mut assets, -1.);
    let presentation = qa_render::world::WorldPresentationId(0);
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    let program = || {
        if native {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                return native_scene(ui);
            }
        }
        qvm_scene(ui)
    };
    let (mut host, _) = if ui {
        let (mut runtime, ids) = runtime(2);
        let requests = ids
            .iter()
            .enumerate()
            .map(|(seat, &client)| {
                let mut request =
                    request(&mut runtime, ModuleId(seat as u16 + 1), client, program());
                request.frame.client = None;
                request
            })
            .collect();
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            requests,
        )
        .unwrap();
        for (seat, &client) in ids.iter().enumerate() {
            host.local_clients[seat] = Some(client);
        }
        (host, ids)
    } else {
        host(program, 2)
    };
    host.runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut peer_profile = profile;
    peer_profile.rules = RuleSetId::Quake2;
    peer_profile.image_settings =
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake2);
    host.runtime.model_assets = Some(
        ModelAssets::new(assets, [profile, peer_profile], [world], Default::default()).unwrap(),
    );
    host.runtime
        .model_assets
        .as_mut()
        .unwrap()
        .bind_loaded_world(
            "maps/probe.bsp",
            world,
            None,
            Bounds {
                mins: Vec3([-1.; 3]),
                maxs: Vec3([1.; 3]),
            },
        )
        .unwrap();
    host.local_worlds.fill(Some((world, presentation)));
    let mut seat_profiles = [[None, None, None, None, Some(profile)]; SeatId::COUNT];
    seat_profiles[1][RuleSetId::Quake3 as usize] = Some(peer_profile);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &seat_profiles,
        320,
        200,
    )
    .unwrap();
    if !ui {
        assert_eq!(probe(&mut host, 0, 6, 0), 0);
    } // Cold selection, before any scene owner.
    assert_eq!(probe(&mut host, 0, 0, 0), 1);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    let material = models.shader_binding(1, profile).unwrap();
    let peer_material = models.shader_binding(1, peer_profile).unwrap();
    assert_ne!(material, peer_material);
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits {
                polys: 2,
                vertices: 6,
                ..Default::default()
            },
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![loaded],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 1, 0), 0);
    assert_eq!(probe(&mut host, 0, 2, 0), 0);
    assert_eq!(probe(&mut host, 0, 2, 0), 0); // Both extra polygons rejected without a trap.
    if ui {
        assert_eq!(probe(&mut host, 0, 2, 0), 0);
        assert_eq!(probe(&mut host, 0, 2, 0), 0);
    }
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    assert_eq!(probe(&mut host, 0, 4, 0), 0);
    assert_eq!(probe(&mut host, 0, 10, 0), 0); // Change guest source after publication.
    assert_eq!(probe(&mut host, 0, 5, 0), 0); // Clear preserves 2D color and earlier view.
    if !ui {
        assert_eq!(probe(&mut host, 0, 11, 0), 0);
    } // Render the selected world, no longer NOWORLDMODEL.
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    assert_eq!(probe(&mut host, 0, 9, 0), 0); // A zero shader does not dereference a bad pointer.
    assert_eq!(probe(&mut host, 1, 4, 0), 0); // Other module retains white and separate target.
    let packet = host.runtime.scene.as_mut().unwrap().finish_frame().unwrap();
    assert_eq!(packet.rejected, 2);
    let Command::View(view) = packet.commands()[1] else {
        panic!("native view")
    };
    assert_eq!(view.refdef.time_ms, -12345);
    assert_eq!(view.refdef.text[7][1], 0xe7);
    assert_eq!(view.scene.polys.count, 2);
    let recipient = host.local_clients[0].map(|client| qa_render::effects::EffectClient {
        client,
        lifetime: host.runtime.server.clients[client.0 as usize].entity,
    });
    assert_eq!(
        view.particle_client, None,
        "no-world model previews omit world effects"
    );
    assert_eq!(
        packet.vertices(packet.polys(view.scene.polys)[0].vertices)[0]
            .position
            .0[0],
        8.
    );
    let Command::Draw2d(picture) = packet.commands()[2] else {
        panic!("native picture")
    };
    assert_eq!(picture.shader, material);
    assert_eq!(picture.rect, [0., 0., 90., 30.]);
    assert_eq!(picture.color, [127, 63, 255, 191]);
    let Command::View(second) = packet.commands()[3] else {
        panic!("second native view")
    };
    assert_eq!(second.refdef.time_ms, 777);
    assert_eq!(second.particle_client, if ui { None } else { recipient });
    assert_eq!(packet.hidden_areas(second.hidden_areas), &[0xa5; 32]);
    let Command::Draw2d(peer) = packet.commands()[4] else {
        panic!("peer picture")
    };
    assert_eq!(peer.rect, [if ui { 0. } else { 160. }, 0., 90., 30.]);
    assert_eq!(peer.color, [255; 4]);
    assert_eq!(peer.shader, if ui { material } else { peer_material });
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

pub(super) fn ownership() {
    use qa_app::renderer::{Context as RenderContext, Renderer};
    use qa_render::material::resources::ImageSettings;
    let (mut runtime, ids) = runtime(2);
    let requests = ids
        .iter()
        .enumerate()
        .map(|(seat, &client)| {
            let mut request = request(
                &mut runtime,
                ModuleId(seat as u16 + 1),
                client,
                qvm_scene(seat == 0),
            );
            request.frame.callback = CallbackId(3);
            if seat == 0 {
                request.frame.client = None;
            }
            request
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.local_clients[..2].copy_from_slice(&ids.iter().copied().map(Some).collect::<Vec<_>>());
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = qa_render::Assets::load().unwrap();
    let world = model_support::world(&mut assets, -1.);
    let presentation = qa_render::world::WorldPresentationId(0);
    host.local_worlds[..2].fill(Some((world, presentation)));
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    let console = qa_app::renderer::console::ConsoleAssets::load(
        &host.runtime.vfs,
        &mut assets,
        RuleSetId::Quake3,
        &mut qa_app::render_settings::PaletteRegistrations::default(),
        ImageSettings::native(RuleSetId::Quake3),
        320,
        200,
    )
    .unwrap();
    let effects = std::sync::Arc::clone(&host.runtime.catalog.registry.effects);
    let bindings = vec![None; effects.len() + 1];
    let effects = qa_render::effects::Effects::load(effects, 8, 2, &bindings).unwrap();
    let configuration = std::sync::Arc::new(qa_core::presentation::RendererConfiguration {
        display: qa_core::presentation::DisplayConfiguration {
            width: 320,
            height: 200,
            ..Default::default()
        },
        ..Default::default()
    });
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    host.runtime.model_assets =
        Some(ModelAssets::new(assets, [], [world], Default::default()).unwrap());
    host.bind_client_model_assets([]);
    let (renderer, scene) = Renderer::load(
        RenderContext::Cpu(configuration),
        320,
        200,
        host.runtime.model_assets.as_mut().unwrap().assets_mut(),
        vec![loaded],
        Some(qa_render::cpu::RasterBands::One),
        console,
        std::array::from_fn(|_| None),
        effects,
    )
    .unwrap();
    host.publish_model_admission(renderer.admitted_assets().unwrap())
        .unwrap();
    host.runtime.scene = Some(scene);
    struct Source {
        renderer: Renderer,
        views: [Option<qa_app::host::ClientView>; SeatId::COUNT],
    }
    impl qa_app::host::FrameSource for Source {
        fn begin_frame(&mut self) -> Duration {
            Duration::from_millis(16)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            queue
                .push(SysEvent {
                    time: EventTime::from_milliseconds(16),
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, _: Duration) -> Duration {
            self.begin_frame()
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
        fn render(
            &mut self,
            scene: Option<&mut qa_app::renderer::SceneComposer>,
            resources: Option<qa_app::model_assets::ModelView<'_>>,
            _: &[Option<qa_app::host::ClientView>; SeatId::COUNT],
            _: &[Option<qa_app::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
            _: &[Option<qa_app::renderer::hud::HudView<'_>>; SeatId::COUNT],
            console: qa_console::presentation::ConsoleView<'_>,
        ) {
            self.renderer.frame(
                scene.unwrap(),
                resources.unwrap(),
                &self.views,
                &std::array::from_fn(|_| None),
                &std::array::from_fn(|_| None),
                console,
            );
        }
    }
    let view = qa_app::host::ClientView {
        world,
        world_presentation: presentation,
        presentation: Default::default(),
        origin: Vec3::default(),
        angles: Vec3::default(),
        fov_x: 90.,
        screen_blend: [0.0; 4],
        damage_blend: [0.0; 4],
        flags: Default::default(),
        time_ms: 16,
        lighting_scales: [0.6, 1.],
    };
    let mut source = Source {
        renderer,
        views: [Some(view), Some(view), None, None],
    };
    let result = host.frame(&mut source, true);
    assert!(!host.runtime.quit);
    assert_eq!(result.drains, 2);
    assert_eq!(
        source.renderer.sample.stats.views, 3,
        "one native client view, one builtin peer, one unbound UI preview"
    );
    assert_eq!(source.renderer.sample.stats.rejected, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

pub(super) fn entities(native: bool, ui: bool) {
    let mut fixture = model_support::Fixture::surfaces();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let program = || {
        if native {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                return native_scene(ui);
            }
        }
        qvm_scene(ui)
    };
    let (mut runtime, ids) = runtime(1);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program());
    request.entries = (0..20).collect();
    if ui {
        request.frame.client = None;
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    host.runtime.model_assets = Some(
        ModelAssets::new(
            qa_render::Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap(),
    );
    let model = probe(&mut host, 0, 12, 0);
    assert!(model > 0);
    assert_eq!(probe(&mut host, 0, 14, model as u64), 0);
    let shader = host
        .runtime
        .model_assets
        .as_mut()
        .unwrap()
        .register_shader(&host.runtime.vfs, b"textures/glow", RuleSetId::Quake3, true)
        .unwrap();
    assert_eq!(shader, 1);
    let skin = probe(&mut host, 0, 16, 0);
    assert!(skin > 0);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    let registration = models.renderer_registration(model as i32, profile).unwrap();
    let skinned = models
        .bindings(
            registration,
            0,
            qa_app::model_assets::ModelMaterialPolicy {
                custom_skin: skin as i32,
                ..Default::default()
            },
        )
        .unwrap()
        .collect::<Vec<_>>();
    let override_material = models.shader_binding(shader, profile).unwrap();
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits {
                entities: 2,
                ..Default::default()
            },
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 13, 0), 0); // A full two-surface model fails atomically.
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    assert_eq!(packet.rejected, 1);
    let Command::View(view) = packet.commands()[1] else {
        panic!("model view")
    };
    assert_eq!(view.scene.entities.count, 2);
    for entity in packet.entities(view.scene.entities) {
        assert_eq!(entity.frame, 1);
        assert_eq!(entity.old_frame, 0);
        assert_eq!(entity.back_lerp, 0.25);
        assert_eq!(entity.color, [201, 150, 90, 255]);
        assert_eq!(entity.shader_time, 1.25);
        assert_eq!(entity.shader_texcoord, [0.125, 0.5]);
        assert!(
            entity.lighting.is_some(),
            "native no-world model lighting reaches the shared stage inputs"
        );
    }
    assert!(owner.recycle(packet).is_ok());
    assert_eq!(probe(&mut host, 0, 16, 0), skin);
    assert_eq!(probe(&mut host, 0, 17, skin as u64), 0);
    assert_eq!(probe(&mut host, 0, 18, 0), 0);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("skin view")
    };
    for (entity, binding) in packet.entities(view.scene.entities).iter().zip(&skinned) {
        assert_eq!(entity.shader, Some(binding.shader));
    }
    assert_eq!(packet.rejected, 0);
    assert!(owner.recycle(packet).is_ok());
    // A custom shader takes priority; an invalid positive handle uses the default.
    for (handle, expected) in [
        (shader as u64, override_material),
        (i32::MAX as u64, qa_render::ShaderBindingId(0)),
    ] {
        assert_eq!(probe(&mut host, 0, 18, handle), 0);
        assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
        assert_eq!(probe(&mut host, 0, 13, 0), 0);
        assert_eq!(probe(&mut host, 0, 3, 0), 0);
        let owner = host.runtime.scene.as_mut().unwrap();
        let packet = owner.finish_frame().unwrap();
        let Command::View(view) = packet.commands()[1] else {
            panic!("override view")
        };
        assert!(
            packet
                .entities(view.scene.entities)
                .iter()
                .all(|entity| entity.shader == Some(expected))
        );
        assert_eq!(packet.rejected, 0);
        assert!(owner.recycle(packet).is_ok());
    }
    assert_eq!(probe(&mut host, 0, 18, shader as u64), 0);
    for (kind, triangles) in [(1, 0), (2, 2), (3, 12), (4, 2), (5, 2), (6, 8), (7, 0)] {
        assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
        assert_eq!(probe(&mut host, 0, 15, kind), 0);
        assert_eq!(probe(&mut host, 0, 13, 0), 0);
        assert_eq!(probe(&mut host, 0, 3, 0), 0);
        let owner = host.runtime.scene.as_mut().unwrap();
        let packet = owner.finish_frame().unwrap();
        let Command::View(view) = packet.commands()[1] else {
            panic!("generated view")
        };
        assert_eq!(view.scene.entities.count, 1);
        assert_eq!(view.scene.polys.count, triangles);
        if kind == 3 {
            assert_eq!(packet.entity(0).origin, Vec3::default());
            assert_eq!(packet.entity(0).color, [255, 0, 0, 255]);
            assert_eq!(
                packet.entity(0).model,
                qa_render::SceneModel::Beam {
                    end: Vec3([-8., 0., 0.]),
                    radius: 4.,
                    world_space: true,
                }
            );
        }
        assert_eq!(packet.rejected, 0);
        assert!(owner.recycle(packet).is_ok());
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

pub(super) fn inline_entities(native: bool) {
    let (mut runtime, ids) = runtime(1);
    let program = if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            native_scene(false)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            qvm_scene(false)
        }
    } else {
        qvm_scene(false)
    };
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
    request.entries = (0..20).collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = qa_render::Assets::load().unwrap();
    let portal_material = assets
        .register_material(
            "native portal",
            &[qa_render::assets::Stage {
                alpha_gen: qa_render::shader::AlphaGen::Const(0.),
                blend: Some(qa_render::shader::StageBlend::ALPHA),
                depth_write: false,
                ..Default::default()
            }],
            qa_render::assets::MaterialSettings {
                sort: 1.,
                portal: true,
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let world = model_support::world_mutated(
        &mut assets,
        10.,
        &[&[qa_render::world::SurfaceMaterial {
            shader: qa_render::ShaderBinding::material(
                portal_material,
                qa_render::world::geometry::LightSource::None,
            ),
            ..Default::default()
        }]],
        |geometry| {
            use qa_render::world::geometry::*;
            let bounds = qa_core::primitives::Bounds {
                mins: Vec3([8., -8., -8.]),
                maxs: Vec3([8., 8., 8.]),
            };
            geometry.vertices = [[8., -8., -8.], [8., 8., -8.], [8., 0., 8.]]
                .map(|position| WorldVertex {
                    vertex: qa_render::Vertex {
                        position: Vec3(position),
                        normal: Vec3([-1., 0., 0.]),
                        ..Default::default()
                    },
                })
                .into();
            geometry.indices = vec![0, 1, 2];
            geometry.boundaries = vec![qa_core::primitives::IndexSpan { first: 0, count: 3 }];
            geometry.surfaces = vec![WorldSurface {
                source_id: 0,
                kind: GeometryKind::Triangles,
                vertices: qa_core::primitives::IndexSpan { first: 0, count: 3 },
                indices: qa_core::primitives::IndexSpan { first: 0, count: 3 },
                boundaries: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                plane: Some(qa_core::primitives::Plane::oriented(
                    Vec3([-1., 0., 0.]),
                    -8.,
                )),
                bounds,
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [0; 2],
                lightmap_projection: None,
                lightmap_grid: [0; 2],
                styles: [255; 4],
                light_source: LightSource::None,
                light_encoding: LightEncoding::Rgb,
                light_samples: Default::default(),
                source_texture: None,
                source_texture_info: None,
                source_shader: None,
                source_flags: 0,
                source_contents: 0,
                no_draw: false,
                source_fog: -1,
                source_brush_side: -1,
                source_lightmap: -1,
                lightmap_rect: [0; 4],
                lightmap_origin: Vec3::default(),
                lightmap_vectors: [Vec3::default(); 3],
                patch: None,
            }];
            geometry.models[0].surfaces = qa_core::primitives::IndexSpan { first: 0, count: 1 };
        },
    );
    let presentation = qa_render::world::WorldPresentationId(0);
    let bounds = assets.world(world).unwrap().geometry().models[1].bounds;
    let grid = qa_render::lighting::LightGrid::load(
        qa_core::primitives::Bounds {
            mins: Vec3::default(),
            maxs: Vec3([1.; 3]),
        },
        Vec3([1.; 3]),
        [qa_formats::bsp::LightGridPoint {
            ambient: [10, 20, 30],
            directed: [0; 3],
            longitude: 0,
            latitude: 0,
        }; 8]
            .into_iter(),
        0,
    )
    .unwrap();
    assets.set_light_grid(world, Some(grid)).unwrap();
    assets.set_world_sun(world, Vec3([0., 1., 0.])).unwrap();
    let meshes = assets.models().len();
    let mut builder = ModelAssets::new(assets, [profile], [world], Default::default()).unwrap();
    builder
        .bind_loaded_world("maps/probe.bsp", world, None, bounds)
        .unwrap();
    host.runtime.model_assets = Some(builder);
    // No scene world is inferred just because geometry exists in the catalog.
    assert_eq!(probe(&mut host, 0, 19, 0), 0);
    assert_eq!(probe(&mut host, 0, 6, 0), 0);
    let model = probe(&mut host, 0, 19, 0);
    assert!(model > 0);
    host.bind_client_model_assets([]);
    let models = host.runtime.model_assets.as_ref().unwrap().catalog();
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .models()
            .len(),
        meshes
    );
    assert_eq!(models.model_bounds(model as i32), Some(bounds));
    let binding = models.resolve_inline(world, 1).unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.local_worlds[0] = Some((world, presentation));
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            Default::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![loaded],
        )
        .unwrap()
        .0,
    );
    let _backend = admit_assets(&mut host);
    assert_eq!(probe(&mut host, 0, 19, 0), model);
    assert_eq!(probe(&mut host, 0, 14, model as u64), 0);
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("inline view")
    };
    assert_eq!(packet.rejected, 0);
    assert_eq!(view.scene.entities.count, 1);
    let entity = packet.entities(view.scene.entities)[0];
    assert_eq!(
        entity.model,
        qa_render::scene::SceneModel::World {
            world,
            presentation,
            surfaces: binding.surfaces
        }
    );
    assert_eq!(entity.origin.0, [8., 0., 0.]);
    assert_eq!(
        entity.lighting.unwrap().direction,
        Vec3([0., 1., 0.]),
        "a no-world native view retains the selected world's sun instead of its grid"
    );
    assert_eq!(
        entity.axes.map(|axis| axis.0),
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
    );
    assert!(owner.recycle(packet).is_ok());
    assert_eq!(probe(&mut host, 0, 11, 0), 0); // Enable world lighting in the next refdef.
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let Command::View(view) = packet.commands()[1] else {
        panic!("grid view")
    };
    assert_eq!(
        packet.entities(view.scene.entities)[0]
            .lighting
            .unwrap()
            .ambient,
        [38., 44., 50.]
    );
    assert_eq!(packet.rejected, 0);
    assert!(owner.recycle(packet).is_ok());
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 15, 7), 0);
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 3, 0), 0);
    let owner = host.runtime.scene.as_mut().unwrap();
    let packet = owner.finish_frame().unwrap();
    let [
        Command::Clear(_),
        Command::View(child),
        Command::View(parent),
    ] = packet.commands()
    else {
        panic!("native portal views")
    };
    assert!(child.refdef.portal && child.refdef.clip_plane.is_some());
    assert!(!parent.refdef.portal);
    assert_eq!(parent.refdef.time_ms, child.refdef.time_ms);
    assert_eq!(
        packet.entities(parent.scene.entities)[0].model,
        packet.entities(child.scene.entities)[0].model
    );
    assert_eq!(packet.rejected, 0);
    assert!(owner.recycle(packet).is_ok());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

fn mark_fragments(native: bool) {
    use qa_render::world::{
        SurfaceMaterial,
        geometry::{GeometryKind, WorldModel},
    };
    let (mut geometry, visibility) = mark_world::inputs(1);
    geometry.surfaces[0].kind = GeometryKind::Polygon;
    geometry.surfaces[0].plane = Some(Plane::oriented(Vec3([-1., 0., 0.]), -1.));
    geometry.models.push(WorldModel {
        bounds: geometry.surfaces[0].bounds,
        origin: Vec3::default(),
        surfaces: IndexSpan { first: 0, count: 1 },
    });
    let mut assets = qa_render::Assets::load().unwrap();
    let world = assets
        .register_world(geometry, visibility, &[&[SurfaceMaterial::default()]])
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let (mut runtime, ids) = runtime(1);
    let program = if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            native_scene(false)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            qvm_scene(false)
        }
    } else {
        qvm_scene(false)
    };
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
    request.entries = (0..26).collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.local_worlds[0] = Some((world, presentation));
    host.runtime.model_assets =
        Some(ModelAssets::new(assets, [profile], [world], Default::default()).unwrap());
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    assert_eq!(probe(&mut host, 0, 20, 0), 0); // No scene owner yet.
    host.bind_client_model_assets([]);
    let loaded = LoadedWorld {
        world,
        presentations: vec![LoadedWorldPresentation {
            id: presentation,
            profile: WorldPresentationProfile::native(RuleSetId::Quake3, None),
            defaults: Default::default(),
        }]
        .into_boxed_slice(),
        diagnostics: Box::default(),
    };
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(
            qa_render::Limits::default(),
            host.runtime.model_assets.as_mut().unwrap().assets_mut(),
            vec![loaded],
        )
        .unwrap()
        .0,
    );
    assert_eq!(probe(&mut host, 0, 20, 0), 0); // An unpublished suffix is not renderer authority.
    let _backend = admit_assets(&mut host);
    assert_eq!(probe(&mut host, 0, 20, 0), 1);
    assert_eq!(probe(&mut host, 0, 21, 0), 0);
    assert_eq!(probe(&mut host, 0, 22, 0), 3);
    assert_eq!(probe(&mut host, 0, 23, 0), 1f32.to_bits());
    assert_eq!(probe(&mut host, 0, 24, 0), (-1f32).to_bits());
    assert_eq!(probe(&mut host, 0, 25, 0), (-1f32).to_bits());
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    println!(
        "actual {} MarkFragments uses published selected world geometry",
        if native { "owned-native" } else { "QVM" }
    );
}
