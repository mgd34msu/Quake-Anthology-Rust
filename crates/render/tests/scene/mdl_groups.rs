use super::*;
use qa_core::primitives::ThinkTime;
use qa_render::{
    CommandList, ModelId,
    assets::{ModelAnimation, TimedGroup},
    scene::{FrameTiming, SkinTiming},
    shader::reference::ShaderView,
};
use std::sync::Arc;

const ENDPOINTS: [f32; 5] = [0.125, 0.375, 1., 0.25, 0.75];

fn groups() -> [TimedGroup; 4] {
    [(0, 1, 0, 0), (1, 3, 0, 3), (4, 1, 3, 0), (5, 2, 3, 2)].map(
        |(first, count, endpoint, endpoints)| TimedGroup {
            members: Span { first, count },
            endpoints: Span {
                first: endpoint,
                count: endpoints,
            },
            bounds: Bounds {
                mins: Vec3([8., -5., -2.]),
                maxs: Vec3([8., 5., 2.]),
            },
        },
    )
}

fn recipe(
    frames: &[TimedGroup],
    endpoints: &[f32],
    physical: u32,
) -> Result<ModelAnimation, &'static str> {
    ModelAnimation::new(
        frames.into(),
        Box::new([TimedGroup {
            members: Span { first: 0, count: 1 },
            endpoints: Span::default(),
            bounds: Bounds::default(),
        }]),
        endpoints.into(),
        1,
        physical,
        1,
    )
}

fn animated(endpoints: &[f32]) -> (Assets, ModelId) {
    animated_model(recipe(&groups(), endpoints, 7).unwrap())
}

fn animated_model(animation: ModelAnimation) -> (Assets, ModelId) {
    let mut assets = Assets::load().unwrap();
    let material = assets
        .register_material(
            "groups/colored poses",
            &[Stage {
                rgb_gen: RgbGen::ExactVertex,
                ..Default::default()
            }],
            MaterialSettings {
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let shader = assets
        .register_shader_binding(qa_render::ShaderBinding::material(
            material,
            LightSource::None,
        ))
        .unwrap();
    let vertices = (0..7u8)
        .flat_map(|frame| {
            [[8., -2., -2.], [8., 2., -2.], [8., 0., 2.]].map(|mut point| {
                point[1] += f32::from(frame) - 3.;
                Vertex {
                    position: Vec3(point),
                    normal: Vec3([-1., 0., 0.]),
                    color: [200 - frame * 20, 20 + frame * 25, 40, 255],
                    ..Default::default()
                }
            })
        })
        .collect::<Vec<_>>();
    let model = assets
        .register_animated_model(&vertices, 3, &[0, 1, 2], shader)
        .unwrap();
    assets
        .set_model_animation(model, Arc::new(animation))
        .unwrap();
    (assets, model)
}

fn pose(model: ModelId) -> SceneEntity {
    SceneEntity {
        model: model.into(),
        frame: 1,
        old_frame: 3,
        ..Default::default()
    }
}

fn capture(assets: &Assets, entities: &[SceneEntity], views: &[Refdef]) -> CommandList {
    capture_with(assets, views, |frame| {
        for &entity in entities {
            assert!(frame.add_entity(entity));
        }
    })
}

pub(super) fn capture_with(
    assets: &Assets,
    views: &[Refdef],
    mut add: impl FnMut(&mut qa_render::Frame),
) -> CommandList {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    for &view in views {
        add(&mut frame);
        assert!(frame.render_scene(view, &[], assets, ShaderView::direct()));
    }
    frame.finish()
}

fn pairs(packet: &CommandList) -> Vec<[u32; 2]> {
    packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(packet.draws(view.scene.draws)),
            _ => None,
        })
        .flatten()
        .filter(|draw| draw.kind == DrawKind::Entity)
        .map(|draw| {
            let pose = draw.frames.unwrap();
            [pose.current, pose.previous]
        })
        .collect()
}

#[test]
fn cumulative_groups_keep_parent_indices_float_narrowing_endpoints_and_negative_time() {
    let (assets, model) = animated(&ENDPOINTS);
    assert_eq!(assets.model(model).unwrap().frame_count(), 7);
    assert_eq!(assets.model(model).unwrap().native_frame_count(), 4);
    let below = f64::from_bits(0.125f64.to_bits() - 1);
    assert!(below < 0.125);
    assert_eq!((below as f32).to_bits(), 0.125f32.to_bits());
    let cases = [
        (0., [1, 5]),
        (0.124_999_99, [1, 5]),
        (below, [2, 5]),
        (0.125, [2, 5]),
        (0.375, [3, 6]),
        (1., [1, 6]),
        (1.125, [2, 6]),
        (-0.125, [1, 5]),
        (-1., [1, 5]),
    ];
    let views = cases.map(|(time, _)| Refdef {
        time_ms: 90_000,
        model_time: Some(ThinkTime::Seconds(time)),
        ..Default::default()
    });
    let entity = pose(model);
    let packet = capture(&assets, &[entity], &views);
    assert_eq!(pairs(&packet), cases.map(|(_, pair)| pair));
    assert_eq!(packet.rejected, 0);
    for stored in packet.entities(Span {
        first: 0,
        count: cases.len() as u32,
    }) {
        assert_eq!(
            (stored.frame, stored.old_frame, stored.sync_base),
            (1, 3, 0.)
        );
    }
    let single = SceneEntity {
        frame: 2,
        old_frame: 0,
        ..entity
    };
    assert_eq!(pairs(&capture(&assets, &[single], &[views[4]])), [[4, 0]]);
    let fallback_clock = capture(
        &assets,
        &[entity],
        &[Refdef {
            time_ms: 125,
            ..Default::default()
        }],
    );
    assert_eq!(pairs(&fallback_clock), [[2, 5]]);

    // Positive authored endpoints may be nonmonotonic; they are never sorted.
    let (assets, model) = animated(&[0.5, 0.25, 0.75, 0.25, 0.75]);
    let packet = capture(
        &assets,
        &[pose(model)],
        &[0.3, 0.5].map(|time| Refdef {
            model_time: Some(ThinkTime::Seconds(time)),
            ..Default::default()
        }),
    );
    assert_eq!(pairs(&packet), [[1, 6], [3, 6]]);
}

#[test]
fn uniform_first_uses_its_own_intervals_and_ignores_independent_sync_inputs() {
    let (assets, model) = animated(&ENDPOINTS);
    let entities = [0., 0.25, f32::NAN].map(|sync_base| SceneEntity {
        sync_base,
        ..pose(model)
    });
    let view = Refdef {
        model_time: Some(ThinkTime::Seconds(0.05)),
        ..Default::default()
    };
    let cumulative = capture(&assets, &entities, &[view]);
    assert_eq!(pairs(&cumulative), [[1, 5], [2, 6]]);
    assert_eq!(cumulative.rejected, 1);
    let uniform = capture(
        &assets,
        &entities,
        &[Refdef {
            frame_timing: FrameTiming::UniformFirst,
            ..view
        }],
    );
    assert_eq!(pairs(&uniform), [[1, 5]; 3]);
    assert_eq!(uniform.rejected, 0);
    let views = [FrameTiming::Cumulative, FrameTiming::UniformFirst].map(|frame_timing| Refdef {
        model_time: Some(ThinkTime::Seconds(0.375)),
        frame_timing,
        ..Default::default()
    });
    assert_eq!(
        pairs(&capture(&assets, &[pose(model)], &views)),
        [[3, 6], [1, 6]]
    );
}

#[test]
fn malformed_recipes_and_overflow_drop_only_the_affected_grouped_draw() {
    let original = groups();
    for endpoints in [
        [0., 0.375, 1., 0.25, 0.75],
        [-1., 0.375, 1., 0.25, 0.75],
        [f32::NAN, 0.375, 1., 0.25, 0.75],
    ] {
        assert!(recipe(&original, &endpoints, 7).is_err());
    }
    let mut invalid = original;
    invalid[1].members.first = 2;
    assert!(recipe(&invalid, &ENDPOINTS, 7).is_err());
    invalid = original;
    invalid[1].endpoints.first = u32::MAX;
    assert!(recipe(&invalid, &ENDPOINTS, 7).is_err());
    let (assets, model) = animated(&ENDPOINTS);
    for (timing, time) in [
        (FrameTiming::Cumulative, 2_147_483_648.),
        (FrameTiming::UniformFirst, 268_435_456.),
    ] {
        let healthy = SceneEntity {
            frame: 0,
            old_frame: 0,
            ..pose(model)
        };
        let packet = capture(
            &assets,
            &[pose(model), healthy],
            &[Refdef {
                frame_timing: timing,
                model_time: Some(ThinkTime::Seconds(time)),
                ..Default::default()
            }],
        );
        assert_eq!(pairs(&packet), [[0, 0]]);
        assert_eq!(packet.rejected, 1);
    }
    let invalid_parent = SceneEntity {
        frame: 4,
        ..pose(model)
    };
    let packet = capture(
        &assets,
        &[invalid_parent, pose(model)],
        &[Refdef::default()],
    );
    assert_eq!(pairs(&packet), [[1, 5]]);
    assert_eq!(packet.rejected, 1);
}

#[test]
fn copied_fractional_clocks_seal_distinct_poses_and_cpu_pixels_in_either_view_order() {
    let (assets, model) = animated(&ENDPOINTS);
    let source = SceneEntity {
        back_lerp: 0.25,
        ..pose(model)
    };
    let views = [0.05, 0.5]
        .into_iter()
        .enumerate()
        .map(|(seat, time)| Refdef {
            viewport: qa_render::Viewport {
                x: seat as u32 * 16,
                y: 0,
                width: 16,
                height: 16,
            },
            fov: [90.; 2],
            near: 1.,
            time_ms: 90_000,
            model_time: Some(ThinkTime::Seconds(time)),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let packet = capture(&assets, &[source], &views);
    assert_eq!(pairs(&packet), [[1, 5], [3, 6]]);
    let mesh = assets.model(model).unwrap();
    for (pair, expected_y) in pairs(&packet).into_iter().zip([-3., -1.25]) {
        assert_eq!(
            mesh.frames(pair[0], pair[1], source.back_lerp)
                .unwrap()
                .vertex(0)
                .position,
            Vec3([8., expected_y, -2.])
        );
    }
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(32, 16, &assets).unwrap();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert!(stats.triangles >= 2);
    let pixels = cpu.pixels().to_vec();
    let depths = cpu
        .inverse_depth()
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();
    for (seat, color) in [[180, 45, 40, 255], [140, 95, 40, 255]]
        .into_iter()
        .enumerate()
    {
        assert!(
            pixels.iter().enumerate().any(
                |(index, &pixel)| index % 32 / 16 == seat && pixel == u32::from_le_bytes(color)
            )
        );
    }
    let reverse = capture(&assets, &[source], &[views[1], views[0]]);
    assert_eq!(pairs(&reverse), [[3, 6], [1, 5]]);
    assert_eq!(cpu.render(&reverse, &assets).rejected, 0);
    assert_eq!(cpu.pixels(), pixels);
    assert_eq!(
        cpu.inverse_depth()
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        depths
    );
    for stored in packet.entities(Span { first: 0, count: 2 }) {
        assert_eq!(
            (
                stored.frame,
                stored.old_frame,
                stored.back_lerp,
                stored.sync_base
            ),
            (1, 3, 0.25, 0.)
        );
    }
}

#[test]
fn portal_child_reselects_from_original_parents_with_the_copied_clock_and_timing() {
    let (mut assets, model) = animated(&ENDPOINTS);
    let portal = assets
        .register_material(
            "groups/portal",
            &[Stage {
                alpha_gen: AlphaGen::Const(0.),
                blend: Some(StageBlend::ALPHA),
                depth_write: false,
                ..Default::default()
            }],
            MaterialSettings {
                portal: true,
                sort: 1.,
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let world = world(
        &mut assets,
        &[SurfaceMaterial {
            shader: qa_render::ShaderBinding::material(portal, LightSource::None),
            ..Default::default()
        }],
    );
    let presentation = presentation(&assets, world);
    let mut scratch = qa_render::world::WorldView::load(assets.world(world).unwrap());
    for (timing, expected) in [
        (FrameTiming::Cumulative, [3, 6]),
        (FrameTiming::UniformFirst, [2, 5]),
    ] {
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = frontend.begin_frame([0; 4]).unwrap();
        assert!(frame.add_entity(SceneEntity {
            sync_base: 0.25,
            ..pose(model)
        }));
        assert!(frame.add_entity(SceneEntity {
            origin: Vec3([1., 0., 0.]),
            model: SceneModel::Portal {
                camera: Vec3([1., 0., 0.]),
                rotation: 0
            },
            ..Default::default()
        }));
        assert!(frame.add_world(
            world,
            presentation,
            &[VisibleSurface {
                surface: 0,
                depth_key: 0
            }]
        ));
        let view = Refdef {
            viewport: qa_render::Viewport {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
            near: 0.25,
            model_time: Some(ThinkTime::Seconds(0.125)),
            frame_timing: timing,
            time_ms: 90_000,
            ..Default::default()
        };
        assert!(frame.render_world_scene(
            world,
            presentation,
            view,
            &[],
            &mut scratch,
            &assets,
            ShaderView::direct()
        ));
        let packet = frame.finish();
        let views = packet
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::View(view) => Some(view),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(views.len(), 2);
        assert!(views[0].refdef.portal);
        assert!(!views[1].refdef.portal);
        assert_eq!(pairs(&packet), [expected; 2]);
        for captured in views {
            assert_eq!(captured.refdef.model_time, view.model_time);
            assert_eq!(captured.refdef.frame_timing, timing);
            let stored = packet
                .entities(captured.scene.entities)
                .iter()
                .find(|entity| matches!(entity.model, SceneModel::Mesh(_)))
                .unwrap();
            assert_eq!(
                (stored.frame, stored.old_frame, stored.sync_base),
                (1, 3, 0.25)
            );
        }
        assert_eq!(packet.rejected, 0);
    }
}

pub(super) fn skin_models() -> (Assets, ModelId, [ShaderBindingId; 12]) {
    let mut endpoints = ENDPOINTS.to_vec();
    endpoints.extend([
        0.125, 0.5, 0.125, 0.375, 1., 0.125, 0.25, 0.5, 0.75, 1., 1.5,
    ]);
    let skins = [(0, 1, 5, 0), (1, 2, 5, 2), (3, 3, 7, 3), (6, 6, 10, 6)].map(
        |(first, count, endpoint, endpoints)| TimedGroup {
            members: Span { first, count },
            endpoints: Span {
                first: endpoint,
                count: endpoints,
            },
            bounds: Bounds::default(),
        },
    );
    let animation = ModelAnimation::new(
        Box::new(groups()),
        Box::new(skins),
        endpoints.into_boxed_slice(),
        1,
        7,
        12,
    )
    .unwrap();
    let (mut assets, model) = animated_model(animation);
    let shaders = std::array::from_fn(|index| {
        let image = assets.register_image(1, 1, &skin_color(index)).unwrap();
        let material = assets
            .register_material(
                &format!("groups/skin-{index}"),
                &[Stage {
                    texture: qa_render::assets::StageTexture::Image(image),
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: qa_render::shader::Cull::None,
                    ..Default::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                LightSource::None,
            ))
            .unwrap()
    });
    (assets, model, shaders)
}

fn skin_color(index: usize) -> [u8; 4] {
    [
        index as u8 * 17,
        255 - index as u8 * 13,
        index as u8 * 19,
        255,
    ]
}

fn skin_pose() -> SceneEntity {
    SceneEntity {
        frame: 0,
        old_frame: 0,
        ..Default::default()
    }
}

fn selected_skins(packet: &CommandList) -> Vec<ShaderBindingId> {
    packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(packet.draws(view.scene.draws)),
            _ => None,
        })
        .flatten()
        .filter(|draw| draw.kind == DrawKind::Entity)
        .map(|draw| draw.shader.unwrap().source(&()).unwrap())
        .collect()
}

fn assert_skin_pixels(assets: &Assets, packet: &CommandList, expected: &[usize]) {
    let width = expected.len() as u32 * 16;
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(width, 16, assets).unwrap();
    let stats = cpu.render(packet, assets);
    assert_eq!(stats.rejected, 0);
    assert!(stats.triangles >= expected.len() as u32);
    for (seat, &skin) in expected.iter().enumerate() {
        assert!(
            cpu.pixels()
                .iter()
                .enumerate()
                .any(|(index, &pixel)| index % width as usize / 16 == seat
                    && pixel == u32::from_le_bytes(skin_color(skin)))
        );
    }
}

#[test]
fn grouped_skin_cumulative_boundaries_sync_and_copied_views_select_real_cpu_materials() {
    let (assets, model, shaders) = skin_models();
    let below = f64::from_bits(0.125f64.to_bits() - 1);
    let cases = [
        (0., 3),
        (below, 4),
        (0.125, 4),
        (0.375, 5),
        (1., 3),
        (-0.125, 3),
    ];
    let views = cases
        .into_iter()
        .enumerate()
        .map(|(index, (time, _))| Refdef {
            viewport: qa_render::Viewport {
                x: index as u32 * 16,
                y: 0,
                width: 16,
                height: 16,
            },
            fov: [90.; 2],
            near: 1.,
            time_ms: 90_000,
            model_time: Some(ThinkTime::Seconds(time)),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let packet = capture_with(&assets, &views, |frame| {
        assert!(model_choices::skin(
            frame,
            skin_pose(),
            model,
            2,
            shaders[3..6].iter().copied().map(Into::into)
        ));
    });
    assert_eq!(
        selected_skins(&packet),
        cases.map(|(_, skin)| shaders[skin])
    );
    assert_eq!(packet.rejected, 0);
    assert_skin_pixels(&assets, &packet, &cases.map(|(_, skin)| skin));
    for entity in packet.entities(Span {
        first: 0,
        count: cases.len() as u32,
    }) {
        assert!(entity.shader.is_none());
        assert_eq!(
            (entity.frame, entity.old_frame, entity.sync_base),
            (0, 0, 0.)
        );
        assert!(matches!(entity.model, SceneModel::Choice { .. }));
    }
    let sync = capture_with(
        &assets,
        &[Refdef {
            model_time: Some(ThinkTime::Seconds(0.05)),
            ..Default::default()
        }],
        |frame| {
            for sync_base in [0., 0.25] {
                assert!(model_choices::skin(
                    frame,
                    SceneEntity {
                        sync_base,
                        ..skin_pose()
                    },
                    model,
                    2,
                    shaders[3..6].iter().copied().map(Into::into)
                ));
            }
        },
    );
    assert_eq!(selected_skins(&sync), [shaders[3], shaders[4]]);
    let group_boundaries = capture_with(
        &assets,
        &[0.125, 0.5, 1.5].map(|time| Refdef {
            model_time: Some(ThinkTime::Seconds(time)),
            ..Default::default()
        }),
        |frame| {
            assert!(model_choices::skin(
                frame,
                skin_pose(),
                model,
                1,
                shaders[1..3].iter().copied().map(Into::into)
            ));
            assert!(model_choices::skin(
                frame,
                skin_pose(),
                model,
                3,
                shaders[6..12].iter().copied().map(Into::into)
            ));
        },
    );
    assert_eq!(
        selected_skins(&group_boundaries),
        [
            shaders[2], shaders[7], shaders[1], shaders[9], shaders[1], shaders[6]
        ]
    );
}

#[test]
fn legacy_four_keeps_final_overwritten_slots_repeats_short_groups_and_ignores_sync() {
    let (assets, model, shaders) = skin_models();
    let views = [0., 0.1, 0.2, 0.3, 0.4, -0.1].map(|time| Refdef {
        model_time: Some(ThinkTime::Seconds(time)),
        skin_timing: SkinTiming::LegacyFour,
        ..Default::default()
    });
    let packet = capture_with(&assets, &views, |frame| {
        for (skin, range, sync_base) in [(1, 1..3, 0.), (2, 3..6, 0.25), (3, 6..12, f32::NAN)] {
            assert!(model_choices::skin(
                frame,
                SceneEntity {
                    sync_base,
                    ..skin_pose()
                },
                model,
                skin,
                shaders[range].iter().copied().map(Into::into)
            ));
        }
    });
    let expected = [
        [1, 3, 10],
        [2, 4, 11],
        [1, 5, 8],
        [2, 3, 9],
        [1, 3, 10],
        [2, 3, 9],
    ];
    assert_eq!(
        selected_skins(&packet),
        expected
            .into_iter()
            .flatten()
            .map(|index| shaders[index])
            .collect::<Vec<_>>()
    );
    assert_eq!(packet.rejected, 0);
    let views = [0., 0.1, 0.2]
        .into_iter()
        .enumerate()
        .map(|(seat, time)| Refdef {
            viewport: qa_render::Viewport {
                x: seat as u32 * 16,
                y: 0,
                width: 16,
                height: 16,
            },
            near: 1.,
            fov: [90.; 2],
            model_time: Some(ThinkTime::Seconds(time)),
            skin_timing: SkinTiming::LegacyFour,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let pixels = capture_with(&assets, &views, |frame| {
        assert!(model_choices::skin(
            frame,
            SceneEntity {
                sync_base: 0.875,
                ..skin_pose()
            },
            model,
            3,
            shaders[6..12].iter().copied().map(Into::into)
        ));
    });
    assert_skin_pixels(&assets, &pixels, &[10, 11, 8]);
}

#[test]
fn timed_skin_custom_shader_and_bounded_choice_capture_preserve_atomic_admission() {
    let (assets, model, shaders) = skin_models();
    assert_eq!(assets.max_model_choices(), 7);
    let grown = Limits {
        entities: 3,
        model_choices: 0,
        model_joints: 0,
        ..Default::default()
    }
    .for_assets(&assets)
    .unwrap();
    assert!(grown.model_choices >= 21);
    assert_eq!(grown.draws, 0);
    let mut frontend = FrontEnd::load(Limits {
        entities: 2,
        model_choices: 3,
        model_joints: 0,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(!model_choices::skin(
        &mut frame,
        skin_pose(),
        model,
        3,
        shaders[6..12].iter().copied().map(Into::into)
    ));
    assert!(model_choices::skin(
        &mut frame,
        skin_pose(),
        model,
        1,
        shaders[1..3].iter().copied().map(Into::into)
    ));
    assert!(frame.render_scene(
        Refdef {
            model_time: Some(ThinkTime::Seconds(0.125)),
            ..Default::default()
        },
        &[],
        &assets,
        ShaderView::direct()
    ));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 1);
    assert_eq!(selected_skins(&packet), [shaders[2]]);
    let Command::View(view) = packet.commands()[1] else {
        panic!("timed model view")
    };
    assert_eq!(view.scene.entities, Span { first: 0, count: 1 });
    let SceneModel::Choice { root } = packet.entities(view.scene.entities)[0].model else {
        panic!("deferred native skin group")
    };
    assert_eq!(root, 2);
    assert_eq!(packet.draws(view.scene.draws)[0].mesh, Some(model));
    let custom = capture_with(
        &assets,
        &[Refdef {
            skin_timing: SkinTiming::LegacyFour,
            ..Default::default()
        }],
        |frame| {
            assert!(model_choices::skin(
                frame,
                SceneEntity {
                    shader: Some(shaders[0].into()),
                    ..skin_pose()
                },
                model,
                3,
                shaders[6..12].iter().copied().map(Into::into)
            ));
        },
    );
    assert_eq!(selected_skins(&custom), [shaders[0]]);
    let single = capture_with(&assets, &[Refdef::default()], |frame| {
        assert!(model_choices::skin(
            frame,
            skin_pose(),
            model,
            0,
            std::iter::once(shaders[0].into())
        ));
    });
    let Command::View(view) = single.commands()[1] else {
        panic!("single skin view")
    };
    assert!(
        matches!(single.entities(view.scene.entities)[0].model, SceneModel::Mesh(id) if id == model)
    );
}
