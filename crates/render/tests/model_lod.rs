use qa_core::primitives::Vec3;
use qa_render::{
    Assets, Command, FrontEnd, Limits, ModelId, ModelLodSettings, ModelPart, Refdef, SceneEntity,
    SceneModel, ShaderBinding, ShaderBindingId, Vertex, Viewport,
    assets::{Cull, MaterialSettings, Stage},
    cpu::CpuBackend,
    scene::Span,
    shader::{AlphaGen, RgbGen, StageBlend, reference::ShaderView},
    world::{SurfaceMaterial, VisibleSurface, WorldPresentationInput, geometry::LightSource},
};

#[path = "support/world_surfaces.rs"]
mod world_surfaces;

const RED: u32 = u32::from_le_bytes([255, 0, 0, 255]);
const GREEN: u32 = u32::from_le_bytes([0, 255, 0, 255]);
const BLUE: u32 = u32::from_le_bytes([0, 0, 255, 255]);
const YELLOW: u32 = u32::from_le_bytes([255, 255, 0, 255]);
const CLEAR: u32 = u32::from_le_bytes([3, 5, 7, 255]);

fn shader(assets: &mut Assets, name: &str, color: [f32; 3]) -> ShaderBindingId {
    let material = assets
        .register_material(
            name,
            &[Stage {
                rgb_gen: RgbGen::Const(color),
                ..Default::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .unwrap()
}

fn vertices(x: f32, minimum_y: f32) -> [Vertex; 4] {
    [
        [x, minimum_y, -128.],
        [x, 128., -128.],
        [x, 128., 128.],
        [x, minimum_y, 128.],
    ]
    .map(|position| Vertex {
        position: Vec3(position),
        normal: Vec3([-1., 0., 0.]),
        color: [255; 4],
        ..Default::default()
    })
}

fn fixture() -> (Assets, [ModelPart; 4]) {
    let mut assets = Assets::load().unwrap();
    let shaders = [
        ("lod/base", [1., 0., 0.]),
        ("lod/middle", [0., 1., 0.]),
        ("lod/coarse", [0., 0., 1.]),
        ("lod/other-middle-skin", [1., 1., 0.]),
    ]
    .map(|(name, color)| shader(&mut assets, name, color));
    let parts = std::array::from_fn(|index| {
        let model = assets
            .register_model(
                &vertices(
                    if index == 3 { -0.5 } else { 0. },
                    if index == 3 { 0. } else { -128. },
                ),
                if index == 1 {
                    &[0, 2, 3]
                } else {
                    &[0, 1, 2, 0, 2, 3]
                },
                shaders[index],
            )
            .unwrap();
        ModelPart {
            model,
            shader: shaders[index].into(),
        }
    });
    (assets, parts)
}

fn view(x: u32, camera_x: f32, settings: ModelLodSettings) -> Refdef {
    Refdef {
        viewport: Viewport {
            x,
            width: 16,
            height: 16,
            ..Default::default()
        },
        origin: Vec3([camera_x, 0., 0.]),
        fov: [90.; 2],
        near: 0.25,
        model_lod: settings,
        ..Default::default()
    }
}

fn choose(
    settings: ModelLodSettings,
    radius: f32,
    count: u32,
    origin: Vec3,
    view: Refdef,
) -> Option<u32> {
    settings.select(
        radius,
        count,
        origin,
        &view,
        view.projection_scales().unwrap(),
    )
}

#[test]
fn native_projection_controls_keep_conditional_clamps_and_numeric_order() {
    let input = view(0, 0., ModelLodSettings::default());
    let defaults = ModelLodSettings::default();
    assert_eq!(choose(defaults, 2., 3, Vec3([8., 0., 0.]), input), Some(0));
    assert_eq!(choose(defaults, 2., 3, Vec3([64., 0., 0.]), input), Some(2));
    for scale in [0., -1.] {
        assert_eq!(
            choose(
                ModelLodSettings { scale, bias: 0 },
                2.,
                3,
                Vec3([8., 0., 0.]),
                input
            ),
            Some(2)
        );
    }
    assert_eq!(
        choose(
            ModelLodSettings {
                scale: f32::NAN,
                bias: 0
            },
            2.,
            3,
            Vec3([8., 0., 0.]),
            input
        ),
        Some(0)
    );
    for scale in [20., 21., f32::INFINITY] {
        assert_eq!(
            choose(
                ModelLodSettings { scale, bias: 0 },
                2.,
                3,
                Vec3([100., 0., 0.]),
                input
            ),
            Some(1)
        );
    }
    assert_eq!(
        choose(
            ModelLodSettings {
                scale: 0.5,
                bias: 0
            },
            20.,
            3,
            Vec3([10., 0., 0.]),
            input
        ),
        Some(1)
    );
    for distance in [0., -100.] {
        assert_eq!(
            choose(
                ModelLodSettings { scale: 0., bias: 0 },
                2.,
                3,
                Vec3([distance, 0., 0.]),
                input
            ),
            Some(0)
        );
    }
    for (bias, expected) in [(i32::MIN, 0), (1, 1), (i32::MAX, 2)] {
        assert_eq!(
            choose(
                ModelLodSettings { bias, ..defaults },
                2.,
                3,
                Vec3([8., 0., 0.]),
                input
            ),
            Some(expected)
        );
    }
    assert_eq!(
        choose(
            ModelLodSettings {
                bias: i32::MAX,
                ..defaults
            },
            f32::NAN,
            1,
            Vec3::default(),
            input
        ),
        Some(0)
    );
    assert_eq!(choose(defaults, 1., 0, Vec3::default(), input), None);

    let unit_scale = ModelLodSettings { scale: 1., bias: 0 };
    assert_eq!(
        choose(unit_scale, 1., 3, Vec3([1.5, 0., 0.]), input),
        Some(0)
    );
    assert_eq!(
        choose(
            unit_scale,
            1.,
            3,
            Vec3([f32::from_bits(1.5_f32.to_bits() + 1), 0., 0.]),
            input
        ),
        Some(1)
    );
    assert_eq!(choose(defaults, 2., 3, Vec3([40., 0., 0.]), input), Some(2));
    assert_eq!(
        choose(
            defaults,
            2.,
            3,
            Vec3([40., 0., 0.]),
            Refdef {
                fov: [90., 45.],
                ..input
            }
        ),
        Some(1)
    );

    let canceling = Refdef {
        origin: Vec3([16_777_216., 0., 0.]),
        axes: qa_core::math::angle_vectors(Vec3([0., 45., 0.])).axes(),
        ..input
    };
    let origin = Vec3([16_777_216., 0.25, 0.]);
    assert_eq!(
        canceling.axes[0].dot(origin),
        canceling.axes[0].dot(canceling.origin)
    );
    assert!((origin - canceling.origin).dot(canceling.axes[0]) > 0.);
    assert_eq!(
        choose(
            ModelLodSettings { scale: 0., bias: 0 },
            1.,
            3,
            origin,
            canceling
        ),
        Some(0)
    );
}

#[test]
fn multipart_choices_capture_each_view_and_drive_real_cpu_pixels_in_either_order() {
    let (assets, parts) = fixture();
    let requests = [
        (56., ModelLodSettings::default(), vec![parts[0].model], RED),
        (0., ModelLodSettings::default(), vec![parts[2].model], BLUE),
        (
            56.,
            ModelLodSettings {
                bias: 1,
                ..Default::default()
            },
            vec![parts[1].model, parts[3].model],
            GREEN,
        ),
        (
            56.,
            ModelLodSettings { scale: 0., bias: 0 },
            vec![parts[2].model],
            BLUE,
        ),
        (
            56.,
            ModelLodSettings {
                scale: -1.,
                bias: 0,
            },
            vec![parts[2].model],
            BLUE,
        ),
        (
            0.,
            ModelLodSettings {
                bias: -2,
                ..Default::default()
            },
            vec![parts[0].model],
            RED,
        ),
    ];
    for reverse in [false, true] {
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
        for index in 0..requests.len() {
            let selected = if reverse {
                requests.len() - index - 1
            } else {
                index
            };
            let (camera_x, settings, _, _) = &requests[selected];
            assert!(frame.add_model_lods(
                SceneEntity {
                    origin: Vec3([64., 0., 0.]),
                    ..Default::default()
                },
                Some(2.),
                [
                    [Some(parts[0]), Some(parts[1]), None],
                    [None, Some(parts[3]), Some(parts[2])],
                ],
            ));
            assert!(frame.render_scene(
                view(index as u32 * 16, *camera_x, *settings),
                &[],
                &assets,
                ShaderView::direct()
            ));
        }
        let mut packet = frame.finish();
        packet.resolve_shaders(&assets, &());
        let views: Vec<_> = packet
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::View(view) => Some(view),
                _ => None,
            })
            .collect();
        for (index, view) in views.iter().enumerate() {
            let selected = if reverse {
                requests.len() - index - 1
            } else {
                index
            };
            let (_, settings, models, _) = &requests[selected];
            assert_eq!(view.refdef.model_lod, *settings);
            assert_eq!(
                packet
                    .draws(view.scene.draws)
                    .iter()
                    .map(|draw| draw.mesh.unwrap())
                    .collect::<Vec<_>>(),
                *models
            );
            assert!(
                packet
                    .entities(view.scene.entities)
                    .iter()
                    .all(|entity| matches!(entity.model, SceneModel::Lods { .. }))
            );
            for draw in packet.draws(view.scene.draws) {
                let part = parts
                    .iter()
                    .find(|part| Some(part.model) == draw.mesh)
                    .unwrap();
                assert_eq!(draw.shader.unwrap().source(&()), part.shader.source(&()));
            }
        }
        let mut cpu =
            CpuBackend::load_with_assets(requests.len() as u32 * 16, 16, &assets).unwrap();
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0, "{stats:?}");
        assert!(stats.triangles > 0);
        for index in 0..requests.len() {
            let selected = if reverse {
                requests.len() - index - 1
            } else {
                index
            };
            let color = requests[selected].3;
            assert_eq!(
                cpu.pixels()[8 * requests.len() * 16 + index * 16 + 12],
                color,
                "view {selected}, reverse={reverse}"
            );
            if color == GREEN {
                assert_eq!(
                    cpu.pixels()[8 * requests.len() * 16 + index * 16 + 4],
                    YELLOW
                );
            }
        }
    }
}

#[test]
fn choice_capacity_rolls_back_the_whole_group_and_single_levels_use_no_arena() {
    let (assets, parts) = fixture();
    let mut front = FrontEnd::load(Limits {
        entities: 2,
        model_choices: 3,
        model_joints: 0,
        ..Default::default()
    })
    .unwrap();
    let mut frame = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
    let pose = SceneEntity {
        origin: Vec3([8., 0., 0.]),
        ..Default::default()
    };
    assert!(!frame.add_model_lods(pose, Some(2.), [[Some(parts[0]); 3]; 2]));
    assert!(frame.add_model_lods(
        pose,
        Some(2.),
        [[Some(parts[0]), Some(parts[1]), Some(parts[2])]]
    ));
    assert!(frame.add_model_lods(pose, None, [[None]]));
    assert!(frame.add_model_lods(
        SceneEntity {
            origin: Vec3([7., 0., 0.]),
            ..pose
        },
        None,
        [[Some(parts[2])]]
    ));
    assert!(frame.render_scene(
        view(
            0,
            0.,
            ModelLodSettings {
                bias: 1,
                ..Default::default()
            }
        ),
        &[],
        &assets,
        ShaderView::direct()
    ));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &());
    let Command::View(view) = packet.commands()[1] else {
        panic!("view")
    };
    assert_eq!(packet.rejected, 1);
    assert_eq!(view.scene.entities.count, 2);
    assert!(matches!(
        packet.entities(view.scene.entities)[0].model,
        SceneModel::Lods {
            choices: Span { first: 0, count: 3 },
            ..
        }
    ));
    assert_eq!(
        packet.entities(view.scene.entities)[1].model,
        SceneModel::Mesh(parts[2].model)
    );
    let mut cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 1);
    assert_eq!(cpu.pixels()[8 * 16 + 8], BLUE);

    assert!(front.recycle(packet).is_ok());
    let mut next = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
    assert!(next.add_model_lods(pose, Some(2.), [[Some(parts[0]); 3]]));
}

#[test]
fn zero_choice_capacity_keeps_ordinary_meshes_and_can_grow_cold() {
    let (assets, parts) = fixture();
    let mut front = FrontEnd::load(Limits {
        entities: 2,
        model_choices: 0,
        model_joints: 0,
        ..Default::default()
    })
    .unwrap();
    let pose = SceneEntity {
        origin: Vec3([8., 0., 0.]),
        ..Default::default()
    };
    let mut frame = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
    assert!(!frame.add_model_lods(pose, Some(2.), [[Some(parts[0]); 2]]));
    assert!(frame.add_model_lods(pose, None, [[Some(parts[2])]]));
    assert!(frame.render_scene(
        view(0, 0., ModelLodSettings::default()),
        &[],
        &assets,
        ShaderView::direct()
    ));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &());
    let Command::View(submitted) = packet.commands()[1] else {
        panic!("view")
    };
    assert_eq!(submitted.scene.entities.count, 1);
    assert_eq!(
        packet.draws(submitted.scene.draws)[0].mesh,
        Some(parts[2].model)
    );
    let mut cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 1);
    assert_eq!(cpu.pixels()[8 * 16 + 8], BLUE);
    assert!(front.recycle(packet).is_ok());

    let required = Limits {
        model_choices: 3,
        model_joints: 0,
        ..front.limits()
    };
    front.stage_capacity(required).unwrap().commit();
    let mut next = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
    assert!(next.add_model_lods(pose, Some(2.), [[Some(parts[0]); 3]]));
    assert!(next.render_scene(
        view(0, 0., ModelLodSettings::default()),
        &[],
        &assets,
        ShaderView::direct()
    ));
    let mut packet = next.finish();
    packet.resolve_shaders(&assets, &());
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert_eq!(cpu.pixels()[8 * 16 + 8], RED);
}

#[test]
fn arbitrary_level_counts_and_bad_selected_frames_drop_only_the_affected_part() {
    let (mut assets, parts) = fixture();
    let frames: Vec<_> = vertices(0., -128.).into_iter().cycle().take(8).collect();
    let animated = assets
        .register_animated_model(
            &frames,
            4,
            &[0, 1, 2, 0, 2, 3],
            parts[0].shader.source(&()).unwrap(),
        )
        .unwrap();
    let animated = ModelPart {
        model: animated,
        ..parts[0]
    };
    let pose = SceneEntity {
        origin: Vec3([64., 0., 0.]),
        frame: 1,
        old_frame: 1,
        ..Default::default()
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
    assert!(frame.add_model_lods(pose, Some(2.), [[Some(animated), Some(parts[1])]]));
    assert!(frame.render_scene(
        view(0, 56., ModelLodSettings::default()),
        &[],
        &assets,
        ShaderView::direct()
    ));
    assert!(frame.add_model_lods(pose, Some(2.), [[Some(animated), Some(parts[1])]]));
    assert!(frame.add_model_lods(
        SceneEntity {
            frame: 0,
            old_frame: 0,
            ..pose
        },
        None,
        [[Some(parts[0]); 2]]
    ));
    assert!(frame.add_model_lods(
        SceneEntity {
            frame: 0,
            old_frame: 0,
            ..pose
        },
        Some(2.),
        [[
            Some(parts[0]),
            Some(parts[1]),
            Some(parts[2]),
            Some(parts[3])
        ]]
    ));
    assert!(frame.render_scene(
        view(16, 0., ModelLodSettings { scale: 0., bias: 0 }),
        &[],
        &assets,
        ShaderView::direct()
    ));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &());
    let Command::View(first) = packet.commands()[1] else {
        panic!("first view")
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!("second view")
    };
    assert_eq!(
        packet.draws(first.scene.draws)[0].mesh,
        Some(animated.model)
    );
    assert_eq!(packet.draws(second.scene.draws).len(), 1);
    assert_eq!(
        packet.draws(second.scene.draws)[0].mesh,
        Some(parts[3].model)
    );
    assert_eq!(packet.rejected, 2);
    let mut cpu = CpuBackend::load_with_assets(32, 16, &assets).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 2);
    assert_eq!(cpu.pixels()[8 * 32 + 8], RED);
    assert_eq!(cpu.pixels()[8 * 32 + 16 + 4], YELLOW);
}

#[test]
fn portal_copies_share_choices_but_capture_their_own_selected_mesh() {
    let (mut assets, parts) = fixture();
    let material = assets
        .register_material(
            "lod/portal",
            &[Stage {
                alpha_gen: AlphaGen::Const(0.),
                blend: Some(StageBlend::ALPHA),
                depth_write: false,
                ..Default::default()
            }],
            MaterialSettings {
                sort: 1.,
                portal: true,
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let portal = ShaderBinding::material(material, LightSource::None);
    let (mut geometry, visibility) = world_surfaces::inputs(1);
    geometry.surfaces[0].light_source = LightSource::None;
    geometry.surfaces[0].styles = [255; 4];
    let surface = [SurfaceMaterial {
        shader: portal,
        ..Default::default()
    }];
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput::new(&surface)],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let mut scratch = qa_render::world::WorldView::load(assets.world(world).unwrap());
    let mut front = FrontEnd::load(Limits {
        model_choices: 3,
        model_joints: 0,
        ..Default::default()
    })
    .unwrap();
    let mut frame = front.begin_frame(CLEAR.to_le_bytes()).unwrap();
    assert!(frame.add_entity(SceneEntity {
        origin: Vec3([1., 0., 0.]),
        model: SceneModel::Portal {
            camera: Vec3([1., 0., 0.]),
            rotation: 0
        },
        ..Default::default()
    }));
    assert!(frame.add_model_lods(
        SceneEntity {
            origin: Vec3([100., 0., 0.]),
            ..Default::default()
        },
        Some(2.),
        [[Some(parts[0]), Some(parts[1]), Some(parts[2])]]
    ));
    assert!(frame.add_world(
        world,
        presentation,
        &[VisibleSurface {
            surface: 0,
            depth_key: 1
        }]
    ));
    assert!(frame.render_world_scene(
        world,
        presentation,
        view(0, 0., ModelLodSettings::default()),
        &[],
        &mut scratch,
        &assets,
        ShaderView::direct()
    ));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &());
    let views: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect();
    assert_eq!(views.len(), 2);
    assert!(views[0].refdef.portal && views[0].refdef.reflected_view);
    assert!(!views[1].refdef.portal);
    let choices = views
        .iter()
        .map(|view| {
            packet
                .entities(view.scene.entities)
                .iter()
                .find_map(|entity| match entity.model {
                    SceneModel::Lods { choices, .. } => Some(choices),
                    _ => None,
                })
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(choices[0], choices[1]);
    for (view, expected) in views.iter().zip([parts[0].model, parts[2].model]) {
        let meshes: Vec<ModelId> = packet
            .draws(view.scene.draws)
            .iter()
            .filter_map(|draw| draw.mesh)
            .collect();
        assert_eq!(meshes, [expected]);
    }
    let mut cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.views, 2);
    assert_eq!(stats.rejected, 0, "{stats:?}");
    assert_eq!(cpu.pixels()[8 * 16 + 8], BLUE);
}
