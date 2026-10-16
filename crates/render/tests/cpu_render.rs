use qa_core::primitives::Vec3;
use qa_render::{
    assets::*,
    cpu::CpuBackend,
    scene::*,
    shader::{BlendFactor, StageBlend},
};

#[test]
fn portal_plane_clips_polygons_and_rotated_models_to_the_same_half_screen() {
    use qa_core::primitives::Plane;
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[240, 120, 60, 255]).unwrap();
    let material = {
        let material = assets
            .register_material(
                "clip plane",
                &[Stage {
                    texture: StageTexture::Image(image),
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
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
    let vertices = [
        Vec3([8., 8., 8.]),
        Vec3([8., -8., 8.]),
        Vec3([8., -8., -8.]),
        Vec3([8., 8., -8.]),
    ]
    .map(|position| Vertex {
        position,
        ..Default::default()
    });
    let model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], material)
        .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut cpu = CpuBackend::load(4, 4).unwrap();
    let background = [1, 2, 3, 255];
    for mesh in [false, true] {
        let mut frame = frontend.begin_frame(background).unwrap();
        if mesh {
            assert!(frame.add_entity(SceneEntity {
                model: SceneModel::Mesh(model),
                origin: Vec3([16., 0., 0.]),
                axes: [Vec3([-1., 0., 0.]), Vec3([0., -1., 0.]), Vec3([0., 0., 1.])],
                ..Default::default()
            }));
        } else {
            assert!(frame.add_poly(material, &vertices));
        }
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 4,
                    height: 4,
                    ..Default::default()
                },
                fov: [90.; 2],
                clip_plane: Some(Plane::oriented(Vec3([0., 1., 0.]), 0.)),
                ..Default::default()
            },
            &[],
            &assets
        ));
        let packet = frame.finish();
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        for row in cpu.pixels().chunks_exact(4) {
            assert_eq!(&row[..2], &[u32::from_le_bytes([240, 120, 60, 255]); 2]);
            assert_eq!(&row[2..], &[u32::from_le_bytes(background); 2]);
        }
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
#[expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "Keep the independent packed-pixel or triangle oracle and incomplete-tail expectations unchanged"
)]
fn fence_holes_ignore_equal_depth_lightmap_pass() {
    let mut assets = Assets::load().unwrap();
    let fence = assets
        .register_image(2, 1, &[255, 255, 255, 0, 255, 255, 255, 255])
        .unwrap();
    let lightmap = assets.register_image(1, 1, &[128, 128, 128, 255]).unwrap();
    let material = {
        let material = assets
            .register_material(
                "fence",
                &[
                    Stage {
                        texture: StageTexture::Image(fence),
                        sampler: Sampler {
                            filter: Filter::Nearest,
                            ..Sampler::default()
                        },
                        alpha_test: AlphaTest::AtLeastHalf,
                        ..Stage::default()
                    },
                    Stage {
                        texture: StageTexture::Image(lightmap),
                        blend: Some(StageBlend {
                            source: BlendFactor::DestinationColor,
                            destination: BlendFactor::Zero,
                        }),
                        texgen: TcGen::Lightmap,
                        depth_func: DepthFunc::Equal,
                        depth_write: false,
                        ..Stage::default()
                    },
                ],
                MaterialSettings {
                    cull: Cull::None,
                    sort: 0.0,
                    ..MaterialSettings::default()
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
    let vertices = [
        (Vec3([8.0, 8.0, 8.0]), [0.0, 0.0]),
        (Vec3([8.0, -8.0, 8.0]), [1.0, 0.0]),
        (Vec3([8.0, -8.0, -8.0]), [1.0, 1.0]),
        (Vec3([8.0, 8.0, -8.0]), [0.0, 1.0]),
    ]
    .map(|(position, texcoord)| Vertex {
        position,
        texcoord,
        ..Vertex::default()
    });
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let background = [40, 80, 120, 255];
    let mut frame = frontend.begin_frame(background).unwrap();
    assert!(frame.add_poly(material, &vertices));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 4,
                height: 4,
                ..Viewport::default()
            },
            fov: [90.0; 2],
            ..Refdef::default()
        },
        &[],
        &assets,
    ));
    let mut cpu = CpuBackend::load(4, 4).unwrap();
    let packet = frame.finish();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    for row in cpu.pixels().chunks_exact(4) {
        assert_eq!(&row[..2], &[u32::from_le_bytes(background); 2]);
        assert_eq!(&row[2..], &[u32::from_le_bytes([128, 128, 128, 255]); 2]);
    }
}

#[test]
fn final_palette_phase_covers_statusbar_after_2d() {
    let assets = Assets::load().unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut cpu = CpuBackend::load(4, 4).unwrap();
    for phase in [BlendPhase::AfterView, BlendPhase::FinalPalette] {
        let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 4,
                    height: 2,
                    ..Viewport::default()
                },
                blend: [1.0, 0.0, 0.0, 0.5],
                blend_phase: phase,
                blend_viewport: Some(Viewport {
                    width: 4,
                    height: 4,
                    ..Viewport::default()
                }),
                ..Refdef::default()
            },
            &[],
            &assets,
        ));
        assert!(frame.draw_2d(Draw2d {
            rect: [0.0, 2.0, 4.0, 2.0],
            color: [0, 0, 255, 255],
            ..Draw2d::default()
        }));
        let packet = frame.finish();
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        for (i, &pixel) in cpu.pixels().iter().enumerate() {
            let rgba = pixel.to_le_bytes();
            let expected_rgb = if i < 8 {
                [128, 0, 0]
            } else if phase == BlendPhase::FinalPalette {
                [128, 0, 128]
            } else {
                [0, 0, 255]
            };
            assert_eq!(&rgba[..3], &expected_rgb);
            if phase == BlendPhase::FinalPalette {
                assert_eq!(rgba[3], 255);
            }
        }
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn two_dimensional_color_alpha_blends_and_submission_failures_are_reported() {
    let assets = Assets::load().unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0, 0, 255, 255]).unwrap();
    assert!(!frame.add_poly(ShaderBindingId(0), &[]));
    assert!(frame.draw_2d(Draw2d {
        rect: [0.0, 0.0, 2.0, 2.0],
        color: [255, 0, 0, 128],
        ..Draw2d::default()
    }));
    let mut cpu = CpuBackend::load(2, 2).unwrap();
    let packet = frame.finish();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 1);
    for pixel in cpu.pixels() {
        assert_eq!(pixel.to_le_bytes(), [128, 0, 127, 191]);
    }
}

#[test]
fn native_first_image_sampler_overrides_later_material_requests() {
    for (registered, requested, expected) in [
        (Wrap::Clamp, Wrap::Repeat, [0, 0, 255, 255]),
        (Wrap::Repeat, Wrap::Clamp, [255, 0, 0, 255]),
    ] {
        let mut assets = Assets::load().unwrap();
        let image = assets
            .register_image(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255])
            .unwrap();
        assets
            .set_image_native_sampler(
                image,
                Sampler {
                    wrap: registered,
                    filter: Filter::Nearest,
                    mipmaps: false,
                },
            )
            .unwrap();
        let material = {
            let material = assets
                .register_material(
                    "reused",
                    &[Stage {
                        texture: StageTexture::Image(image),
                        sampler: Sampler {
                            wrap: requested,
                            ..Sampler::default()
                        },
                        ..Stage::default()
                    }],
                    MaterialSettings::default(),
                )
                .unwrap();
            assets
                .register_shader_binding(qa_render::ShaderBinding::material(
                    material,
                    qa_render::world::geometry::LightSource::TwoDimensional,
                ))
                .unwrap()
        };
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.draw_2d(Draw2d {
            shader: material,
            rect: [0.0, 0.0, 2.0, 2.0],
            texcoords: [1.25, 0.0, 1.25, 0.0],
            ..Draw2d::default()
        }));
        let mut cpu = CpuBackend::load(2, 2).unwrap();
        let packet = frame.finish();
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert!(
            cpu.pixels()
                .iter()
                .all(|pixel| pixel.to_le_bytes() == expected)
        );
    }
}
