use qa_core::{math, primitives::Vec3};
use qa_render::{
    lighting::{setup_entity, unlit_sample},
    scene::{Light, SceneEntity},
    stage::EntityLighting,
};

#[test]
fn native_unlit_sample_keeps_minimum_add_clamp_and_model_local_direction() {
    let sun = math::normalized(Vec3([0.45, 0.3, 0.9]));
    for identity in [0.0, 0.25, 0.5, 1.0] {
        for minimum_light in [false, true] {
            let entity = SceneEntity {
                minimum_light,
                axes: [Vec3([0., 1., 0.]), Vec3([-1., 0., 0.]), Vec3([0., 0., 1.])],
                ..Default::default()
            };
            let result = setup_entity(&entity, unlit_sample(identity, sun), identity, &[]);
            let expected =
                ((identity * 150.) + (identity * 32.)).min((identity * 255.) as i32 as f32);
            assert_eq!(result.ambient, [expected; 3]);
            assert_eq!(result.directed, [identity * 150.; 3]);
            if identity != 0. {
                assert!(
                    result.direction.0[0] > 0.
                        && result.direction.0[1] < 0.
                        && result.direction.0[2] > 0.
                );
            } else {
                assert_eq!(result.direction, Vec3::default());
            }
        }
    }
}

#[test]
fn dynamic_light_uses_lighting_origin_and_minimum_radius_without_double_normalization() {
    let entity = SceneEntity {
        origin: Vec3([100., 0., 0.]),
        lighting_origin: Some(Vec3([1., 0., 0.])),
        ..Default::default()
    };
    let sample = EntityLighting {
        ambient: [300., -10., 4.],
        directed: [0.; 3],
        direction: Vec3::default(),
    };
    let light = Light {
        origin: Vec3([5., 0., 0.]),
        radius: 8.,
        color: [1., 2., 3.],
        additive: true,
    };
    let result = setup_entity(&entity, sample, 1., &[light]);
    assert_eq!(result.ambient, [255., 22., 36.]);
    assert_eq!(result.directed, [4., 8., 12.]);
    assert_eq!(result.direction, Vec3([1., 0., 0.]));
}

#[test]
fn shared_view_lighting_keeps_explicit_values_and_does_not_light_generated_geometry() {
    use qa_render::{Assets, FrontEnd, Limits, MaterialId, Refdef, SceneModel, Vertex};
    let mut assets = Assets::load().unwrap();
    let model = assets
        .register_model(&[Vertex::default(); 3], &[0, 1, 2], MaterialId(0))
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let explicit = EntityLighting::default();
    assert!(frame.add_entity(SceneEntity {
        model: model.into(),
        lighting: Some(explicit),
        ..Default::default()
    }));
    assert!(frame.add_entity(SceneEntity {
        model: model.into(),
        ..Default::default()
    }));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Sprite {
            radius: 1.,
            rotation: 0.
        },
        material: Some(MaterialId(0)),
        ..Default::default()
    }));
    let mut samples = 0;
    frame.light_entities(1., |_| {
        samples += 1;
        unlit_sample(1., Vec3([0., 0., 1.]))
    });
    assert_eq!(samples, 1);
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    let packet = frame.finish();
    let qa_render::scene::Command::View(view) = packet.commands()[1] else {
        panic!("view")
    };
    let entities = packet.entities(view.scene.entities);
    assert_eq!(entities[0].lighting, Some(explicit));
    assert_eq!(entities[1].lighting.unwrap().ambient, [182.; 3]);
    assert_eq!(entities[2].lighting, None);
}

#[test]
fn no_world_diffuse_models_reach_cpu_pixels_through_shared_lighting() {
    use qa_render::{
        Assets, FrontEnd, Limits, Refdef, Vertex, Viewport,
        assets::{MaterialSettings, Stage},
        cpu::CpuBackend,
        shader::{Cull, RgbGen},
    };
    let mut assets = Assets::load().unwrap();
    let material = assets
        .register_material(
            "preview diffuse",
            &[Stage {
                rgb_gen: RgbGen::LightingDiffuse,
                ..Default::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let vertices =
        [[8., -2., -2.], [8., 2., -2.], [8., 2., 2.], [8., -2., 2.]].map(|position| Vertex {
            position: Vec3(position),
            normal: Vec3([0., 0., 1.]),
            ..Default::default()
        });
    let model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], material)
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: model.into(),
        ..Default::default()
    }));
    frame.light_entities(1., |_| unlit_sample(1., Vec3([0., 0., 1.])));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 32,
                height: 24,
                ..Default::default()
            },
            ..Default::default()
        },
        &[],
        &assets
    ));
    let mut cpu = CpuBackend::load(32, 24).unwrap();
    let stats = cpu.render(&frame.finish(), &assets);
    assert_eq!(stats.rejected, 0);
    assert!(cpu.inverse_depth().iter().any(|&depth| depth > 0.));
    assert!(
        cpu.pixels()
            .iter()
            .any(|&pixel| pixel != u32::from_le_bytes([0, 0, 0, 255]))
    );
}
