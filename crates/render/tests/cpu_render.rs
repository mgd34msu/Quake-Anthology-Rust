use qa_core::primitives::Vec3;
use qa_render::{
    assets::*,
    cpu::CpuBackend,
    scene::*,
    shader::{BlendFactor, StageBlend},
};

#[test]
fn fence_holes_ignore_equal_depth_lightmap_pass() {
    let mut assets = Assets::load();
    let fence = assets
        .register_image(2, 1, &[255, 255, 255, 0, 255, 255, 255, 255])
        .unwrap();
    let lightmap = assets.register_image(1, 1, &[128, 128, 128, 255]).unwrap();
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
    let assets = Assets::load();
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
    let assets = Assets::load();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0, 0, 255, 255]).unwrap();
    assert!(!frame.add_poly(MaterialId(0), &[]));
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
