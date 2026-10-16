#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    client_scene::{ClientEntities, ClientModelBindings, ClientSceneInput},
    model_assets::{
        ModelAssets, ModelLimits, ModelMaterialPolicy, ModelProfile, ModelRequest, ModelView,
    },
    snapshots::{
        ClockOptions, GunAnimation, Interpolation, PresentationClock, SnapshotBinding,
        SnapshotClockSample,
    },
};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    events::{EventRing, OutputTarget},
    primitives::{
        ClientId, EntityAppearance, ModelRef, ModuleId, PlayerState, ResourceNamespace, RuleSetId,
        ThinkTime, Vec3, ViewWeapon,
    },
    sys_events::SeatId,
};
use qa_gameplay::rules::appearance::AliasRoll;
use qa_network::{
    commands::packet::Protocol,
    ingress::Incoming,
    projection::{EntityProjection, EntityRecord, PlayerContext, PlayerProjection},
    snapshots::{Entity, Frame, Q2Ring, ReceivedFrame},
    states,
};
use qa_render::{Assets, FrontEnd, Limits, Refdef};

fn catalog(rules: RuleSetId) -> (support::Fixture, ModelAssets, ModelProfile) {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", rules, Some(palette)).profile();
    let owner = ModelAssets::load(
        &fixture.vfs,
        assets,
        [ModelRequest::native(
            "models/test/tris.md2",
            rules,
            Some(palette),
        )],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    (fixture, owner, profile)
}

fn configs() -> ConfigStrings {
    let layout = ConfigLayout::QUAKE2;
    ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap())).unwrap()
}

fn reference(client: ClientId) -> ModelRef {
    ModelRef::native(ResourceNamespace::Client(client), 50).unwrap()
}

#[test]
fn native_linked_model_cells_keep_their_own_skin_alpha_and_draw_properties() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let (_fixture, owner, profile) = catalog(rules);
        let ModelView { assets, catalog } = owner.view();
        let protocol = if rules == RuleSetId::Quake2 {
            Protocol::Quake2_34
        } else {
            Protocol::Quake2Repro1038
        };
        let layout = ConfigLayout::for_rules(rules);
        let mut configs =
            ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
                .unwrap();
        // Classic 130 is a translucent reference to2. Rerelease130 is an
        // ordinary resource; neither source changes the linked skin reset.
        let linked = if rules == RuleSetId::Quake2 { 2 } else { 130 };
        for ordinal in [50, linked, 3, 4] {
            configs
                .set(
                    layout.resources[0].get(ordinal).unwrap(),
                    b"models/test/tris.md2",
                )
                .unwrap();
        }
        let client = ClientId(0);
        let namespace = ResourceNamespace::Client(client);
        let mut bindings = ClientModelBindings::load(client, layout);
        assert_eq!(bindings.bind_all(&configs, catalog, profile, None), 0);
        let mut record = EntityRecord {
            position: Vec3([32., 0., 0.]),
            appearance: EntityAppearance {
                source: rules,
                models: [50, 130, 3, 4].map(|ordinal| ModelRef::native(namespace, ordinal)),
                frame: 1,
                skin: 1,
                alpha: 0.6,
                ..Default::default()
            },
            ..Default::default()
        };
        record.appearance.native.render_flags = 8 | 16 | 32 | 64 | 0x8000;
        let projection = EntityProjection::load(protocol);
        let player_projection = PlayerProjection::load(protocol, &[]);
        let mut classic = [Entity {
            number: 12,
            words: [0; states::Q2_ENTITY_WORDS],
        }];
        let mut rerelease = [Entity {
            number: 12,
            words: [0; states::Q2_RERELEASE_ENTITY_WORDS],
        }];
        let words: &mut [u32] = if rules == RuleSetId::Quake2 {
            &mut classic[0].words
        } else {
            &mut rerelease[0].words
        };
        assert!(projection.reduce(
            &record,
            &Default::default(),
            words,
            |reference, _| Some(reference.ordinal),
            |_| None,
            |reference| Some(reference.ordinal())
        ));
        let mut accepted = EntityRecord::default();
        assert!(projection.apply(words, &mut accepted, namespace));
        let classic_player = [0; states::Q2_PLAYER_WORDS];
        let rerelease_player = [0; states::Q2_REPRO_PLAYER_WORDS];
        let current = if rules == RuleSetId::Quake2 {
            ReceivedFrame::Quake2(Frame {
                sequence: 1,
                time: ThinkTime::Milliseconds(100),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &classic_player,
                entities: &classic,
            })
        } else {
            ReceivedFrame::Quake2Repro(Frame {
                sequence: 1,
                time: ThinkTime::Milliseconds(100),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &rerelease_player,
                entities: &rerelease,
            })
        };
        let player = PlayerState::default();
        let retained = [bindings.capture_static(
            accepted,
            &configs,
            Some(catalog),
            qa_app::client_scene::StaticWorld::Selected(None),
        )];
        let registration = catalog
            .registration("models/test/tris.md2", profile)
            .unwrap();
        for static_record in [false, true] {
            let input = ClientSceneInput {
                synchronization: None,
                client,
                current: (!static_record).then_some(current),
                previous: None,
                projection: &projection,
                appearance: qa_gameplay::rules::appearance::policy(rules),
                player_projection: &player_projection,
                bindings: &bindings,
                models: catalog,
                static_entities: if static_record { &retained } else { &[] },
                player: &player,
                weapon_view_angles: Vec3::default(),
                alias_roll: AliasRoll::default(),
                interpolation: Default::default(),
                gun_animation: None,
                gun_previous: None,
                view_entity: None,
            };
            let mut frontend = FrontEnd::load(Limits::default()).unwrap();
            let mut frame = frontend.begin_frame([0; 4]).unwrap();
            let counts = input.append(&mut frame, assets, &Refdef::default(), true, None);
            assert_eq!(
                (counts.models, counts.missing_models, counts.rejected),
                (4, 0, 0),
                "{rules:?} static={static_record}"
            );
            let packet = frame.finish();
            for slot in 0..4 {
                let entity = packet.entity(slot);
                let alpha = if rules == RuleSetId::Quake2 {
                    [0., 0.32, 1., 1.][slot as usize]
                } else {
                    0.6
                };
                assert_eq!(entity.alpha, Some(alpha), "{rules:?} slot{slot}");
                assert_eq!(entity.depth_hack, slot == 0);
                assert_eq!(entity.infrared_visible, slot == 0);
                let expected = catalog
                    .bindings(
                        registration,
                        if slot == 0 { 1 } else { 0 },
                        ModelMaterialPolicy {
                            translucent: alpha != 1.,
                            fullbright: slot == 0,
                            ..Default::default()
                        },
                    )
                    .unwrap()
                    .next()
                    .unwrap();
                assert_eq!(
                    entity.shader.map(|shader| shader.source(&()).unwrap()),
                    Some(expected.shader),
                    "{rules:?} slot{slot}"
                );
                assert_eq!(entity.model, expected.model.into());
                assert_eq!(
                    entity.previous_origin.is_some(),
                    rules == RuleSetId::Quake2 && slot == 0
                );
            }
        }
    }
}

#[test]
fn q2_snapshot_sprite_frames_reach_the_shared_scene() {
    let fixture = support::Fixture::new(&[
        ("sprites/test.sp2", support::sp2()),
        ("sprites/first.pcx", support::pcx(11)),
        ("sprites/second.pcx", support::pcx(22)),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("sprites/test.sp2", RuleSetId::Quake2, Some(palette));
    let models_before = assets.models().len();
    let owner =
        ModelAssets::load(&fixture.vfs, assets, [request], [], ModelLimits::default()).unwrap();
    let ModelView {
        assets,
        catalog: models,
    } = owner.view();
    let client = ClientId(0);
    let mut configs = configs();
    configs
        .set(
            ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
            request.path.as_bytes(),
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(
        bindings.bind_all(&configs, models, request.profile(), None),
        0
    );
    let projection = EntityProjection::load(Protocol::Quake2_34);
    let player_projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let player = PlayerState::default();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut materials = vec![];
    for native_frame in [0, 1, 2] {
        let mut appearance = EntityAppearance {
            source: RuleSetId::Quake2,
            frame: native_frame,
            skin: 99,
            ..Default::default()
        };
        appearance.models[0] = Some(reference(client));
        let record = EntityRecord {
            position: Vec3([16., 0., 0.]),
            appearance,
            ..Default::default()
        };
        let mut words = [0; states::Q2_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &qa_network::projection::EntityContext::default(),
            &mut words,
            |model, _usage| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        let rows = [Entity { number: 12, words }];
        let native_player = [0; states::Q2_PLAYER_WORDS];
        let input = ClientSceneInput {
            synchronization: None,
            client,
            current: Some(ReceivedFrame::Quake2(Frame {
                sequence: 1,
                time: ThinkTime::Milliseconds(100),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &native_player,
                entities: &rows,
            })),
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(RuleSetId::Quake2),
            player_projection: &player_projection,
            bindings: &bindings,
            models: models,
            static_entities: &[],
            player: &player,
            weapon_view_angles: player.view_angles,
            alias_roll: AliasRoll::default(),
            interpolation: Interpolation::default(),
            gun_animation: None,
            gun_previous: None,
            view_entity: None,
        };
        let mut frame = frontend.begin_frame([0; 4]).unwrap();
        let refdef = Refdef::default();
        let counts = input.append(&mut frame, assets, &refdef, true, None);
        assert_eq!(
            (counts.models, counts.rejected, counts.missing_models),
            (1, 0, 0)
        );
        assert!(frame.render_scene(
            refdef,
            &[],
            assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let entity = packet.entity(0);
        let qa_render::SceneModel::Timed {
            model,
            selector: qa_render::scene::ModelChoiceSelector::Frame,
            ..
        } = entity.model
        else {
            panic!("deferred sprite model")
        };
        let qa_render::Command::View(view) = packet.commands()[1] else {
            panic!("sprite view")
        };
        let first = packet.draws(view.scene.draws)[0];
        assert_eq!(first.frames, Some([native_frame as u32 % 2; 2]));
        materials.push(first.shader.unwrap().source(&()).unwrap());
        assert_eq!(assets.models().len(), models_before + 1);
        assert_eq!(
            assets
                .model(model)
                .unwrap()
                .sprite_frame(native_frame as u32 % 2)
                .unwrap()
                .extents[0],
            if native_frame == 1 { 4. } else { -2. }
        );
        assert_eq!(packet.rejected, 0);
        assert!(frontend.recycle(packet).is_ok());
    }
    assert_ne!(materials[0], materials[1]);
    assert_eq!(materials[0], materials[2]);
}

#[test]
fn classic_frame_origin_motion_reaches_alias_pose_without_affecting_rerelease() {
    let (_fixture, owner, profile) = catalog(RuleSetId::Quake2);
    let ModelView {
        assets,
        catalog: models,
    } = owner.view();
    let client = ClientId(7);
    let mut configs = configs();
    configs
        .set(
            ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
            b"models/test/tris.md2",
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, models, profile, None), 0);
    let player = PlayerState::default();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let protocol = if source == RuleSetId::Quake2 {
            Protocol::Quake2_34
        } else {
            Protocol::Quake2Repro1038
        };
        let projection = EntityProjection::load(protocol);
        let player_projection = PlayerProjection::load(protocol, &[]);
        let mut record = EntityRecord {
            position: Vec3([16., 0., 0.]),
            appearance: EntityAppearance {
                source,
                frame: 1,
                old_origin: Vec3([26., 4., -8.]),
                models: [Some(reference(client)), None, None, None],
                ..Default::default()
            },
            ..Default::default()
        };
        record.appearance.native.render_flags = 64;
        let mut classic_words = [0; states::Q2_ENTITY_WORDS];
        let mut rerelease_words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &qa_network::projection::EntityContext::default(),
            if source == RuleSetId::Quake2 {
                &mut classic_words[..]
            } else {
                &mut rerelease_words[..]
            },
            |model, _usage| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        let classic_rows = [Entity {
            number: 12,
            words: classic_words,
        }];
        let rerelease_rows = [Entity {
            number: 12,
            words: rerelease_words,
        }];
        let classic_player = [0; states::Q2_PLAYER_WORDS];
        let rerelease_player = [0; states::Q2_REPRO_PLAYER_WORDS];
        let current = if source == RuleSetId::Quake2 {
            ReceivedFrame::Quake2(Frame {
                sequence: 1,
                time: ThinkTime::Milliseconds(100),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &classic_player,
                entities: &classic_rows,
            })
        } else {
            ReceivedFrame::Quake2Repro(Frame {
                sequence: 1,
                time: ThinkTime::Milliseconds(100),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &rerelease_player,
                entities: &rerelease_rows,
            })
        };
        let input = ClientSceneInput {
            synchronization: None,
            client,
            current: Some(current),
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(source),
            player_projection: &player_projection,
            bindings: &bindings,
            models: models,
            static_entities: &[],
            player: &player,
            weapon_view_angles: player.view_angles,
            alias_roll: AliasRoll::default(),
            interpolation: Interpolation {
                fraction: 0.25,
                ..Default::default()
            },
            gun_animation: None,
            gun_previous: None,
            view_entity: None,
        };
        let mut frame = frontend.begin_frame([0; 4]).unwrap();
        let counts = input.append(&mut frame, assets, &Refdef::default(), true, None);
        assert_eq!(
            (counts.models, counts.rejected, counts.unsupported),
            (1, 0, 0)
        );
        let packet = frame.finish();
        let entity = packet.entity(0);
        assert_eq!(entity.origin, record.position);
        assert_eq!((entity.frame, entity.old_frame), (1, 1));
        let qa_render::SceneModel::Mesh(model) = entity.model else {
            panic!("alias model")
        };
        let posed = assets
            .model(model)
            .unwrap()
            .posed_frames(entity)
            .unwrap()
            .vertex(0);
        if source == RuleSetId::Quake2 {
            assert_eq!(entity.previous_origin, Some(record.appearance.old_origin));
            assert_eq!(posed.position, Vec3([3.5, 11., 15.]));
        } else {
            assert!(entity.previous_origin.is_none());
            assert_eq!(posed.position, Vec3([-4., 8., 21.]));
        }
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn received_native_beams_keep_palette_width_endpoints_and_float_alpha_without_a_model() {
    use qa_app::client_scene::BeamInput;
    use qa_render::{Command, SceneModel, shader::StageBlend};
    let (_fixture, mut owner, profile) = catalog(RuleSetId::Quake2);
    let material =
        qa_render::entities::register_beam_shader(owner.assets_mut(), StageBlend::ALPHA).unwrap();
    let ModelView {
        assets,
        catalog: models,
    } = owner.view();
    let client = ClientId(7);
    let configs = configs();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, models, profile, None), 0);
    let palette = profile.source_palette.unwrap();
    let color = assets.palette(palette).unwrap().color(33).to_le_bytes();
    let player = PlayerState::default();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let protocol = if source == RuleSetId::Quake2 {
            Protocol::Quake2_34
        } else {
            Protocol::Quake2Repro1038
        };
        let projection = EntityProjection::load(protocol);
        let player_projection = PlayerProjection::load(protocol, &[]);
        let mut current = EntityRecord {
            position: Vec3([32., 0., 0.]),
            appearance: EntityAppearance {
                source,
                frame: 7,
                skin: 0x2c21160b,
                alpha: 0.,
                old_origin: Vec3([64., 0., 0.]),
                ..Default::default()
            },
            ..Default::default()
        };
        current.appearance.native.render_flags = 128;
        let old = EntityRecord {
            position: Vec3([16., 4., 0.]),
            appearance: EntityAppearance {
                old_origin: Vec3([48., 4., 0.]),
                ..current.appearance
            },
            ..current
        };
        let mut classic = [[0; states::Q2_ENTITY_WORDS]; 2];
        let mut rerelease = [[0; states::Q2_RERELEASE_ENTITY_WORDS]; 2];
        for (index, record) in [old, current].iter().enumerate() {
            assert!(projection.reduce(
                record,
                &qa_network::projection::EntityContext::default(),
                if source == RuleSetId::Quake2 {
                    &mut classic[index][..]
                } else {
                    &mut rerelease[index][..]
                },
                |model, _usage| Some(model.ordinal),
                |sound| Some(sound.ordinal),
                |reference| Some(reference.ordinal())
            ));
        }
        let classic_rows = classic.map(|words| [Entity { number: 12, words }]);
        let rerelease_rows = rerelease.map(|words| [Entity { number: 12, words }]);
        let classic_player = [0; states::Q2_PLAYER_WORDS];
        let rerelease_player = [0; states::Q2_REPRO_PLAYER_WORDS];
        let frame = |index: usize| {
            if source == RuleSetId::Quake2 {
                ReceivedFrame::Quake2(Frame {
                    sequence: index as u32 + 1,
                    time: ThinkTime::Milliseconds(100 + index as i64 * 100),
                    command: 0,
                    flags: 0,
                    qw_peers: 0,
                    qw_dead: 0,
                    areas: &[],
                    player: &classic_player,
                    entities: &classic_rows[index],
                })
            } else {
                ReceivedFrame::Quake2Repro(Frame {
                    sequence: index as u32 + 1,
                    time: ThinkTime::Milliseconds(100 + index as i64 * 100),
                    command: 0,
                    flags: 0,
                    qw_peers: 0,
                    qw_dead: 0,
                    areas: &[],
                    player: &rerelease_player,
                    entities: &rerelease_rows[index],
                })
            }
        };
        let input = ClientSceneInput {
            synchronization: None,
            client,
            current: Some(frame(1)),
            previous: Some(frame(0)),
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(source),
            player_projection: &player_projection,
            bindings: &bindings,
            models: models,
            static_entities: &[],
            player: &player,
            weapon_view_angles: player.view_angles,
            alias_roll: AliasRoll::default(),
            interpolation: Interpolation {
                fraction: 0.5,
                ..Default::default()
            },
            gun_animation: None,
            gun_previous: None,
            view_entity: None,
        };
        let mut random_calls = 0;
        let mut random = || {
            random_calls += 1;
            2
        };
        let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
        let refdef = Refdef::default();
        let counts = input.append(
            &mut frame,
            assets,
            &refdef,
            true,
            Some(BeamInput {
                shader: material,
                random: &mut random,
            }),
        );
        assert_eq!(
            (
                counts.beams,
                counts.models,
                counts.missing_models,
                counts.unsupported,
                counts.rejected
            ),
            (1, 0, 0, 0, 0)
        );
        assert!(frame.render_scene(
            refdef,
            &[],
            assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let entity = packet.entity(0);
        let (origin, end, radius) = if source == RuleSetId::Quake2 {
            (current.position, current.appearance.old_origin, 3.)
        } else {
            (Vec3([24., 2., 0.]), Vec3([56., 2., 0.]), 3.5)
        };
        assert_eq!(entity.origin, origin);
        assert_eq!(
            entity.model,
            SceneModel::Beam {
                end,
                radius,
                world_space: true
            }
        );
        assert_eq!(entity.color[..3], color[..3]);
        assert_eq!(entity.alpha, Some(0.3));
        assert_eq!(entity.palette_color, Some((palette, 33)));
        let view = packet
            .commands()
            .iter()
            .find_map(|cmd| {
                if let Command::View(view) = cmd {
                    Some(view)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(view.scene.polys.count, 12);
        let poly = packet.poly(view.scene.polys.first);
        let inputs = qa_render::stage::DrawInputs::for_view(&refdef).for_poly(poly, &packet);
        let prepared = qa_render::stage::StageEvaluator::load()
            .prepare(
                &assets
                    .material(assets.shader_binding(material).unwrap().material)
                    .unwrap()
                    .stages[0],
                assets
                    .material(assets.shader_binding(material).unwrap().material)
                    .unwrap()
                    .settings,
                inputs,
            )
            .unwrap();
        assert_eq!(prepared.float_alpha, Some(0.3));
        assert_eq!(random_calls, 1);
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn retained_static_aliases_share_the_dynamic_scene_path_without_source_edicts() {
    use qa_app::client_resources::ClientResources;

    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("progs/test.mdl", RuleSetId::Quake, Some(palette));
    let owner =
        ModelAssets::load(&fixture.vfs, assets, [request], [], ModelLimits::default()).unwrap();
    let ModelView {
        assets,
        catalog: models,
    } = owner.view();
    let client = ClientId(7);
    let mut resources = ClientResources::load(client, Protocol::QuakeWorld28, RuleSetId::Quake)
        .unwrap()
        .unwrap();
    let model_index = resources.state.layout.resources[0].get(50).unwrap();
    assert!(resources.apply(model_index, request.path.as_bytes(), None, None));
    assert_eq!(
        resources
            .state
            .models
            .bind_all(&resources.state.configs, models, request.profile(), None),
        0
    );
    let mut source_edict = EntityRecord {
        position: Vec3([12.0, 24.0, 48.0]),
        angles: Vec3([20.0, 45.0, 10.0]),
        appearance: EntityAppearance {
            source: RuleSetId::QuakeWorld,
            models: [
                ModelRef::native(ResourceNamespace::Module(ModuleId(3)), 50),
                None,
                None,
                None,
            ],
            frame: 1,
            native: qa_core::primitives::NativeAppearanceTail {
                colormap: 3,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        resources
            .state
            .receive_static_entity(source_edict, None, None)
    );
    let retained = resources.state.static_entities()[0].clone();
    assert_eq!(retained.appearance.models[0], Some(reference(client)));
    assert_eq!(retained.appearance.native.colormap, 3);
    assert_eq!(retained.appearance.native.effects, 0);

    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let player = PlayerState::default();
    let input = ClientSceneInput {
        synchronization: None,
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &resources.state.models,
        models: models,
        static_entities: resources.state.static_entities(),
        player: &player,
        weapon_view_angles: player.view_angles,
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation {
            fraction: 0.25,
            ..Default::default()
        },
        gun_animation: None,
        gun_previous: None,
        view_entity: Some(3),
    };
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let counts = input.append(&mut frame, assets, &Refdef::default(), true, None);
    assert_eq!(counts.entities, 1);
    assert_eq!(counts.models, 1);
    assert_eq!(
        (counts.rejected, counts.unsupported, counts.missing_models),
        (0, 0, 0)
    );
    let scene = frame.finish();
    let copied = scene.entity(0);
    assert_eq!(copied.origin, retained.position);
    assert_eq!(copied.frame, 1);
    assert_eq!(copied.old_frame, 1);
    assert_eq!(copied.back_lerp, 0.0);
    assert_eq!(
        copied.axes,
        qa_core::math::angle_vectors(Vec3([-20.0, 45.0, 10.0])).axes()
    );
    let static_model = copied.model;
    let static_material = copied.shader.map(|shader| shader.source(&()).unwrap());
    assert!(frontend.recycle(scene).is_ok());

    // Reusing the native source edict changes accepted dynamic rows, not the
    // independent signon copy. A static colormap is the global colormap, even
    // when its byte happens to equal a native player number.
    source_edict.position = Vec3([100.0, 200.0, 300.0]);
    source_edict.appearance.frame = 0;
    source_edict.appearance.native.colormap = 0;
    let mut words = [0; states::QW_ENTITY_WORDS];
    assert!(projection.reduce(
        &source_edict,
        &qa_network::projection::EntityContext::default(),
        &mut words,
        |model, _usage| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal())
    ));
    let rows = [Entity { number: 3, words }];
    let native_player = [];
    let current = Frame {
        sequence: 2,
        time: ThinkTime::Seconds(1.0),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &native_player,
        entities: &rows,
    };
    let changed = ClientSceneInput {
        current: Some(ReceivedFrame::QuakeWorld(current)),
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let counts = changed.append(&mut frame, assets, &Refdef::default(), true, None);
    assert_eq!(counts.entities, 2);
    assert_eq!(counts.models, 1);
    let scene = frame.finish();
    assert_eq!(scene.entity(0).origin, retained.position);
    assert_eq!(scene.entity(0).model, static_model);
    assert_eq!(
        scene
            .entity(0)
            .shader
            .map(|shader| shader.source(&()).unwrap()),
        static_material
    );
    assert_eq!((scene.entity(0).frame, scene.entity(0).old_frame), (1, 1));
    assert_eq!(scene.entity(0).back_lerp, 0.0);
    assert_eq!(resources.state.static_entities().len(), 1);
    assert_eq!(resources.state.static_entities()[0].record, retained.record);
}

#[test]
fn static_entity_admission_and_scene_capacity_reject_without_growing() {
    use qa_app::client_resources::ClientResources;

    let client = ClientId(9);
    let mut resources = ClientResources::load(client, Protocol::QuakeWorld28, RuleSetId::Quake)
        .unwrap()
        .unwrap();
    let capacity = resources.state.static_entity_capacity();
    assert_eq!(
        capacity,
        qa_core::primitives::NATIVE_WIRE_BYTES / qa_network::outputs::STATIC_ENTITY_BYTES
    );
    for index in 0..capacity {
        assert!(resources.state.receive_static_entity(
            EntityRecord {
                position: Vec3([index as f32, 0.0, 0.0]),
                ..Default::default()
            },
            None,
            None
        ));
    }
    assert!(
        !resources
            .state
            .receive_static_entity(EntityRecord::default(), None, None)
    );
    assert_eq!(resources.state.rejected, 1);
    assert_eq!(resources.state.static_entities().len(), capacity);
    assert_eq!(
        resources.state.static_entities()[capacity - 1].position.0[0],
        (capacity - 1) as f32
    );
    // Ordinal zero remains an accepted invisible static value.
    assert!(
        resources.state.static_entities()[0]
            .appearance
            .models
            .iter()
            .all(Option::is_none)
    );
    let replacement = ClientResources::load(client, Protocol::QuakeWorld28, RuleSetId::Quake)
        .unwrap()
        .unwrap();
    assert!(replacement.state.static_entities().is_empty());
    assert_eq!(replacement.state.rejected, 0);

    let (_fixture, owner, profile) = catalog(RuleSetId::Quake2);
    let ModelView {
        assets,
        catalog: models,
    } = owner.view();
    let mut resources = replacement;
    let model_index = resources.state.layout.resources[0].get(50).unwrap();
    assert!(resources.apply(model_index, b"models/test/tris.md2", None, None));
    assert_eq!(
        resources
            .state
            .models
            .bind_all(&resources.state.configs, models, profile, None),
        0
    );
    for position in [Vec3([1.0, 2.0, 3.0]), Vec3([4.0, 5.0, 6.0])] {
        assert!(resources.state.receive_static_entity(
            EntityRecord {
                position,
                appearance: EntityAppearance {
                    source: RuleSetId::QuakeWorld,
                    models: [Some(reference(client)), None, None, None],
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None
        ));
    }
    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let player = PlayerState::default();
    let input = ClientSceneInput {
        synchronization: None,
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &resources.state.models,
        models: models,
        static_entities: resources.state.static_entities(),
        player: &player,
        weapon_view_angles: player.view_angles,
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation::default(),
        gun_animation: None,
        gun_previous: None,
        view_entity: None,
    };
    let mut frontend = FrontEnd::load(Limits {
        entities: 1,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let counts = input.append(&mut frame, assets, &Refdef::default(), true, None);
    assert_eq!(counts.entities, 2);
    assert_eq!(counts.models, 1);
    assert_eq!(counts.rejected, 1);
    assert_eq!(resources.state.static_entities().len(), 2);
    assert_eq!(resources.state.rejected, 0);
    assert_eq!(frame.finish().entity(0).origin, Vec3([1.0, 2.0, 3.0]));
}

#[test]
fn connection_model_names_bind_once_and_reset_with_their_owner() {
    let (_fixture, owner, profile) = catalog(RuleSetId::Quake2);
    let ModelView {
        assets: _assets,
        catalog,
    } = owner.view();
    let client = ClientId(7);
    let mut configs = configs();
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    configs.set(index, b"models/test/tris.md2").unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, catalog, profile, None), 0);
    let policy = ModelMaterialPolicy::default();
    let selected = bindings
        .resolve(reference(client), 1, catalog, policy)
        .and_then(|mut surfaces| surfaces.next())
        .unwrap();
    assert_ne!(selected.model.0, 50);
    assert_eq!(
        bindings
            .resolve(
                ModelRef::native(ResourceNamespace::Module(ModuleId(7)), 50).unwrap(),
                1,
                catalog,
                policy
            )
            .and_then(|mut surfaces| surfaces.next()),
        None,
    );
    assert_eq!(
        bindings
            .resolve(reference(ClientId(8)), 1, catalog, policy)
            .and_then(|mut surfaces| surfaces.next()),
        None
    );
    configs.set(index, b"models/missing.md2").unwrap();
    assert!(!bindings.config_changed(index, &configs, catalog));
    assert_eq!(
        bindings
            .resolve(reference(client), 1, catalog, policy)
            .and_then(|mut surfaces| surfaces.next()),
        None
    );
    configs.set(index, b"models/test/tris.md2").unwrap();
    assert!(bindings.config_changed(index, &configs, catalog));
    bindings.reset();
    assert_eq!(
        bindings
            .resolve(reference(client), 1, catalog, policy)
            .and_then(|mut surfaces| surfaces.next()),
        None
    );
}

#[test]
fn configuration_cache_selects_an_explicit_profile_and_never_a_same_name_fallback() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let source = support::palette(&mut assets);
    let target = support::shifted_palette(&mut assets, 128);
    let native = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(source));
    let mut foreign = native;
    foreign.rules = RuleSetId::Quake;
    foreign.palette = Some(target);
    foreign.image_settings =
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake);
    let owner = ModelAssets::load(
        &fixture.vfs,
        assets,
        [native, foreign],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let catalog = owner.catalog();
    let client = ClientId(7);
    let mut configs = configs();
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    configs.set(index, native.path.as_bytes()).unwrap();
    // Wire resource layout does not select source or upload policy.
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    let policy = ModelMaterialPolicy::default();
    assert_eq!(
        bindings.bind_all(&configs, catalog, native.profile(), None),
        0
    );
    let native_binding = bindings
        .resolve(reference(client), 1, catalog, policy)
        .and_then(|mut surfaces| surfaces.next())
        .unwrap();
    assert_eq!(
        bindings.bind_all(&configs, catalog, foreign.profile(), None),
        0
    );
    let foreign_binding = bindings
        .resolve(reference(client), 1, catalog, policy)
        .and_then(|mut surfaces| surfaces.next())
        .unwrap();
    assert_eq!(native_binding.model, foreign_binding.model);
    assert_ne!(native_binding.shader, foreign_binding.shader);
    assert_eq!(
        foreign_binding.pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake).pose
    );
    configs.set(index, b"MODELS\\TEST\\TRIS.MD2").unwrap();
    assert!(bindings.config_changed(index, &configs, catalog));
    assert_eq!(
        bindings
            .resolve(reference(client), 1, catalog, policy)
            .and_then(|mut surfaces| surfaces.next()),
        Some(foreign_binding)
    );
    let mut absent = foreign.profile();
    absent.source_rules = RuleSetId::Quake3;
    assert_eq!(bindings.bind_all(&configs, catalog, absent, None), 1);
    assert_eq!(
        bindings
            .resolve(reference(client), 1, catalog, policy)
            .and_then(|mut surfaces| surfaces.next()),
        None
    );
    // A later text update retains the unregistered profile, rather than
    // silently accepting a native registration for the same filename.
    configs.set(index, native.path.as_bytes()).unwrap();
    assert!(!bindings.config_changed(index, &configs, catalog));
    assert_eq!(
        bindings
            .resolve(reference(client), 1, catalog, policy)
            .and_then(|mut surfaces| surfaces.next()),
        None
    );
}

#[test]
fn owned_client_snapshots_and_view_weapon_fill_the_same_scene_packet() {
    let (_fixture, owner, profile) = catalog(RuleSetId::Quake2);
    let ModelView { assets, catalog } = owner.view();
    let client = ClientId(3);
    let mut configs = configs();
    configs
        .set(
            ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
            b"models/test/tris.md2",
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, catalog, profile, None), 0);
    let projection = EntityProjection::load(Protocol::Quake2_34);
    let player_projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut ring = Q2Ring::load(8, 1024, 32, None).unwrap();
    for (sequence, position, frame) in [
        (1, Vec3([10.0, 20.0, 30.0]), 0),
        (2, Vec3([30.0, 20.0, 30.0]), 1),
    ] {
        let mut appearance = EntityAppearance {
            source: RuleSetId::Quake2,
            frame,
            skin: 1,
            ..Default::default()
        };
        appearance.models[0] = Some(reference(client));
        let record = EntityRecord {
            position,
            angles: if frame == 0 {
                Vec3([350.15625, 19.6875, 0.0])
            } else {
                Vec3([9.84375, 40.78125, 0.0])
            },
            appearance,
            ..Default::default()
        };
        let mut words = [0; states::Q2_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &qa_network::projection::EntityContext::default(),
            &mut words,
            |reference, _usage| Some(reference.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        let entities = [Entity { number: 12, words }];
        let mut native_player = PlayerState::default();
        native_player.view_weapon = ViewWeapon {
            model: Some(reference(client)),
            frame,
            offset: if frame == 0 {
                Vec3::default()
            } else {
                Vec3([2.0, 3.0, 4.0])
            },
            angles: if frame == 0 {
                Vec3([-1.0, -2.0, -3.0])
            } else {
                Vec3([1.0, 2.0, 3.0])
            },
            ..Default::default()
        };
        let mut player_words = [0; states::Q2_PLAYER_WORDS];
        assert!(player_projection.reduce(
            &native_player,
            &PlayerContext::default(),
            &mut player_words,
            |model, _usage| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(800 + i64::from(sequence) * 100),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[!0b0101],
            player: &player_words,
            entities: &entities,
        })
        .unwrap();
    }
    let mut player = PlayerState::with_capacity(1, 0, 0);
    player.view_angles = Vec3([5.0, 10.0, 0.0]);
    player.view_weapon = ViewWeapon {
        model: Some(reference(client)),
        frame: 1,
        offset: Vec3([2.0, 3.0, 4.0]),
        angles: Vec3([1.0, 2.0, 3.0]),
        skin: 1,
        ..Default::default()
    };
    let input = ClientSceneInput {
        synchronization: None,
        client,
        current: Some(ReceivedFrame::Quake2(ring.frame(2).unwrap())),
        previous: Some(ReceivedFrame::Quake2(ring.frame(1).unwrap())),
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &bindings,
        static_entities: &[],
        models: catalog,
        player: &player,
        weapon_view_angles: player.view_angles + Vec3([4.0, 6.0, 1.0]),
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation {
            time: ThinkTime::Milliseconds(950),
            fraction: 0.5,
            frame_ms: 100,
        },
        gun_animation: None,
        gun_previous: None,
        view_entity: Some(4),
    };
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let refdef = Refdef {
        origin: Vec3([100.0, 200.0, 300.0]),
        time_ms: 250_000,
        ..Default::default()
    };
    let counts = input.append(&mut frame, assets, &refdef, true, None);
    let mut mask = [0; 8];
    assert_eq!(input.hidden_areas(&mut mask), (&[!0b0101u8][..], false));
    assert_eq!(counts.entities, 1);
    assert_eq!(counts.models, 2);
    assert_eq!(counts.missing_models, 0);
    assert_eq!(counts.unsupported, 0);
    let packet = frame.finish();
    assert_eq!(packet.entity(0).origin, Vec3([20.0, 20.0, 30.0]));
    assert_eq!(
        input.entities().get(12).unwrap().origin,
        packet.entity(0).origin
    );
    assert_eq!(
        input.entities().get(12).unwrap().angles,
        Vec3([360.0, 30.234375, 0.0])
    );

    assert_eq!((packet.entity(0).frame, packet.entity(0).old_frame), (1, 0));
    assert_eq!(packet.entity(0).back_lerp, 0.5);
    assert_eq!(
        packet.entity(0).axes,
        qa_core::math::angle_vectors(Vec3([360.0, 30.234375, 0.0])).axes()
    );
    assert_eq!(packet.entity(1).origin, Vec3([101.0, 201.5, 302.0]));
    assert_eq!(packet.entity(1).frame, 1);
    assert_eq!(packet.entity(1).old_frame, 0);
    assert_eq!(packet.entity(1).back_lerp, 0.5);
    assert_eq!(
        packet
            .entity(1)
            .shader
            .map(|shader| shader.source(&()).unwrap()),
        Some(
            catalog
                .registration("models/test/tris.md2", profile)
                .and_then(|id| catalog
                    .bindings(id, 0, ModelMaterialPolicy::default())
                    .and_then(|mut surfaces| surfaces.next()))
                .unwrap()
                .shader
        )
    );
    assert_ne!(
        packet
            .entity(1)
            .shader
            .map(|shader| shader.source(&()).unwrap()),
        Some(
            catalog
                .registration("models/test/tris.md2", profile)
                .and_then(|id| catalog
                    .bindings(id, 1, ModelMaterialPolicy::default())
                    .and_then(|mut surfaces| surfaces.next()))
                .unwrap()
                .shader
        )
    );
    assert!(packet.entity(1).depth_hack);
    assert_eq!(
        packet.entity(1).axes,
        qa_core::math::angle_vectors(input.weapon_view_angles).axes()
    );
    assert!(frontend.recycle(packet).is_ok());
    let mut player_words = *ring.frame(2).unwrap().player;
    player_words[23] = 0;
    let reset_gun = ClientSceneInput {
        current: Some(ReceivedFrame::Quake2(Frame {
            player: &player_words,
            ..ring.frame(2).unwrap()
        })),
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert_eq!(
        reset_gun
            .append(&mut frame, assets, &refdef, true, None)
            .models,
        2
    );
    let packet = frame.finish();
    assert_eq!((packet.entity(1).frame, packet.entity(1).old_frame), (0, 0));
    // Original Q2 CL_AddViewWeapon resets oldframe only. The fractional
    // weight remains 1-lerpfrac even when both native frames are zero.
    assert_eq!(
        packet.entity(1).back_lerp,
        1.0 - input.interpolation.fraction
    );
    let q3_projection = EntityProjection::load(Protocol::Quake3_68);
    let q3 = ClientSceneInput {
        current: Some(ReceivedFrame::Quake3(Frame {
            sequence: 3,
            time: ThinkTime::Milliseconds(1000),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[0b0101, 0b1010],
            player: &[0; states::PLAYER_WORDS],
            entities: &[],
        })),
        projection: &q3_projection,
        appearance: qa_gameplay::rules::appearance::policy(q3_projection.rule_set()),
        ..input
    };
    assert_eq!(q3.hidden_areas(&mut mask), (&[0b0101, 0b1010][..], false));
    assert_eq!(q3.hidden_areas(&mut mask[..1]), (&[0b0101][..], true));
}

#[test]
fn received_qw_peer_velocity_and_raw_angles_use_registered_model_presentation() {
    use qa_core::loopback::Endpoint;
    use qa_network::{
        channel::Channel,
        commands::{QwCmd, connection::Commands, packet::ZERO_QW},
        message::{Encoding, Writer},
    };
    // QW serverinfo/model-list adoption remains separate. This fixture supplies
    // the owned connection's resource namespace explicitly, then exercises the
    // actual playerinfo parser and existing accepted entity/snapshot storage.
    let client = ClientId(0);
    let mut channel =
        Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel.configure_snapshots(Protocol::QuakeWorld28).unwrap();
    let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    let mut peer = [0; states::QW_PLAYER_WORDS];
    peer[0] = 60.0f32.to_bits();
    peer[6] = 100.0f32.to_bits();
    peer[8] = 50.0f32.to_bits();
    peer[9] = 1.0f32.to_bits();
    peer[12] = (1 << 1) | (1 << 3) | (1 << 5) | (1 << 6);
    assert!(
        states::write_qw_player(
            &mut writer,
            1,
            &peer,
            QwCmd {
                view_angles: Vec3([90.0, 0.0, 0.0]),
                ..ZERO_QW
            }
        )
        .unwrap()
    );
    writer.write_bits(47, 8).unwrap();
    let mut packet = [0; states::QW_ENTITY_WORDS];
    packet[0] = 50;
    packet[5] = 100.0f32.to_bits();
    packet[6] = 90.0f32.to_bits();
    assert!(
        states::write_qw_entity(
            &mut writer,
            40,
            &[0; states::QW_ENTITY_WORDS],
            Some(&packet),
            true
        )
        .unwrap()
    );
    writer.write_bits(0, 16).unwrap();
    let length = commands.stage(writer.bytes()).unwrap();
    assert_eq!(
        commands
            .decode_output(
                length,
                1,
                &mut channel,
                ResourceNamespace::Client(client),
                |_| {}
            )
            .unwrap(),
        Some(1)
    );
    let Some(ReceivedFrame::QuakeWorld(current)) = channel.current_snapshot() else {
        panic!("accepted QW frame")
    };
    assert!(current.qw_player(2));
    assert!(!current.qw_player(40));
    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let mut decoded = EntityRecord::default();
    assert!(projection.apply(
        &current.entities[0].words,
        &mut decoded,
        ResourceNamespace::Client(client)
    ));
    assert_eq!(decoded.velocity, Vec3([0.0, 100.0, 0.0]));
    assert_eq!(decoded.angles, Vec3([90.0, 0.0, 0.0]));
    for presentation in [RuleSetId::QuakeWorld, RuleSetId::Quake2] {
        let (_fixture, owner, profile) = catalog(presentation);
        let ModelView { assets, catalog } = owner.view();
        let mut configs = configs();
        configs
            .set(
                ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
                b"models/test/tris.md2",
            )
            .unwrap();
        let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
        assert_eq!(bindings.bind_all(&configs, catalog, profile, None), 0);
        let player = PlayerState::default();
        let input = ClientSceneInput {
            synchronization: None,
            client,
            current: Some(ReceivedFrame::QuakeWorld(current)),
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
            player_projection: &player_projection,
            bindings: &bindings,
            static_entities: &[],
            models: catalog,
            player: &player,
            weapon_view_angles: player.view_angles,
            alias_roll: AliasRoll {
                angle: 2.0,
                speed: 200.0,
            },
            interpolation: Interpolation::default(),
            gun_animation: None,
            gun_previous: None,
            view_entity: Some(1),
        };
        let common = input.entities().get(2).unwrap();
        assert_eq!(common.record.angles, Vec3([90.0, 0.0, 0.0]));
        assert_eq!(common.angles, Vec3([-30.0, 0.0, -4.0]));
        let packet_pose = input.entities().get(40).unwrap();
        assert_eq!(packet_pose.angles, packet_pose.record.angles);
        for software in [true, false] {
            let mut frontend = FrontEnd::load(Limits::default()).unwrap();
            let mut frame = frontend.begin_frame([0; 4]).unwrap();
            let counts = input.append(&mut frame, assets, &Refdef::default(), software, None);
            assert_eq!(counts.models, 2);
            let scene = frame.finish();
            let (peer_angles, packet_angles) = match (presentation, software) {
                (RuleSetId::QuakeWorld, _) => (Vec3([30.0, 0.0, -4.0]), Vec3([-90.0, 0.0, 0.0])),
                (_, true) => (Vec3([-30.0, 0.0, -4.0]), Vec3([90.0, 0.0, 0.0])),
                (_, false) => (Vec3([-30.0, 0.0, 4.0]), Vec3([90.0, 0.0, -0.0])),
            };
            assert_eq!(
                scene.entity(0).axes,
                qa_core::math::angle_vectors(peer_angles).axes()
            );
            assert_eq!(
                scene.entity(1).axes,
                qa_core::math::angle_vectors(packet_angles).axes()
            );
            assert_eq!(
                scene
                    .entity(0)
                    .shader
                    .map(|shader| shader.source(&()).unwrap()),
                Some(
                    catalog
                        .registration("models/test/tris.md2", profile)
                        .and_then(|id| catalog
                            .bindings(id, 1, ModelMaterialPolicy::default())
                            .and_then(|mut surfaces| surfaces.next()))
                        .unwrap()
                        .shader
                )
            );
        }
    }
}

fn clock_snapshot(time: ThinkTime) -> SnapshotClockSample {
    SnapshotClockSample {
        sequence: 0,
        time,
        flags: 0,
    }
}

#[test]
fn client_presentation_uses_queued_elapsed_time_in_the_native_snapshot_domain() {
    use qa_core::sys_events::EventTime;
    let mut clock = PresentationClock::load(qa_gameplay::rules::presentation::clock_rules(
        RuleSetId::Quake2,
    ));
    for (queued, current, old, expected_time, expected_fraction) in [
        (10_000, 100, 0, 0, 0.0),
        (10_050, 100, 0, 50, 0.5),
        (10_100, 200, 100, 100, 0.0),
        (10_150, 200, 100, 150, 0.5),
        (10_150, 200, 100, 150, 0.5),
        (20_000, 200, 100, 200, 1.0),
    ] {
        clock
            .advance(
                EventTime::from_milliseconds(queued),
                clock_snapshot(ThinkTime::Milliseconds(current)),
                Some(ThinkTime::Milliseconds(old)),
                Some(100),
                ClockOptions::default(),
            )
            .unwrap();
        assert_eq!(clock.sample().time, ThinkTime::Milliseconds(expected_time));
        assert_eq!(clock.sample().fraction, expected_fraction);
    }
    let mut qw = PresentationClock::default();
    qw.advance(
        EventTime::from_milliseconds(500),
        clock_snapshot(ThinkTime::Milliseconds(0)),
        Some(ThinkTime::Milliseconds(0)),
        None,
        ClockOptions::default(),
    )
    .unwrap();
    assert_eq!(qw.sample().fraction, 1.0); // no native server-time field
    let mut nq = PresentationClock::default();
    nq.advance(
        EventTime::from_milliseconds(500),
        clock_snapshot(ThinkTime::Seconds(0.1)),
        Some(ThinkTime::Seconds(0.05)),
        None,
        ClockOptions::default(),
    )
    .unwrap();
    nq.advance(
        EventTime::from_milliseconds(525),
        clock_snapshot(ThinkTime::Seconds(0.1)),
        Some(ThinkTime::Seconds(0.05)),
        None,
        ClockOptions::default(),
    )
    .unwrap();
    assert!((nq.sample().fraction - 0.5).abs() < 0.00001);
}

#[test]
fn cached_clock_policy_preserves_native_interpolation_rounding() {
    use qa_core::sys_events::EventTime;
    // Native remaining-time and interval-reciprocal forms intentionally differ.
    let rows = [
        (1, 0x3c23d740, 0x3c23d70a),
        (3, 0x3cf5c2a0, 0x3cf5c28f),
        (17, 0x3e2e147c, 0x3e2e147b),
        (29, 0x3e947ae2, 0x3e947ae1),
        (49, 0x3efae148, 0x3efae147),
        (51, 0x3f028f5c, 0x3f028f5c),
        (67, 0x3f2b851f, 0x3f2b851e),
        (99, 0x3f7d70a4, 0x3f7d70a3),
    ];
    // QW has no server-time interpolation domain. Its realtime/latency path
    // has independent native-bit checks in presentation_clock.rs.
    for rules in RuleSetId::ALL
        .into_iter()
        .filter(|rules| !matches!(rules, RuleSetId::Quake3 | RuleSetId::QuakeWorld))
    {
        let mut clock =
            PresentationClock::load(qa_gameplay::rules::presentation::clock_rules(rules));
        clock
            .advance(
                EventTime::default(),
                clock_snapshot(ThinkTime::Milliseconds(100)),
                Some(ThinkTime::Milliseconds(0)),
                Some(100),
                ClockOptions::default(),
            )
            .unwrap();
        for (ms, q2_bits, interval_bits) in rows {
            clock
                .advance(
                    EventTime::from_milliseconds(ms),
                    clock_snapshot(ThinkTime::Milliseconds(100)),
                    Some(ThinkTime::Milliseconds(0)),
                    Some(100),
                    ClockOptions::default(),
                )
                .unwrap();
            assert_eq!(clock.sample().time, ThinkTime::Milliseconds(ms as i64));
            assert_eq!(
                clock.sample().fraction.to_bits(),
                if rules == RuleSetId::Quake2 {
                    q2_bits
                } else {
                    interval_bits
                },
                "{rules:?}, {ms} ms",
            );
        }
    }
}

#[test]
fn every_accepted_gun_change_is_captured_before_one_client_phase() {
    use qa_network::snapshots::Q2ReproRing;
    let client = ClientId(3);
    let catalog = qa_app::catalog::GameplayCatalog::load([]).unwrap();
    let values = &catalog.hud.values;
    let mut output = EventRing::load(8, 1, 4, 64).unwrap();
    let consumer = output.bind(OutputTarget::Client(client)).unwrap();
    let mut snapshot = SnapshotBinding::load(
        client,
        Some(SeatId::FIRST),
        consumer,
        Protocol::Quake2Repro1038,
        RuleSetId::Quake2Rerelease,
        qa_app::snapshots::NativePlayerBinding::Supplied(0),
        &values,
    );
    let mut player = PlayerState::with_capacity(0, 0, values.capacity());
    let projection = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut ring = Q2ReproRing::load(4, 8192, 0, None).unwrap();
    for (sequence, gunframe, model) in [(3, 0, 50), (4, 1, 50), (5, 2, 50), (6, 2, 50), (7, 2, 51)]
    {
        let mut source = PlayerState::default();
        source.view_weapon = ViewWeapon {
            model: ModelRef::native(ResourceNamespace::Client(client), model),
            frame: gunframe,
            ..Default::default()
        };
        let mut words = [0; states::Q2_REPRO_PLAYER_WORDS];
        assert!(projection.reduce(
            &source,
            &PlayerContext::default(),
            &mut words,
            |model, _usage| Some(model.ordinal),
            |sound| Some(sound.ordinal)
        ));
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(sequence) * 25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &words,
            entities: &[],
        })
        .unwrap();
        snapshot.apply(
            Incoming::Snapshot(ReceivedFrame::Quake2Repro(ring.frame(sequence).unwrap())),
            &mut player,
            |_| None,
            &catalog.hud,
            &catalog.registry,
            qa_core::sys_events::EventTime::from_milliseconds(u64::from(sequence) * 25),
        );
        assert_eq!(player.view_weapon.frame, gunframe);
        // No CLIENT phase has advanced yet, even after two changes in a drain.
        assert_eq!(snapshot.interpolation(), Some(Interpolation::default()));
        let (changed, previous) = match sequence {
            3 => (75, None),
            4 => (100, Some(3)),
            5 | 6 => (125, Some(4)),
            7 => (175, Some(6)),
            _ => unreachable!(),
        };
        assert_eq!(
            snapshot.gun_animation(),
            Some(GunAnimation {
                changed_at: ThinkTime::Milliseconds(changed),
                previous,
            })
        );
    }
    assert_eq!(snapshot.applied, 5);
    assert_eq!(snapshot.rejected, 0);
}

#[test]
fn rerelease_weapon_skin_pose_and_rate_use_received_player_words_and_transition_time() {
    use qa_network::snapshots::Q2ReproRing;
    let (_fixture, owner, profile) = catalog(RuleSetId::Quake2Rerelease);
    let ModelView { assets, catalog } = owner.view();
    let client = ClientId(3);
    let layout = ConfigLayout::QUAKE2_RERELEASE;
    let mut configs =
        ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
            .unwrap();
    configs
        .set(
            layout.resources[0].get(50).unwrap(),
            b"models/test/tris.md2",
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, layout);
    assert_eq!(bindings.bind_all(&configs, catalog, profile, None), 0);
    let projection = EntityProjection::load(Protocol::Quake2Repro1038);
    let player_projection = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut ring = Q2ReproRing::load(4, 8192, 0, None).unwrap();
    for sequence in [3, 4, 5] {
        let current = sequence != 3;
        let mut source = PlayerState::default();
        source.view_weapon = ViewWeapon {
            model: Some(reference(client)),
            frame: i32::from(current),
            offset: if current {
                Vec3([4.0, 6.0, 8.0])
            } else {
                Vec3::default()
            },
            angles: Vec3([0.0, 0.0, if current { 2.0 } else { -2.0 }]),
            skin: i32::from(current),
            rate: 10,
        };
        let mut words = [0; states::Q2_REPRO_PLAYER_WORDS];
        assert!(player_projection.reduce(
            &source,
            &PlayerContext::default(),
            &mut words,
            |reference, _usage| Some(reference.ordinal),
            |sound| Some(sound.ordinal),
        ));
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(sequence) * 25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &words,
            entities: &[],
        })
        .unwrap();
    }
    let player = PlayerState::default();
    let input = ClientSceneInput {
        synchronization: None,
        client,
        current: Some(ReceivedFrame::Quake2Repro(ring.frame(4).unwrap())),
        previous: Some(ReceivedFrame::Quake2Repro(ring.frame(3).unwrap())),
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &bindings,
        static_entities: &[],
        models: catalog,
        player: &player,
        weapon_view_angles: player.view_angles,
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation {
            time: ThinkTime::Milliseconds(87),
            fraction: 0.5,
            frame_ms: 25,
        },
        gun_animation: Some(GunAnimation {
            changed_at: ThinkTime::Milliseconds(100),
            previous: Some(3),
        }),
        gun_previous: Some(ReceivedFrame::Quake2Repro(ring.frame(3).unwrap())),
        view_entity: Some(4),
    };
    let refdef = Refdef {
        origin: Vec3([100.0, 200.0, 300.0]),
        time_ms: 70_000,
        ..Default::default()
    };
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert_eq!(
        input.append(&mut frame, assets, &refdef, true, None).models,
        1
    );
    let scene = frame.finish();
    let gun = scene.entity(0);
    assert_eq!(gun.origin, Vec3([102.0, 203.0, 304.0]));
    assert_eq!(
        gun.axes,
        qa_core::math::angle_vectors(Vec3::default()).axes()
    );
    assert_eq!((gun.frame, gun.old_frame), (1, 0));
    assert_eq!(gun.back_lerp, 1.0 - 12.0 / 100.0);
    assert_eq!(
        gun.shader.map(|shader| shader.source(&()).unwrap()),
        Some(
            catalog
                .registration("models/test/tris.md2", profile)
                .and_then(|id| catalog
                    .bindings(id, 1, ModelMaterialPolicy::default())
                    .and_then(|mut surfaces| surfaces.next()))
                .unwrap()
                .shader
        )
    );
    assert!(frontend.recycle(scene).is_ok());
    // The following snapshot repeats gunframe1. Animation still reads frame0
    // from the sole ring's original transition, not repeated immediate frame1.
    let repeated = ClientSceneInput {
        current: Some(ReceivedFrame::Quake2Repro(ring.frame(5).unwrap())),
        previous: Some(ReceivedFrame::Quake2Repro(ring.frame(4).unwrap())),
        interpolation: Interpolation {
            time: ThinkTime::Milliseconds(112),
            fraction: 0.5,
            frame_ms: 25,
        },
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert_eq!(
        repeated
            .append(&mut frame, assets, &refdef, true, None)
            .models,
        1
    );
    let scene = frame.finish();
    assert_eq!((scene.entity(0).frame, scene.entity(0).old_frame), (1, 0));
    assert_eq!(scene.entity(0).back_lerp, 1.0 - 37.0 / 100.0);
    let sample = Interpolation {
        time: ThinkTime::Milliseconds(100),
        fraction: 1.0,
        frame_ms: 25,
    };
    assert_eq!(sample.gun_back_lerp(ThinkTime::Milliseconds(100), 0), 0.75);
    assert_eq!(sample.gun_back_lerp(ThinkTime::Milliseconds(100), 20), 0.5);

    assert!(frontend.recycle(scene).is_ok());
    let mut words = ring.frame(4).unwrap().player.to_owned();
    words[35] = 1 << 4; // native RDF_TELEPORT_BIT changed
    let teleported = ClientSceneInput {
        current: Some(ReceivedFrame::Quake2Repro(Frame {
            player: &words,
            ..ring.frame(4).unwrap()
        })),
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert_eq!(
        teleported
            .append(&mut frame, assets, &refdef, true, None)
            .models,
        1
    );
    let scene = frame.finish();
    assert_eq!(scene.entity(0).origin, Vec3([104.0, 206.0, 308.0]));
    assert_eq!((scene.entity(0).frame, scene.entity(0).old_frame), (1, 1));
    assert_eq!(scene.entity(0).back_lerp, 0.0);
    assert!(frontend.recycle(scene).is_ok());

    let mut entity_words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
    entity_words[18] = 6; // native EV_PLAYER_TELEPORT
    let entities = [Entity {
        number: 4,
        words: entity_words,
    }];
    let event = ClientSceneInput {
        current: Some(ReceivedFrame::Quake2Repro(Frame {
            entities: &entities,
            ..ring.frame(4).unwrap()
        })),
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert_eq!(
        event.append(&mut frame, assets, &refdef, true, None).models,
        1
    );
    let scene = frame.finish();
    assert_eq!(scene.entity(0).origin, Vec3([104.0, 206.0, 308.0]));
    assert_eq!(scene.entity(0).back_lerp, 0.0);
    assert!(frontend.recycle(scene).is_ok());

    let gap = ClientSceneInput {
        previous: Some(ReceivedFrame::Quake2Repro(Frame {
            sequence: 2,
            ..ring.frame(3).unwrap()
        })),
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert_eq!(
        gap.append(&mut frame, assets, &refdef, true, None).models,
        1
    );
    let scene = frame.finish();
    assert_eq!(scene.entity(0).origin, Vec3([104.0, 206.0, 308.0]));
    assert_eq!(scene.entity(0).back_lerp, 0.0);
}

#[test]
fn client_pose_query_keeps_native_discontinuities_without_assets_or_server_state() {
    let client = ClientId(7);
    let projection = EntityProjection::load(Protocol::Quake2_34);
    let player = [0; states::Q2_PLAYER_WORDS];
    let old = EntityRecord {
        position: Vec3([100.0, 0.0, 0.0]),
        appearance: EntityAppearance {
            source: RuleSetId::Quake2,
            models: [Some(reference(client)), None, None, None],
            ..Default::default()
        },
        ..Default::default()
    };
    let current = EntityRecord {
        position: Vec3([-100.0, 0.0, 0.0]),
        ..old
    };
    let encode = |record: EntityRecord| {
        let mut words = [0; states::Q2_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &qa_network::projection::EntityContext::default(),
            &mut words,
            |model, _usage| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        [Entity { number: 12, words }]
    };
    let old_rows = encode(old);
    let current_rows = encode(current);
    let old_frame = Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(100),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &player,
        entities: &old_rows,
    };
    let current_frame = Frame {
        sequence: 2,
        time: ThinkTime::Milliseconds(200),
        entities: &current_rows,
        ..old_frame
    };
    let entities = ClientEntities {
        client,
        player: None,
        current: Some(ReceivedFrame::Quake2(current_frame)),
        previous: Some(ReceivedFrame::Quake2(old_frame)),
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation {
            time: ThinkTime::Milliseconds(125),
            fraction: 0.25,
            frame_ms: 100,
        },
    };
    assert_eq!(entities.get(12).unwrap().origin, Vec3([50.0, 0.0, 0.0]));
    assert!(entities.get(11).is_none());
    assert!(entities.get(13).is_none());
    assert!(entities.get(0).is_none());
    assert_eq!(entities.sound_origin(0, None), Some(Vec3::default()));
    assert_eq!(
        entities.get(12).unwrap().record.appearance.models[0],
        Some(reference(client))
    );
    let gap = ClientEntities {
        previous: Some(ReceivedFrame::Quake2(Frame {
            sequence: 0,
            ..old_frame
        })),
        ..entities
    };
    assert_eq!(gap.get(12).unwrap().origin, current.position);
    for change in 0..4 {
        let mut changed = current;
        match change {
            0 => changed.appearance.native.render_flags = 64, // RF_FRAMELERP
            1 => changed.appearance.native.event = 6,         // EV_PLAYER_TELEPORT
            2 => changed.appearance.models[0].as_mut().unwrap().ordinal += 1,
            3 => changed.position.0[0] = -500.0, // native large-displacement reset
            _ => unreachable!(),
        }
        let rows = encode(changed);
        let view = ClientEntities {
            current: Some(ReceivedFrame::Quake2(Frame {
                entities: &rows,
                ..current_frame
            })),
            ..entities
        };
        assert_eq!(
            view.get(12).unwrap().origin,
            changed.position,
            "case {change}"
        );
    }

    let rate = std::num::NonZeroU32::new(44100).unwrap();
    let bank = qa_audio::Bank::load(
        rate,
        vec![(
            "sound/test.wav".into(),
            qa_core::primitives::Pcm {
                rate,
                channels: qa_core::primitives::PcmChannels::Mono,
                samples: vec![1024; 3],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let sound = bank.find("sound/test.wav").unwrap();
    let mut mixer = qa_audio::Mixer::load(bank.rate, 1, 1);
    mixer.listen(
        &[Some(qa_audio::Listener {
            negate_dot: false,
            client,
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
        })],
        1.0,
    );
    assert!(mixer.sound(
        &bank,
        qa_core::primitives::SoundEvent {
            sound,
            spatial: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
            entity: Some(qa_core::primitives::SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: 12,
            }),
            position: old.position,
            volume: 1.0,
            attenuation: 1.0,
            client: Some(client),
            ..Default::default()
        }
    ));
    qa_app::audio::respatialize(&mut mixer, &[Some(entities), None, None, None]);
    let mut pcm = [0; 2];
    assert!(mixer.paint(&bank, &mut pcm));
    assert_eq!(pcm, [0, 1020]);
    let later = ClientEntities {
        interpolation: Interpolation {
            fraction: 0.75,
            ..entities.interpolation
        },
        ..entities
    };
    assert_eq!(later.get(12).unwrap().origin, Vec3([-50.0, 0.0, 0.0]));
    qa_app::audio::respatialize(&mut mixer, &[Some(later), None, None, None]);
    assert!(mixer.paint(&bank, &mut pcm));
    assert_eq!(pcm, [1020, 0]);
    assert_eq!(mixer.counts.started, 1);
    // A different client with the same ordinal cannot supply this voice's pose.
    let foreign = ClientEntities {
        client: ClientId(9),
        ..later
    };
    qa_app::audio::respatialize(&mut mixer, &[Some(foreign), None, None, None]);
    assert!(mixer.paint(&bank, &mut pcm));
    assert_eq!(pcm, [1020, 0]);
    assert_eq!(mixer.counts.missing_origins, 1);
}

#[test]
fn native_world_sound_zero_is_static_but_q3_entity_zero_has_a_client_pose() {
    let client = ClientId(4);
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let player = [0; states::PLAYER_WORDS];
    let record = EntityRecord {
        position: Vec3([24.0, 48.0, 96.0]),
        appearance: EntityAppearance {
            source: RuleSetId::Quake3,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut words = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &qa_network::projection::EntityContext::default(),
        &mut words,
        |model, _usage| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal())
    ));
    let rows = [Entity { number: 0, words }];
    let frame = Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(100),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &player,
        entities: &rows,
    };
    let entities = ClientEntities {
        client,
        player: None,
        current: Some(ReceivedFrame::Quake3(frame)),
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation::default(),
    };
    assert_eq!(entities.sound_origin(0, None), Some(record.position));
    let absent = ClientEntities {
        current: Some(ReceivedFrame::Quake3(Frame {
            entities: &[],
            ..frame
        })),
        ..entities
    };
    assert_eq!(absent.sound_origin(0, None), None);
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let projection = EntityProjection::load(protocol);
        let world = ClientEntities {
            current: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
            alias_roll: AliasRoll::default(),
            ..entities
        };
        assert_eq!(world.sound_origin(0, None), Some(Vec3::default()));
        assert_eq!(world.sound_origin(1, None), None);
    }
}

#[test]
fn synthesized_q3_player_sound_origin_does_not_interpolate_a_packet_pose() {
    use qa_core::primitives::{NativePlayerTail, SoundRef};
    use qa_network::snapshots::Q3Frame;
    let client = ClientId(4);
    let namespace = ResourceNamespace::Client(client);
    let mut player = PlayerState::default();
    player.body.position = Vec3([64., 32., 16.]);
    player.native = Some(NativePlayerTail {
        client_number: Some(0),
        ..NativePlayerTail::new(RuleSetId::Quake3, namespace)
    });
    player.loop_sound = SoundRef::native(namespace, 1);
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mut previous_words = [0; states::ENTITY_WORDS];
    let record = EntityRecord {
        position: Vec3([4., 8., 12.]),
        ..Default::default()
    };
    assert!(projection.reduce(
        &record,
        &qa_network::projection::EntityContext::default(),
        &mut previous_words,
        |_, _usage| None,
        |_| None,
        |reference| Some(reference.ordinal())
    ));
    let previous_entities = [Entity {
        number: 0,
        words: previous_words,
    }];
    let words = [0; states::PLAYER_WORDS];
    let frame = Q3Frame {
        sequence: 2,
        time: ThinkTime::Milliseconds(200),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &words,
        entities: &[],
    };
    let previous = Q3Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(100),
        entities: &previous_entities,
        ..frame
    };
    let entities = ClientEntities {
        client,
        player: Some(&player),
        current: Some(ReceivedFrame::Quake3(frame)),
        previous: Some(ReceivedFrame::Quake3(previous)),
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(RuleSetId::Quake3),
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation {
            fraction: 0.25,
            ..Default::default()
        },
    };
    assert_eq!(entities.get(0).unwrap().origin, player.body.position);
    assert_eq!(entities.sound_origin(0, None), Some(player.body.position));
}

#[test]
fn static_model_captures_survive_alias_rewrite_before_and_after_initial_registration() {
    use qa_app::{
        Runtime, client_resources::ClientResources, host::FrameHost, model_assets::ModelAssets,
    };
    use qa_console::{commands::Console, views::Context};
    use qa_session::timing::TickRate;
    let presented = |host: &FrameHost| {
        let ModelView { assets, catalog } = host.runtime.model_assets.as_ref().unwrap().view();
        let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
        let client = ClientId(0);
        let projection = EntityProjection::load(Protocol::QuakeWorld28);
        let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
        let player = PlayerState::default();
        let input = ClientSceneInput {
            synchronization: None,
            client,
            current: None,
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
            player_projection: &player_projection,
            bindings: &state.models,
            models: catalog,
            static_entities: state.static_entities(),
            player: &player,
            weapon_view_angles: player.view_angles,
            alias_roll: AliasRoll::default(),
            interpolation: Default::default(),
            gun_animation: None,
            gun_previous: None,
            view_entity: None,
        };
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = frontend.begin_frame([0; 4]).unwrap();
        let counts = input.append(&mut frame, assets, &Refdef::default(), false, None);
        let packet = frame.finish();
        let model = (counts.models != 0).then(|| packet.entity(0).model);
        (counts, model)
    };
    for (bound_at_receipt, pending_at_receipt) in [(false, false), (true, false), (false, true)] {
        let fixture = support::Fixture::new(&[
            ("models/a.md2", support::md2()),
            ("models/b.md2", support::md2()),
            ("models/test/skin0.pcx", support::pcx(11)),
            ("models/test/skin1.pcx", support::pcx(22)),
        ]);
        let mut assets = Assets::load().unwrap();
        let palette = support::palette(&mut assets);
        let first = ModelRequest::native("models/a.md2", RuleSetId::Quake2, Some(palette));
        let second = ModelRequest::native("models/b.md2", RuleSetId::Quake2, Some(palette));
        let profile = first.profile();
        let mut builder = ModelAssets::new(assets, [profile], [], ModelLimits::default()).unwrap();
        let mut backend = pending_at_receipt.then(|| {
            let backend = qa_render::cpu::CpuBackend::load_with_limits(
                8,
                8,
                builder.assets(),
                qa_render::cpu::CpuLimits {
                    cache_bytes: 4096,
                    ..Default::default()
                },
            )
            .unwrap();
            builder
                .publish_admission(backend.admitted_assets().unwrap())
                .unwrap();
            builder.end_initial_registration();
            backend
        });
        builder.register_request(&fixture.vfs, first).unwrap();
        builder.register_request(&fixture.vfs, second).unwrap();
        let client = ClientId(0);
        let mut resources =
            ClientResources::load(client, Protocol::QuakeWorld28, RuleSetId::Quake2)
                .unwrap()
                .unwrap();
        let row = resources.state.layout.resources[0].get(50).unwrap();
        assert!(resources.apply(row, first.path.as_bytes(), None, None));
        let record = EntityRecord {
            position: Vec3([8., 4., 2.]),
            appearance: EntityAppearance {
                source: RuleSetId::Quake2,
                models: [Some(reference(client)), None, None, None],
                ..Default::default()
            },
            ..Default::default()
        };
        if bound_at_receipt {
            builder.end_initial_registration();
            let catalog = builder.catalog();
            assert_eq!(
                resources
                    .state
                    .models
                    .bind_all(&resources.state.configs, catalog, profile, None),
                0
            );
            assert!(resources.state.receive_static_entity(record, None, None));
            assert!(resources.apply(row, second.path.as_bytes(), Some(catalog), None));
        } else {
            // Receipt can precede this CLIENT's profile binding, including a
            // logical file whose backend admission is still pending.
            assert!(
                resources
                    .state
                    .receive_static_entity(record, Some(builder.catalog()), None)
            );
            assert!(resources.apply(row, second.path.as_bytes(), None, None));
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.client_resources[0] = Some(resources);
        runtime.model_assets = Some(builder);
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        if !bound_at_receipt && !pending_at_receipt {
            // Another CLIENT's prepared prefix can publish before this seat
            // selects a profile. That must retain its numeric static identity.
            let backend = qa_render::cpu::CpuBackend::load_with_limits(
                8,
                8,
                host.runtime.model_assets.as_ref().unwrap().assets(),
                qa_render::cpu::CpuLimits {
                    cache_bytes: 4096,
                    ..Default::default()
                },
            )
            .unwrap();
            host.publish_model_admission(backend.admitted_assets().unwrap())
                .unwrap();
        }
        host.bind_client_model_assets([(client, profile, None)]);
        if let Some(backend) = backend.as_mut() {
            let owner = host.runtime.model_assets.as_ref().unwrap();
            let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
            assert!(
                state
                    .models
                    .resolve(reference(client), 0, owner.catalog(), Default::default())
                    .is_none()
            );
            assert_eq!(state.static_entities().len(), 1);
            let (counts, submitted) = presented(&host);
            assert_eq!(
                (counts.entities, counts.models, counts.missing_models),
                (1, 0, 1)
            );
            assert!(submitted.is_none());
            backend.prepare_assets(owner.assets(), None).unwrap();
            // Backend preparation alone does not publish CLIENT readiness.
            assert_eq!(presented(&host).0.models, 0);
            host.publish_model_admission(backend.admitted_assets().unwrap())
                .unwrap();
        }
        let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
        let first_registration = catalog.registration(first.path, profile).unwrap();
        let first_model = catalog
            .bindings(first_registration, 0, ModelMaterialPolicy::default())
            .unwrap()
            .next()
            .unwrap()
            .model;
        let second_registration = catalog.registration(second.path, profile).unwrap();
        let second_model = catalog
            .bindings(second_registration, 0, ModelMaterialPolicy::default())
            .unwrap()
            .next()
            .unwrap()
            .model;
        assert_ne!(first_model, second_model);
        let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
        assert_eq!(
            state
                .models
                .resolve(
                    reference(client),
                    0,
                    catalog,
                    ModelMaterialPolicy::default()
                )
                .unwrap()
                .next()
                .unwrap()
                .model,
            second_model
        );
        let (counts, submitted) = presented(&host);
        assert_eq!(
            (counts.entities, counts.models, counts.missing_models),
            (1, 1, 0),
            "bound={bound_at_receipt} pending={pending_at_receipt}"
        );
        assert_eq!(
            submitted,
            Some(first_model.into()),
            "bound={bound_at_receipt} pending={pending_at_receipt}"
        );
    }
}

#[test]
fn native_inline_config_names_use_common_prefix_parser_and_integer_limits() {
    let mut assets = Assets::load().unwrap();
    let world = support::world(&mut assets, 0.0);
    let profile = ModelRequest::native("", RuleSetId::Quake2, None).profile();
    let owner = ModelAssets::new(assets, [profile], [world], Default::default()).unwrap();
    let mut rows = configs();
    let mut bindings = ClientModelBindings::load(ClientId(0), ConfigLayout::QUAKE2);
    bindings.bind_all(
        &rows,
        owner.catalog(),
        profile,
        Some((world, qa_render::WorldPresentationId(0))),
    );
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    for text in [b"*1suffix".as_slice(), b"*+1rest", b"*", b"*nonnumeric"] {
        rows.set(index, text).unwrap();
        assert!(
            bindings.config_changed(index, &rows, owner.catalog()),
            "{text:?}"
        );
    }
    for text in [b"*2".as_slice(), b"*4294967297", b"*-4294967295"] {
        rows.set(index, text).unwrap();
        assert!(
            !bindings.config_changed(index, &rows, owner.catalog()),
            "{text:?}"
        );
    }
}

#[test]
fn netquake_oriented_spr_preserves_pitch_and_native_parent_fallback_in_shared_views() {
    let mut sprite = support::spr();
    sprite[8..12].copy_from_slice(&3i32.to_le_bytes());
    let fixture = support::Fixture::new(&[("progs/oriented.spr", sprite)]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("progs/oriented.spr", RuleSetId::Quake, Some(palette));
    let owner = ModelAssets::load(&fixture.vfs, assets, [request], [], Default::default()).unwrap();
    let client = ClientId(0);
    let layout = ConfigLayout::for_rules(RuleSetId::Quake);
    let mut configs =
        ConfigStrings::load((0..layout.count).map(|row| layout.capacity(row).unwrap())).unwrap();
    configs
        .set(
            layout.resources[0].get(50).unwrap(),
            request.path.as_bytes(),
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, layout);
    assert_eq!(
        bindings.bind_all(&configs, owner.catalog(), request.profile(), None),
        0
    );
    let projection = EntityProjection::load(Protocol::NetQuake15);
    let player_projection = PlayerProjection::load(Protocol::NetQuake15, &[]);
    let player = PlayerState::default();
    let angles = Vec3([30., 45., 10.]);
    let registration = owner
        .catalog()
        .registration(request.path, request.profile())
        .unwrap();
    let choice = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .choices()
        .next()
        .unwrap();
    for (native_frame, physical) in [(1, 2), (99, 0), (255, 0)] {
        let record = EntityRecord {
            position: Vec3([32., 0., 0.]),
            angles,
            appearance: EntityAppearance {
                source: RuleSetId::Quake,
                frame: native_frame,
                models: [Some(reference(client)), None, None, None],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut words = [0; states::NQ_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &Default::default(),
            &mut words,
            |reference, _| Some(reference.ordinal),
            |_| None,
            |_| None
        ));
        let mut wire = [0; 128];
        let mut writer =
            qa_network::message::Writer::new(&mut wire, qa_network::message::Encoding::Bytes);
        assert!(
            states::write_nq_entity(
                &mut writer,
                12,
                &[0; states::NQ_ENTITY_WORDS],
                &words,
                false
            )
            .unwrap()
        );
        let mut reader =
            qa_network::message::Reader::new(writer.bytes(), qa_network::message::Encoding::Bytes);
        let update = states::read_nq_entity(&mut reader, &[0; states::NQ_ENTITY_WORDS]).unwrap();
        let words = update.words.unwrap();
        let mut accepted = EntityRecord::default();
        assert!(projection.apply(&words, &mut accepted, ResourceNamespace::Client(client)));
        let expected_angles = accepted.angles;
        let rows = [Entity {
            number: u32::from(update.number),
            words,
        }];
        let input = ClientSceneInput {
            synchronization: None,
            client,
            current: Some(ReceivedFrame::NetQuake(Frame {
                sequence: 1,
                time: ThinkTime::Seconds(0.125),
                command: 0,
                flags: 0,
                qw_peers: 0,
                qw_dead: 0,
                areas: &[],
                player: &[0; states::NQ_PLAYER_WORDS],
                entities: &rows,
            })),
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(RuleSetId::Quake),
            player_projection: &player_projection,
            bindings: &bindings,
            models: owner.catalog(),
            static_entities: &[],
            player: &player,
            weapon_view_angles: Vec3::default(),
            alias_roll: Default::default(),
            interpolation: Default::default(),
            gun_animation: None,
            gun_previous: None,
            view_entity: None,
        };
        for software in [true, false] {
            let mut frontend = FrontEnd::load(Default::default()).unwrap();
            let mut frame = frontend.begin_frame([0; 4]).unwrap();
            let view = Refdef {
                model_time: Some(ThinkTime::Seconds(0.125)),
                frame_timing: qa_render::scene::FrameTiming::UniformFirst,
                ..Default::default()
            };
            let counts = input.append(&mut frame, owner.assets(), &view, software, None);
            assert_eq!(
                (counts.models, counts.rejected, counts.missing_models),
                (1, 0, 0),
                "frame={native_frame} software={software}"
            );
            assert!(frame.render_scene(
                view,
                &[],
                owner.assets(),
                qa_render::shader::reference::ShaderView::direct()
            ));
            let packet = frame.finish();
            let captured = packet.entity(0);
            assert_eq!(
                captured.axes,
                qa_core::math::angle_vectors(expected_angles).axes()
            );
            assert_eq!(captured.sprite_roll, expected_angles.0[2]);
            let qa_render::Command::View(view) = packet.commands()[1] else {
                panic!("oriented SPR view")
            };
            assert_eq!(view.scene.polys.count, 2);
            let expected = choice
                .resolve_shader(
                    choice.shaders().nth(physical).unwrap(),
                    qa_render::shader::reference::ShaderView::direct(),
                    None,
                )
                .unwrap()
                .source(&())
                .unwrap();
            for draw in packet.draws(view.scene.draws) {
                assert_eq!(draw.frames, Some([physical as u32; 2]));
                assert_eq!(draw.shader.unwrap().source(&()), Some(expected));
            }
            assert_eq!(packet.rejected, 0);
        }
    }
}
