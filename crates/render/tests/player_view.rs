use qa_core::primitives::{PlayerViewFlags, Vec3};
use qa_render::{
    assets::*,
    cpu::CpuBackend,
    scene::*,
    shader::{AlphaGen, RgbGen},
};

fn image_material(assets: &mut Assets, name: &str, image: ImageId) -> MaterialId {
    assets
        .register_material(
            name,
            &[Stage {
                texture: StageTexture::Image(image),
                sampler: Sampler {
                    filter: Filter::Nearest,
                    wrap: Wrap::Clamp,
                    ..Default::default()
                },
                ..Default::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap()
}

fn pattern(assets: &mut Assets) -> MaterialId {
    let pixels: Vec<_> = (0..256)
        .flat_map(|y| (0..256).flat_map(move |x| [x as u8, y as u8, (x ^ y) as u8, 255]))
        .collect();
    let image = assets.register_image(256, 256, &pixels).unwrap();
    image_material(assets, "warp pattern", image)
}

fn quad() -> [Vertex; 4] {
    [
        ([8., 8., 8.], [0., 0.]),
        ([8., -8., 8.], [1., 0.]),
        ([8., -8., -8.], [1., 1.]),
        ([8., 8., -8.], [0., 1.]),
    ]
    .map(|(position, texcoord)| Vertex {
        position: Vec3(position),
        texcoord,
        ..Default::default()
    })
}

#[test]
fn native_software_water_warp_scales_320_by_240_and_stays_inside_its_viewport() {
    let mut assets = Assets::load().unwrap();
    let material = pattern(&mut assets);
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut reference = CpuBackend::load(320, 240).unwrap();
    let mut frame = frontend.begin_frame([1, 2, 3, 255]).unwrap();
    assert!(frame.add_poly(material, &quad()));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 320,
                height: 240,
                ..Default::default()
            },
            fov: [90.; 2],
            ..Default::default()
        },
        &[],
        &assets
    ));
    let packet = frame.finish();
    assert_eq!(reference.render(&packet, &assets).rejected, 0);
    assert!(frontend.recycle(packet).is_ok());
    let viewport = Viewport {
        x: 16,
        y: 12,
        width: 640,
        height: 360,
    };
    let background = u32::from_le_bytes([1, 2, 3, 255]);
    let mut output = CpuBackend::load(672, 392).unwrap();
    let mut frame = frontend.begin_frame([1, 2, 3, 255]).unwrap();
    assert!(frame.add_poly(material, &quad()));
    assert!(frame.render_scene(
        Refdef {
            viewport,
            fov: [90.; 2],
            flags: PlayerViewFlags::UNDERWATER,
            software_waterwarp: true,
            ..Default::default()
        },
        &[],
        &assets
    ));
    let packet = frame.finish();
    assert_eq!(output.render(&packet, &assets).rejected, 0);
    // Original D_WarpScreen, phase zero: native sine-table offsets and the
    // independent width/(width+6), height/(height+6) mappings.
    for (x, y, source_x, source_y) in [
        (0, 0, 1, 1),
        (100, 50, 51, 32),
        (319, 239, 158, 158),
        (639, 359, 316, 236),
        (17, 211, 8, 141),
        (500, 100, 247, 66),
    ] {
        assert_eq!(
            output.pixels()[((viewport.y + y) * 672 + viewport.x + x) as usize],
            reference.pixels()[(source_y * 320 + source_x) as usize],
            "pixel {x},{y}"
        );
    }
    for (x, y) in [(0, 0), (15, 12), (656, 12), (16, 372), (671, 391)] {
        assert_eq!(output.pixels()[(y * 672 + x) as usize], background);
    }
    assert_ne!(
        output.pixels()[(62 * 672 + 116) as usize],
        reference.pixels()[(50 * 320 + 100) as usize]
    );
}

#[test]
fn underwater_flag_requires_explicit_software_policy_and_uv_has_no_invented_effect() {
    let mut assets = Assets::load().unwrap();
    let material = pattern(&mut assets);
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut output = CpuBackend::load(64, 64).unwrap();
    let mut baseline = Vec::new();
    for flags in [
        PlayerViewFlags::default(),
        PlayerViewFlags::UNDERWATER | PlayerViewFlags::ULTRAVIOLET,
    ] {
        let mut frame = frontend.begin_frame([1, 2, 3, 255]).unwrap();
        assert!(frame.add_poly(material, &quad()));
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 64,
                    height: 64,
                    ..Default::default()
                },
                fov: [90.; 2],
                flags,
                ..Default::default()
            },
            &[],
            &assets
        ));
        let packet = frame.finish();
        assert_eq!(output.render(&packet, &assets).rejected, 0);
        if baseline.is_empty() {
            baseline.extend_from_slice(output.pixels());
        } else {
            assert_eq!(output.pixels(), baseline);
        }
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn damage_vignette_uses_native_byte_truncation_border_and_before_hud_phase() {
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[255; 4]).unwrap();
    let hud = assets
        .register_material(
            "hud",
            &[Stage {
                texture: StageTexture::Image(image),
                rgb_gen: RgbGen::Vertex,
                alpha_gen: AlphaGen::Vertex,
                ..Default::default()
            }],
            Default::default(),
        )
        .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut output = CpuBackend::load(24, 24).unwrap();
    let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                x: 2,
                y: 2,
                width: 20,
                height: 20
            },
            blend: [0., 0., 1., 1.],
            damage_blend: [1., 0., 0., 0.5],
            ..Default::default()
        },
        &[],
        &assets
    ));
    assert!(frame.draw_2d(Draw2d {
        rect: [2., 2., 1., 1.],
        material: hud,
        color: [0, 255, 0, 255],
        ..Default::default()
    }));
    let packet = frame.finish();
    assert_eq!(output.render(&packet, &assets).rejected, 0);
    assert_eq!(
        output.pixels()[2 * 24 + 2],
        u32::from_le_bytes([0, 255, 0, 255])
    );
    // alpha truncates .5*255 to127; first pixel center is .5/4 into border.
    let edge = output.pixels()[3 * 24 + 2].to_le_bytes();
    // Native SRC_ALPHA/ONE_MINUS_SRC_ALPHA also blends the alpha channel.
    assert_eq!(edge, [111, 0, 144, 192]);
    assert_eq!(
        output.pixels()[12 * 24 + 12],
        u32::from_le_bytes([0, 0, 255, 255])
    );
    assert_eq!(output.pixels()[0], u32::from_le_bytes([0, 0, 0, 255]));
}

#[test]
fn infrared_changes_only_eligible_models_with_native_indexed_and_rgb_paths() {
    use qa_render::surface_cache::{IndexedLighting, IndexedTexture, PaletteLighting};
    static TEST_MAP: [u8; 256] = [79; 256];
    for indexed in [false, true] {
        let mut assets = Assets::load().unwrap();
        let (image, presentation, normal, infrared) = if indexed {
            let colors: Vec<_> = (0..=255u8).flat_map(|v| [v, v, v]).collect();
            let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
            let palette = assets
                .register_palette(PaletteLighting::load(&colors, &shades, None, 224).unwrap())
                .unwrap();
            let image = assets
                .register_indexed_image(
                    IndexedTexture::load_base(1, 1, &[7], None).unwrap(),
                    palette,
                )
                .unwrap();
            (
                image,
                CpuPresentation::Indexed {
                    palette,
                    lighting: IndexedLighting::Gray,
                    ambient: 0,
                    fullbright: true,
                },
                [7, 7, 7, 255],
                [79, 79, 79, 255],
            )
        } else {
            (
                assets.register_image(1, 1, &[255; 4]).unwrap(),
                CpuPresentation::Rgb,
                [255; 4],
                [255, 0, 0, 255],
            )
        };
        let material = assets
            .register_material(
                "infrared alias",
                &[Stage {
                    texture: StageTexture::Image(image),
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    software_alias: Some(SoftwareAlias {
                        missing_skin: false,
                        infrared_map: Some(&TEST_MAP),
                        fullbright_light: None,
                        alpha_cutoffs: None,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        let model = assets
            .register_model(&quad(), &[0, 1, 2, 0, 2, 3], material)
            .unwrap();
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut cpu = CpuBackend::load(16, 16).unwrap();
        for (eligible, enabled, expected) in [
            (false, true, normal),
            (true, false, normal),
            (true, true, infrared),
        ] {
            let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
            assert!(frame.add_entity(SceneEntity {
                model: SceneModel::Mesh(model),
                infrared_visible: eligible,
                ..Default::default()
            }));
            assert!(frame.render_scene(
                Refdef {
                    viewport: Viewport {
                        width: 16,
                        height: 16,
                        ..Default::default()
                    },
                    fov: [90.; 2],
                    cpu_presentation: presentation,
                    flags: if enabled {
                        PlayerViewFlags::INFRARED
                    } else {
                        PlayerViewFlags::default()
                    },
                    ..Default::default()
                },
                &[],
                &assets
            ));
            let packet = frame.finish();
            assert_eq!(cpu.render(&packet, &assets).rejected, 0);
            assert_eq!(
                cpu.pixels()[8 * 16 + 8].to_le_bytes(),
                expected,
                "indexed={indexed}, eligible={eligible}, enabled={enabled}"
            );
            assert!(frontend.recycle(packet).is_ok());
        }
    }
}

#[test]
fn second_empty_warped_view_does_not_inherit_another_seats_pixels() {
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[240, 30, 60, 255]).unwrap();
    let material = image_material(&mut assets, "first seat", image);
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut cpu = CpuBackend::load(40, 20).unwrap();
    let background = [2, 3, 4, 255];
    let mut frame = frontend.begin_frame(background).unwrap();
    assert!(frame.add_poly(material, &quad()));
    let first = Refdef {
        viewport: Viewport {
            x: 0,
            y: 0,
            width: 16,
            height: 16,
        },
        fov: [90.; 2],
        flags: PlayerViewFlags::UNDERWATER,
        software_waterwarp: true,
        ..Default::default()
    };
    assert!(frame.render_scene(first, &[], &assets));
    frame.clear_scene();
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                x: 20,
                y: 2,
                width: 16,
                height: 16
            },
            ..first
        },
        &[],
        &assets
    ));
    let packet = frame.finish();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.views, 2);
    assert_eq!(
        cpu.pixels()[8 * 40 + 8],
        u32::from_le_bytes([240, 30, 60, 255])
    );
    for y in 2..18 {
        for x in 20..36 {
            assert_eq!(
                cpu.pixels()[y * 40 + x],
                u32::from_le_bytes(background),
                "second seat {x},{y}"
            );
        }
    }
    assert_eq!(cpu.pixels()[19 * 40 + 39], u32::from_le_bytes(background));
}
