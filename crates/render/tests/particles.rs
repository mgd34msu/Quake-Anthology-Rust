use qa_core::primitives::Vec3;
use qa_render::{
    Assets, Command, FrontEnd, Limits, PaletteId, ParticlePresentation, Refdef, SceneParticle,
    ShaderBindingId,
    assets::{Cull, Filter, StageTexture, Wrap},
    particles::{gl_vertices, register_point_shader},
    shader::{AlphaGen, RgbGen, StageBlend},
    surface_cache::PaletteLighting,
};

fn resources() -> (Assets, PaletteId, ShaderBindingId) {
    let mut assets = Assets::load().unwrap();
    let rgb: Vec<u8> = (0..256)
        .flat_map(|index| [index as u8, (index / 2) as u8, (255 - index) as u8])
        .collect();
    let palette = assets
        .register_palette(PaletteLighting::load(&rgb, &vec![0; 64 * 256], None, 256).unwrap())
        .unwrap();
    let material = register_point_shader(&mut assets, "particles/native-point").unwrap();
    (assets, palette, material)
}

#[test]
fn point_material_uses_the_original_mask_and_the_shared_stage_table() {
    let (assets, _, material) = resources();
    let material = assets
        .material(assets.shader_binding(material).unwrap().material)
        .unwrap();
    assert_eq!(material.settings.cull, Cull::None);
    assert_eq!(material.stages.len(), 1);
    let stage = material.stages[0];
    assert_eq!(stage.rgb_gen, RgbGen::ExactVertex);
    assert_eq!(stage.alpha_gen, AlphaGen::Vertex);
    assert_eq!(stage.blend, Some(StageBlend::ALPHA));
    assert!(stage.depth_write);
    assert_eq!(stage.sampler.wrap, Wrap::Repeat);
    assert_eq!(stage.sampler.filter, Filter::Linear);
    assert!(!stage.sampler.mipmaps);
    let StageTexture::Image(image) = stage.texture else {
        panic!()
    };
    let image = assets.image(image).unwrap();
    assert_eq!((image.width, image.height), (8, 8));
    let filled = [
        (1, 0),
        (2, 0),
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 1),
        (0, 2),
        (1, 2),
        (2, 2),
        (3, 2),
        (1, 3),
        (2, 3),
    ];
    for y in 0..8 {
        for x in 0..8 {
            let pixel: [u8; 4] = image.rgba[(y * 8 + x) * 4..][..4].try_into().unwrap();
            assert_eq!(
                pixel,
                [
                    255,
                    255,
                    255,
                    if filled.contains(&(x, y)) { 255 } else { 0 }
                ]
            );
        }
    }
}

#[test]
fn gl_point_vertices_match_native_scale_order_axes_uvs_and_palette_color() {
    let (assets, palette, material) = resources();
    let view = Refdef {
        origin: Vec3([-3.5, 5.0, -7.25]),
        axes: [
            Vec3([0.0, 1.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
        ],
        ..Refdef::default()
    };
    for depth in [
        f32::from_bits(20.0_f32.to_bits() - 1),
        20.0,
        20.25,
        256.0,
        -3.0,
    ] {
        let particle = SceneParticle {
            position: Vec3([11.5, view.origin.0[1] + depth, -0.0]),
            color: 73,
            alpha: 123,
            palette,
            shader: material.into(),
            ..SceneParticle::default()
        };
        let output = gl_vertices(particle, &view, assets.palette(palette).unwrap()).unwrap();
        let vertices = output.vertices();
        assert_eq!(vertices.len(), 3);
        let org = particle.position.0;
        let eye = view.origin.0;
        let axis = view.axes[0].0;
        let d =
            (org[0] - eye[0]) * axis[0] + (org[1] - eye[1]) * axis[1] + (org[2] - eye[2]) * axis[2];
        let scale = if d < 20.0 {
            1.0
        } else {
            (1.0 + f64::from(d) * 0.004) as f32
        };
        let up = view.axes[2].0.map(|value| value * 1.5);
        let right = view.axes[1].0.map(|value| -value * 1.5);
        let expected: [[f32; 3]; 3] = [
            org,
            std::array::from_fn(|i| org[i] + up[i] * scale),
            std::array::from_fn(|i| org[i] + right[i] * scale),
        ];
        for (vertex, expected) in vertices.iter().zip(expected) {
            assert_eq!(
                vertex.position.0.map(f32::to_bits),
                expected.map(f32::to_bits)
            );
            assert_eq!(vertex.color, [73, 36, 182, 123]);
        }
        assert_eq!(
            vertices.iter().map(|v| v.texcoord).collect::<Vec<_>>(),
            [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
        );
        // Software pixel aspect changes its footprint, not the native GL triangle.
        let alternate = gl_vertices(
            SceneParticle {
                presentation: ParticlePresentation::Square { pixel_aspect: 2.0 },
                ..particle
            },
            &view,
            assets.palette(palette).unwrap(),
        )
        .unwrap();
        assert_eq!(alternate.vertices(), vertices);
    }
}

#[test]
fn card_uses_the_same_palette_and_fixed_vertex_output() {
    let (assets, palette, material) = resources();
    let particle = SceneParticle {
        position: Vec3([8.0, 11.0, 13.0]),
        color: 10,
        alpha: 64,
        size: 4.0,
        palette,
        shader: material.into(),
        presentation: ParticlePresentation::Card,
    };
    let output = gl_vertices(
        particle,
        &Refdef::default(),
        assets.palette(palette).unwrap(),
    )
    .unwrap();
    assert_eq!(
        output
            .vertices()
            .iter()
            .map(|v| v.position.0)
            .collect::<Vec<_>>(),
        [
            [8.0, 13.0, 11.0],
            [8.0, 9.0, 11.0],
            [8.0, 9.0, 15.0],
            [8.0, 13.0, 15.0]
        ]
    );
    assert!(
        output
            .vertices()
            .iter()
            .all(|v| v.color == [10, 5, 245, 64])
    );
}

#[test]
fn particles_are_copied_bounded_and_kept_in_separate_view_ranges() {
    let (assets, palette, material) = resources();
    let mut front = FrontEnd::load(Limits {
        particles: 2,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let mut particle = SceneParticle {
        position: Vec3([17.0, 2.0, 3.0]),
        palette,
        shader: material.into(),
        ..SceneParticle::default()
    };
    assert!(frame.add_particle(particle));
    particle.position = Vec3([31.0, 7.0, 11.0]);
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    frame.clear_scene();
    assert!(frame.add_particle(particle));
    assert!(!frame.add_particle(particle));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(
        (first.scene.particles.first, first.scene.particles.count),
        (0, 1)
    );
    assert_eq!(
        (second.scene.particles.first, second.scene.particles.count),
        (1, 1)
    );
    assert_eq!(
        packet.particles(first.scene.particles)[0].position,
        Vec3([17.0, 2.0, 3.0])
    );
    assert_eq!(packet.particles(second.scene.particles)[0], particle);
    assert_eq!(packet.rejected, 1);
    assert!(front.recycle(packet).is_ok());
    let empty = front.begin_frame([0; 4]).unwrap().finish();
    assert!(front.recycle(empty).is_ok());
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_particle(particle));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        (view.scene.particles.first, view.scene.particles.count),
        (0, 1)
    );
    assert_eq!(packet.rejected, 0);
}

#[test]
fn failed_view_and_invalid_particle_do_not_damage_valid_peers_or_prior_views() {
    let (assets, palette, material) = resources();
    let mut front = FrontEnd::load(Limits {
        area_bytes: 1,
        particles: 3,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let particle = SceneParticle {
        palette,
        shader: material.into(),
        ..SceneParticle::default()
    };
    assert!(frame.add_particle(particle));
    assert!(!frame.render_scene(
        Refdef::default(),
        &[1, 2],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_particle(SceneParticle {
        palette: PaletteId(u32::MAX),
        ..particle
    }));
    assert!(frame.add_particle(SceneParticle {
        color: 9,
        ..particle
    }));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_particle(SceneParticle {
        shader: qa_render::ShaderBindingId(u32::MAX).into(),
        ..particle
    }));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    let Command::View(third) = packet.commands()[3] else {
        panic!()
    };
    assert_eq!(packet.particles(first.scene.particles), &[particle]);
    assert_eq!(
        packet.particles(second.scene.particles),
        &[SceneParticle {
            color: 9,
            ..particle
        }]
    );
    assert_eq!(third.scene.particles.count, 0);
    assert_eq!(packet.rejected, 3);
}

#[test]
fn nonfinite_or_nonpositive_particle_admission_leaves_capacity_for_valid_data() {
    let (assets, palette, material) = resources();
    let particle = SceneParticle {
        palette,
        shader: material.into(),
        ..SceneParticle::default()
    };
    let mut front = FrontEnd::load(Limits {
        particles: 1,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for bad in [
        SceneParticle {
            position: Vec3([f32::NAN, 0.0, 0.0]),
            ..particle
        },
        SceneParticle {
            size: 0.0,
            ..particle
        },
        SceneParticle {
            size: f32::INFINITY,
            ..particle
        },
        SceneParticle {
            presentation: ParticlePresentation::Square { pixel_aspect: -1.0 },
            ..particle
        },
    ] {
        assert!(!frame.add_particle(bad));
    }
    assert!(frame.add_particle(particle));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(packet.particles(view.scene.particles), &[particle]);
    assert_eq!(packet.rejected, 4);
}
