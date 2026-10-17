use super::*;
use qa_core::primitives::ThinkTime;
use qa_render::{
    ModelId,
    assets::{ModelAnimation, SpriteFrame, SpriteGeometry, SpriteOrientation, TimedGroup},
    scene::{FrameTiming, SpritePlacement},
};
use std::sync::Arc;

const EXTENTS: [[f32; 4]; 4] = [
    [-2., 2., 2., -2.],
    [-3., 1., 1., -1.],
    [-1., 3., 3., -1.],
    [-4., 1., 1., -2.],
];
const COLORS: [[u8; 4]; 4] = [
    [211, 17, 29, 255],
    [31, 223, 47, 255],
    [53, 67, 229, 255],
    [191, 113, 71, 255],
];

fn sprites() -> (Assets, ModelId, ModelId, [ShaderBindingId; 4]) {
    let mut assets = Assets::load().unwrap();
    let shaders = COLORS.map(|color| {
        let image = assets.register_image(1, 1, &color).unwrap();
        let material = assets
            .register_material(
                &format!("sprites/{color:?}"),
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
    let spr = assets
        .register_sprite(
            SpriteGeometry {
                frames: EXTENTS.map(|extents| SpriteFrame { extents }).into(),
                orientation: SpriteOrientation::ViewParallel,
                beam_length: 0.,
                wrap_frames: false,
            },
            shaders[0],
        )
        .unwrap();
    assets
        .set_model_animation(
            spr,
            Arc::new(
                ModelAnimation::new(
                    [(0, 1, 0, 0), (1, 2, 0, 2), (3, 1, 2, 0)]
                        .map(|(first, count, at, endpoints)| TimedGroup {
                            members: Span { first, count },
                            endpoints: Span {
                                first: at,
                                count: endpoints,
                            },
                            bounds: Bounds::default(),
                        })
                        .into(),
                    Box::new([TimedGroup {
                        members: Span { first: 0, count: 1 },
                        endpoints: Span::default(),
                        bounds: Bounds::default(),
                    }]),
                    Box::new([0.125, 0.5]),
                    1,
                    4,
                    1,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let sp2 = assets
        .register_sprite(
            SpriteGeometry {
                frames: EXTENTS[..2]
                    .iter()
                    .copied()
                    .map(|extents| SpriteFrame { extents })
                    .collect(),
                orientation: SpriteOrientation::ViewParallel,
                beam_length: 0.,
                wrap_frames: true,
            },
            shaders[0],
        )
        .unwrap();
    (assets, spr, sp2, shaders)
}

fn selected(packet: &qa_render::CommandList) -> Vec<(u32, ShaderBindingId, Vec3)> {
    packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .map(|view| {
            let draw = packet.draws(view.scene.draws)[0];
            (
                draw.frames.unwrap().current,
                draw.shader.unwrap().source(&()).unwrap(),
                packet
                    .polys(view.scene.polys)
                    .iter()
                    .flat_map(|poly| packet.vertices(poly.vertices))
                    .find(|vertex| vertex.texcoord == [0., 0.])
                    .unwrap()
                    .position,
            )
        })
        .collect()
}

#[test]
fn spr_groups_keep_extent_and_material_together_with_cumulative_time_under_both_frame_policies() {
    let (assets, model, _, shaders) = sprites();
    let below = f64::from_bits(0.125f64.to_bits() - 1);
    for timing in [FrameTiming::Cumulative, FrameTiming::UniformFirst] {
        let cases = [(0., 1), (below, 2), (0.125, 2), (0.5, 1), (-0.125, 1)];
        let views = cases
            .into_iter()
            .enumerate()
            .map(|(seat, (time, _))| Refdef {
                viewport: qa_render::Viewport {
                    x: seat as u32 * 16,
                    width: 16,
                    height: 16,
                    ..Default::default()
                },
                fov: [90.; 2],
                near: 1.,
                time_ms: 90_000,
                model_time: Some(ThinkTime::Seconds(time)),
                frame_timing: timing,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let pose = SceneEntity {
            origin: Vec3([8., 0., 0.]),
            frame: 1,
            old_frame: 0,
            ..Default::default()
        };
        let capture = |views: &[Refdef]| {
            super::mdl_groups::capture_with(&assets, views, |frame| {
                assert!(model_choices::sprite(
                    frame,
                    pose,
                    model,
                    shaders.into_iter().map(Into::into)
                ));
            })
        };
        let packet = capture(&views);
        assert_eq!(
            selected(&packet),
            cases.map(|(_, physical)| {
                let extents = EXTENTS[physical as usize];
                (
                    physical,
                    shaders[physical as usize],
                    Vec3([8., -extents[0], extents[2]]),
                )
            })
        );
        for index in 0..cases.len() as u32 {
            let source = packet.entity(index);
            assert_eq!(
                (source.frame, source.old_frame, source.sync_base),
                (1, 0, 0.)
            );
            assert!(matches!(source.model, SceneModel::Choice { .. }));
        }
        let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(80, 16, &assets).unwrap();
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        let pixels = cpu.pixels().to_vec();
        for (seat, (_, physical)) in cases.into_iter().enumerate() {
            assert!(
                pixels
                    .iter()
                    .enumerate()
                    .any(|(offset, pixel)| offset % 80 / 16 == seat
                        && *pixel == u32::from_le_bytes(COLORS[physical as usize]))
            );
        }
        let reverse = capture(&views.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(cpu.render(&reverse, &assets).rejected, 0);
        assert_eq!(cpu.pixels(), pixels);
    }
    let packet = super::mdl_groups::capture_with(
        &assets,
        &[Refdef {
            model_time: Some(ThinkTime::Seconds(0.)),
            ..Default::default()
        }],
        |frame| {
            for sync_base in [0., 0.25] {
                assert!(model_choices::sprite(
                    frame,
                    SceneEntity {
                        origin: Vec3([8., 0., 0.]),
                        frame: 1,
                        sync_base,
                        ..Default::default()
                    },
                    model,
                    shaders.into_iter().map(Into::into)
                ));
            }
        },
    );
    let Command::View(view) = packet.commands()[1] else {
        panic!("sprite sync view")
    };
    assert_eq!(
        packet
            .polys(view.scene.polys)
            .iter()
            .map(|poly| poly.shader.source(&()).unwrap())
            .collect::<Vec<_>>(),
        [shaders[1], shaders[1], shaders[2], shaders[2]]
    );
}

#[test]
fn sp2_wraps_geometry_and_shader_at_the_same_boundary_and_choice_failure_is_atomic() {
    let (assets, spr, sp2, shaders) = sprites();
    let mut front = FrontEnd::load(Limits {
        entities: 2,
        model_choices: 3,
        model_joints: 0,
        ..Default::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(!model_choices::sprite(
        &mut frame,
        SceneEntity::default(),
        spr,
        shaders.into_iter().map(Into::into)
    ));
    assert!(!model_choices::sprite(
        &mut frame,
        SceneEntity::default(),
        sp2,
        []
    ));
    assert!(model_choices::sprite(
        &mut frame,
        SceneEntity {
            frame: 5,
            origin: Vec3([8., 0., 0.]),
            ..Default::default()
        },
        sp2,
        shaders[..2].iter().copied().map(Into::into)
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 2);
    assert_eq!(selected(&packet), [(1, shaders[1], Vec3([8., 3., 1.]))]);
    let SceneModel::Choice { root } = packet.entity(0).model else {
        panic!("SP2 choice span")
    };
    assert_eq!(root, 2);
    let invalid = super::mdl_groups::capture_with(&assets, &[Refdef::default()], |frame| {
        assert!(model_choices::sprite(
            frame,
            SceneEntity {
                frame: u32::MAX,
                origin: Vec3([8., 0., 0.]),
                ..Default::default()
            },
            spr,
            shaders.into_iter().map(Into::into)
        ));
    });
    assert_eq!(selected(&invalid), [(0, shaders[0], Vec3([8., 2., 2.]))]);
}

fn corner(assets: &Assets, model: ModelId, pose: SceneEntity, view: Refdef) -> Option<Vec3> {
    match qa_render::entities::prepare(
        &SceneEntity {
            model: model.into(),
            ..pose
        },
        view,
        assets,
    )
    .unwrap()
    {
        qa_render::entities::Prepared::Quad(vertices) => Some(vertices[0].position),
        qa_render::entities::Prepared::Empty => None,
        _ => panic!("shared sprite quad"),
    }
}
fn near(actual: Vec3, expected: Vec3) {
    for (a, b) in actual.0.into_iter().zip(expected.0) {
        assert!((a - b).abs() < 0.00001, "{actual:?} != {expected:?}");
    }
}

#[test]
fn native_sprite_orientations_poles_roll_and_beam_use_the_shared_quad() {
    let mut assets = Assets::load().unwrap();
    let models = [
        SpriteOrientation::ViewParallelUpright,
        SpriteOrientation::FacingUpright,
        SpriteOrientation::ViewParallel,
        SpriteOrientation::Oriented,
        SpriteOrientation::ViewParallelOriented,
    ]
    .map(|orientation| {
        let mut geometry = SpriteGeometry::single(EXTENTS[0]);
        geometry.orientation = orientation;
        assets
            .register_sprite(geometry, ShaderBindingId(0))
            .unwrap()
    });
    let pose = SceneEntity {
        origin: Vec3([16., 0., 0.]),
        sprite_roll: 90.,
        ..Default::default()
    };
    let pitched = Refdef {
        axes: [
            Vec3([0.8, 0., 0.6]),
            Vec3([0., 1., 0.]),
            Vec3([-0.6, 0., 0.8]),
        ],
        ..Default::default()
    };
    near(
        corner(&assets, models[0], pose, pitched).unwrap(),
        Vec3([16., 2., 2.]),
    );
    let diagonal = SceneEntity {
        origin: Vec3([16., 16., 0.]),
        ..pose
    };
    near(
        corner(&assets, models[1], diagonal, Refdef::default()).unwrap(),
        Vec3([16. - 2f32.sqrt(), 16. + 2f32.sqrt(), 2.]),
    );
    near(
        corner(&assets, models[2], pose, pitched).unwrap(),
        Vec3([14.8, 2., 1.6]),
    );
    let entity_axes = qa_core::math::angle_vectors(Vec3([30., 0., 0.])).axes();
    near(
        corner(
            &assets,
            models[3],
            SceneEntity {
                axes: entity_axes,
                ..pose
            },
            Refdef::default(),
        )
        .unwrap(),
        Vec3([17., 2., 3f32.sqrt()]),
    );
    near(
        corner(&assets, models[4], pose, Refdef::default()).unwrap(),
        Vec3([16., 2., -2.]),
    );
    for model in [models[0], models[1], models[2], models[4]] {
        let gl = Refdef {
            sprite_placement: SpritePlacement::OrientedOnly,
            ..Default::default()
        };
        near(
            corner(&assets, model, pose, gl).unwrap(),
            Vec3([16., 2., 2.]),
        );
    }
    let vertical = Refdef {
        axes: [Vec3([0., 0., 1.]), Vec3([0., 1., 0.]), Vec3([-1., 0., 0.])],
        ..Default::default()
    };
    assert!(corner(&assets, models[0], pose, vertical).is_none());
    assert!(
        corner(
            &assets,
            models[1],
            SceneEntity {
                origin: Vec3([0., 0., 16.]),
                ..pose
            },
            Refdef::default()
        )
        .is_none()
    );
    let mut beam = SpriteGeometry::single(EXTENTS[0]);
    beam.beam_length = 2.;
    let beam = assets.register_sprite(beam, ShaderBindingId(0)).unwrap();
    near(
        corner(&assets, beam, pose, Refdef::default()).unwrap(),
        Vec3([14., 2., 2.]),
    );
    near(
        corner(
            &assets,
            beam,
            pose,
            Refdef {
                sprite_placement: SpritePlacement::OrientedOnly,
                ..Default::default()
            },
        )
        .unwrap(),
        Vec3([16., 2., 2.]),
    );
    assert!(
        corner(
            &assets,
            models[2],
            SceneEntity {
                origin: Vec3([-16., 0., 0.]),
                ..pose
            },
            Refdef::default()
        )
        .is_none()
    );
}

#[test]
fn loaded_sprite_quad_adds_up_before_right_at_the_native_float_cancellation_boundary() {
    let mut assets = Assets::load().unwrap();
    let mut geometry = SpriteGeometry::single([2., 3., 33_554_432., 0.]);
    geometry.orientation = SpriteOrientation::Oriented;
    let model = assets
        .register_sprite(geometry, ShaderBindingId(0))
        .unwrap();
    let position = corner(
        &assets,
        model,
        SceneEntity {
            origin: Vec3([16_777_216., 0., 0.]),
            axes: [
                Vec3([0., 0., 1.]),
                Vec3([-0.8660254, -0.5, 0.]),
                Vec3([-0.5, 0.8660254, 0.]),
            ],
            ..Default::default()
        },
        Refdef {
            origin: Vec3([0., 0., -1.]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(position.0[0].to_bits(), 1.7320508f32.to_bits());
}
