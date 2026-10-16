use qa_core::primitives::Vec3;
use qa_render::scene::EntityVisibility;
use qa_render::{
    Assets, Command, FrontEnd, Limits, Refdef, SceneEntity, SceneModel, ShaderBindingId, Vertex,
};

#[test]
fn shared_beams_apply_world_or_entity_pose_and_keep_supplied_color() {
    let assets = Assets::load().unwrap();
    for world_space in [false, true] {
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        let axes = [Vec3([0., 1., 0.]), Vec3([-1., 0., 0.]), Vec3([0., 0., 1.])];
        let origin = Vec3([16., 20., 24.]);
        let color = [11, 22, 33, 77];
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::Beam {
                end: origin + Vec3([16., 0., 0.]),
                radius: 2.,
                world_space
            },
            origin,
            axes,
            color,
            shader: Some(ShaderBindingId(0).into()),
            ..Default::default()
        }));
        assert!(frame.render_scene(
            Refdef::default(),
            &[],
            &assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let Command::View(view) = packet.commands()[1] else {
            panic!("beam view")
        };
        assert_eq!(view.scene.polys.count, 12);
        let points = packet
            .polys(view.scene.polys)
            .iter()
            .flat_map(|poly| packet.vertices(poly.vertices));
        for vertex in points {
            assert_eq!(vertex.color, color);
            let local = if world_space {
                vertex.position - origin
            } else {
                qa_core::math::inverse_transform_point(origin, axes, vertex.position)
            };
            assert!(local.0[0] == 0. || local.0[0] == 16.);
            assert!((local.0[1] * local.0[1] + local.0[2] * local.0[2] - 4.).abs() < 0.00002);
        }
    }
}

#[test]
fn native_float_entity_alpha_reaches_cpu_pixels_without_byte_narrowing() {
    let mut assets = Assets::load().unwrap();
    let material = qa_render::entities::register_beam_shader(
        &mut assets,
        qa_render::shader::StageBlend::ALPHA,
    )
    .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Sprite {
            radius: 8.,
            rotation: 0.
        },
        origin: Vec3([16., 0., 0.]),
        color: [255, 0, 0, 76],
        alpha: Some(0.3),
        shader: Some(material.into()),
        ..Default::default()
    }));
    assert!(frame.render_scene(
        Refdef {
            viewport: qa_render::Viewport {
                width: 64,
                height: 64,
                ..Default::default()
            },
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let mut cpu = qa_render::cpu::CpuBackend::load(64, 64).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    let pixel = cpu.pixels()[20 * 64 + 22].to_le_bytes();
    assert_eq!(pixel[..3], [77, 0, 0]);
}

#[test]
fn indexed_beams_use_flat_palette_indices_and_the_native_translucency_table() {
    use qa_render::{
        assets::{MaterialSettings, Stage},
        shader::{AlphaGen, RgbGen},
        surface_cache::{IndexedLighting, PaletteLighting},
    };
    let mut assets = Assets::load().unwrap();
    let rgb: Vec<_> = (0..=255u8).flat_map(|value| [value; 3]).collect();
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    let palette = assets
        .register_palette(
            PaletteLighting::load(&rgb, &shades, Some(&vec![77; 65536]), 256).unwrap(),
        )
        .unwrap();
    let mut authored = PaletteLighting::load(&rgb, &shades, Some(&vec![77; 65536]), 256).unwrap();
    authored.ensure_translucency();
    assert!(
        authored
            .translucency_map()
            .unwrap()
            .iter()
            .all(|&index| index == 77)
    );
    let material = qa_render::entities::register_beam_shader(
        &mut assets,
        qa_render::shader::StageBlend::ALPHA,
    )
    .unwrap();
    let backdrop = {
        let material = assets
            .register_material(
                "beam/background",
                &[Stage {
                    rgb_gen: RgbGen::ExactVertex,
                    alpha_gen: AlphaGen::Entity,
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: qa_render::assets::Cull::None,
                    ..Default::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Sprite {
            radius: 40.,
            rotation: 0.
        },
        origin: Vec3([40., 0., 0.]),
        palette_color: Some((palette, 0)),
        shader: Some(backdrop.into()),
        ..Default::default()
    }));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Beam {
            end: Vec3([30., 0., 0.]),
            radius: 3.,
            world_space: true
        },
        origin: Vec3([16., 0., 0.]),
        color: [33, 33, 33, 76],
        alpha: Some(0.3),
        palette_color: Some((palette, 33)),
        shader: Some(material.into()),
        ..Default::default()
    }));
    assert!(frame.render_scene(
        Refdef {
            viewport: qa_render::Viewport {
                width: 64,
                height: 64,
                ..Default::default()
            },
            cpu_presentation: qa_render::CpuPresentation::Indexed {
                palette,
                lighting: IndexedLighting::BrightestRgb,
                ambient: 0,
                fullbright: false
            },
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let mut cpu = qa_render::cpu::CpuBackend::load(64, 64).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .any(|&pixel| pixel == u32::from_le_bytes([77, 77, 77, 255]))
    );
}

#[test]
fn generated_geometry_uses_one_packet_and_keeps_entity_inputs() {
    let assets = Assets::load().unwrap();
    for (model, count) in [
        (
            SceneModel::Sprite {
                radius: 2.,
                rotation: 0.,
            },
            2,
        ),
        (
            SceneModel::Beam {
                end: Vec3([12., 0., 0.]),
                radius: 4.0,
                world_space: false,
            },
            12,
        ),
        (
            SceneModel::RailCore {
                end: Vec3::default(),
                width: 6,
            },
            2,
        ),
        (
            SceneModel::RailRings {
                end: Vec3::default(),
                width: 16,
                segment_length: 2.,
            },
            6,
        ),
        (
            SceneModel::Lightning {
                end: Vec3([12., 0., 0.]),
            },
            8,
        ),
    ] {
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        assert!(frame.add_entity(SceneEntity {
            model,
            origin: Vec3([8., 0., 0.]),
            color: [200, 100, 50, 255],
            shader: Some(ShaderBindingId(0).into()),
            shader_time: 1.25,
            shader_texcoord: [0.125, 0.5],
            ..Default::default()
        }));
        assert!(frame.render_scene(
            Refdef::default(),
            &[],
            &assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let Command::View(view) = packet.commands()[1] else {
            panic!("view")
        };
        assert_eq!(view.scene.polys.count, count);
        assert_eq!(packet.rejected, 0);
        for poly in packet.polys(view.scene.polys) {
            assert_eq!(poly.entity, Some(0));
            assert!(packet.vertices(poly.vertices).iter().all(|v| v.is_finite()));
            let inputs =
                qa_render::stage::DrawInputs::for_view(&view.refdef).for_poly(poly, &packet);
            assert_eq!(inputs.entity_shader_time, 1.25);
            assert_eq!(inputs.entity_texcoord, [0.125, 0.5]);
            assert_eq!(inputs.view_origin, view.refdef.origin);
        }
    }
}

#[test]
fn generated_capacity_failure_drops_a_whole_entity_and_keeps_a_peer_polygon() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits {
        polys: 2,
        vertices: 6,
        ..Default::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Sprite {
            radius: 2.,
            rotation: 0.
        },
        shader: Some(ShaderBindingId(0).into()),
        ..Default::default()
    }));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!("view")
    };
    assert_eq!(view.scene.polys.count, 1);
    assert_eq!(view.scene.draws.count, 1);
    assert_eq!(packet.rejected, 1);
    assert_eq!(packet.polys(view.scene.polys)[0].entity, None);
}

#[test]
fn model_group_overflow_restores_previous_entity_and_draw_ranges() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits {
        entities: 2,
        ..Default::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(SceneEntity::default()));
    assert!(!frame.add_entities([SceneEntity::default(); 2]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!("view")
    };
    assert_eq!(view.scene.entities.count, 1);
    assert_eq!(view.scene.draws.count, 1);
    assert_eq!(packet.rejected, 1);
}

#[test]
fn view_visibility_filters_native_person_flags_without_dropping_shared_state() {
    let assets = Assets::load().unwrap();
    for portal in [false, true] {
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        for visibility in [
            EntityVisibility::Always,
            EntityVisibility::Direct,
            EntityVisibility::Portal,
            EntityVisibility::Hidden,
        ] {
            assert!(frame.add_entity(SceneEntity {
                visibility,
                ..Default::default()
            }));
        }
        assert!(frame.render_scene(
            Refdef {
                portal,
                ..Default::default()
            },
            &[],
            &assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let Command::View(view) = packet.commands()[1] else {
            panic!("view")
        };
        assert_eq!(view.scene.entities.count, 4);
        assert_eq!(view.scene.draws.count, 2);
        assert_eq!(packet.rejected, 0);
    }
}
