use qa_core::primitives::Vec3;
use qa_render::scene::EntityVisibility;
use qa_render::{
    Assets, Command, FrontEnd, Limits, MaterialId, Refdef, SceneEntity, SceneModel, Vertex,
};

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
            material: Some(MaterialId(0)),
            shader_time: 1.25,
            shader_texcoord: [0.125, 0.5],
            ..Default::default()
        }));
        assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Sprite {
            radius: 2.,
            rotation: 0.
        },
        material: Some(MaterialId(0)),
        ..Default::default()
    }));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
            &assets
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
