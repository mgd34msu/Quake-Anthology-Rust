#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    client_scene::{ClientEntities, ClientModelBindings, ClientSceneInput},
    model_assets::{ModelCatalog, ModelLimits, ModelMaterialPolicy, ModelProfile, ModelRequest},
    snapshots::{GunAnimation, Interpolation, PresentationClock, SnapshotBinding},
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

fn catalog(rules: RuleSetId) -> (support::Fixture, Assets, ModelCatalog, ModelProfile) {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", rules, Some(palette)).profile();
    let (assets, catalog) = ModelCatalog::load(
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
    (fixture, assets, catalog, profile)
}

fn configs() -> ConfigStrings {
    let layout = ConfigLayout::QUAKE2;
    ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap())).unwrap()
}

fn reference(client: ClientId) -> ModelRef {
    ModelRef::native(ResourceNamespace::Client(client), 50).unwrap()
}

#[test]
fn retained_static_aliases_share_the_dynamic_scene_path_without_source_edicts() {
    use qa_app::client_resources::ClientResources;

    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("progs/test.mdl", RuleSetId::Quake, Some(palette));
    let (assets, models) =
        ModelCatalog::load(&fixture.vfs, assets, [request], [], ModelLimits::default()).unwrap();
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
            .bind_all(&resources.state.configs, &models, request.profile(), None),
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
    assert!(resources.state.receive_static_entity(source_edict));
    let retained = resources.state.static_entities()[0];
    assert_eq!(retained.appearance.models[0], Some(reference(client)));
    assert_eq!(retained.appearance.native.colormap, 3);
    assert_eq!(retained.appearance.native.effects, 0);

    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let player = PlayerState::default();
    let input = ClientSceneInput {
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &resources.state.models,
        models: &models,
        static_entities: resources.state.static_entities(),
        player: &player,
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
    let counts = input.append(&mut frame, &assets, &Refdef::default(), true);
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
    let static_material = copied.material;
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
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal)
    ));
    let rows = [Entity { number: 3, words }];
    let native_player = [];
    let current = Frame {
        sequence: 2,
        time: ThinkTime::Seconds(1.0),
        command: 0,
        flags: 0,
        qw_peers: 0,
        areas: &[],
        player: &native_player,
        entities: &rows,
    };
    let changed = ClientSceneInput {
        current: Some(ReceivedFrame::QuakeWorld(current)),
        ..input
    };
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let counts = changed.append(&mut frame, &assets, &Refdef::default(), true);
    assert_eq!(counts.entities, 2);
    assert_eq!(counts.models, 1);
    let scene = frame.finish();
    assert_eq!(scene.entity(0).origin, retained.position);
    assert_eq!(scene.entity(0).model, static_model);
    assert_eq!(scene.entity(0).material, static_material);
    assert_eq!((scene.entity(0).frame, scene.entity(0).old_frame), (1, 1));
    assert_eq!(scene.entity(0).back_lerp, 0.0);
    assert_eq!(resources.state.static_entities(), &[retained]);
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
        assert!(resources.state.receive_static_entity(EntityRecord {
            position: Vec3([index as f32, 0.0, 0.0]),
            ..Default::default()
        }));
    }
    assert!(
        !resources
            .state
            .receive_static_entity(EntityRecord::default())
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

    let (_fixture, assets, models, profile) = catalog(RuleSetId::Quake2);
    let mut resources = replacement;
    let model_index = resources.state.layout.resources[0].get(50).unwrap();
    assert!(resources.apply(model_index, b"models/test/tris.md2", None, None));
    assert_eq!(
        resources
            .state
            .models
            .bind_all(&resources.state.configs, &models, profile, None),
        0
    );
    for position in [Vec3([1.0, 2.0, 3.0]), Vec3([4.0, 5.0, 6.0])] {
        assert!(resources.state.receive_static_entity(EntityRecord {
            position,
            appearance: EntityAppearance {
                source: RuleSetId::QuakeWorld,
                models: [Some(reference(client)), None, None, None],
                ..Default::default()
            },
            ..Default::default()
        }));
    }
    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let player_projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let player = PlayerState::default();
    let input = ClientSceneInput {
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &resources.state.models,
        models: &models,
        static_entities: resources.state.static_entities(),
        player: &player,
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
    let counts = input.append(&mut frame, &assets, &Refdef::default(), true);
    assert_eq!(counts.entities, 2);
    assert_eq!(counts.models, 1);
    assert_eq!(counts.rejected, 1);
    assert_eq!(resources.state.static_entities().len(), 2);
    assert_eq!(resources.state.rejected, 0);
    assert_eq!(frame.finish().entity(0).origin, Vec3([1.0, 2.0, 3.0]));
}

#[test]
fn connection_model_names_bind_once_and_reset_with_their_owner() {
    let (_fixture, _assets, catalog, profile) = catalog(RuleSetId::Quake2);
    let client = ClientId(7);
    let mut configs = configs();
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    configs.set(index, b"models/test/tris.md2").unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, &catalog, profile, None), 0);
    let policy = ModelMaterialPolicy::default();
    let selected = bindings
        .resolve(reference(client), 1, &catalog, policy)
        .unwrap();
    assert_ne!(selected.model.0, 50);
    assert_eq!(
        bindings.resolve(
            ModelRef::native(ResourceNamespace::Module(ModuleId(7)), 50).unwrap(),
            1,
            &catalog,
            policy
        ),
        None,
    );
    assert_eq!(
        bindings.resolve(reference(ClientId(8)), 1, &catalog, policy),
        None
    );
    configs.set(index, b"models/missing.md2").unwrap();
    assert!(!bindings.config_changed(index, &configs, &catalog));
    assert_eq!(
        bindings.resolve(reference(client), 1, &catalog, policy),
        None
    );
    configs.set(index, b"models/test/tris.md2").unwrap();
    assert!(bindings.config_changed(index, &configs, &catalog));
    bindings.reset();
    assert_eq!(
        bindings.resolve(reference(client), 1, &catalog, policy),
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
    let (_assets, catalog) = ModelCatalog::load(
        &fixture.vfs,
        assets,
        [native, foreign],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let client = ClientId(7);
    let mut configs = configs();
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    configs.set(index, native.path.as_bytes()).unwrap();
    // Wire resource layout does not select source or upload policy.
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    let policy = ModelMaterialPolicy::default();
    assert_eq!(
        bindings.bind_all(&configs, &catalog, native.profile(), None),
        0
    );
    let native_binding = bindings
        .resolve(reference(client), 1, &catalog, policy)
        .unwrap();
    assert_eq!(
        bindings.bind_all(&configs, &catalog, foreign.profile(), None),
        0
    );
    let foreign_binding = bindings
        .resolve(reference(client), 1, &catalog, policy)
        .unwrap();
    assert_eq!(native_binding.model, foreign_binding.model);
    assert_ne!(native_binding.material, foreign_binding.material);
    assert_eq!(
        foreign_binding.pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake).pose
    );
    configs.set(index, b"MODELS\\TEST\\TRIS.MD2").unwrap();
    assert!(bindings.config_changed(index, &configs, &catalog));
    assert_eq!(
        bindings.resolve(reference(client), 1, &catalog, policy),
        Some(foreign_binding)
    );
    let mut absent = foreign.profile();
    absent.source_rules = RuleSetId::Quake3;
    assert_eq!(bindings.bind_all(&configs, &catalog, absent, None), 1);
    assert_eq!(
        bindings.resolve(reference(client), 1, &catalog, policy),
        None
    );
    // A later text update retains the unregistered profile, rather than
    // silently accepting a native registration for the same filename.
    configs.set(index, native.path.as_bytes()).unwrap();
    assert!(!bindings.config_changed(index, &configs, &catalog));
    assert_eq!(
        bindings.resolve(reference(client), 1, &catalog, policy),
        None
    );
}

#[test]
fn owned_client_snapshots_and_view_weapon_fill_the_same_scene_packet() {
    let (_fixture, assets, catalog, profile) = catalog(RuleSetId::Quake2);
    let client = ClientId(3);
    let mut configs = configs();
    configs
        .set(
            ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
            b"models/test/tris.md2",
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, &catalog, profile, None), 0);
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
            &mut words,
            |reference| Some(reference.ordinal),
            |sound| Some(sound.ordinal)
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
            |model| Some(model.ordinal)
        ));
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(800 + i64::from(sequence) * 100),
            command: 0,
            flags: 0,
            qw_peers: 0,
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
        client,
        current: Some(ReceivedFrame::Quake2(ring.frame(2).unwrap())),
        previous: Some(ReceivedFrame::Quake2(ring.frame(1).unwrap())),
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &bindings,
        static_entities: &[],
        models: &catalog,
        player: &player,
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
    let counts = input.append(&mut frame, &assets, &refdef, true);
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
        packet.entity(1).material,
        Some(
            catalog
                .registration("models/test/tris.md2", profile)
                .and_then(|id| catalog.binding(id, 0, ModelMaterialPolicy::default()))
                .unwrap()
                .material
        )
    );
    assert_ne!(
        packet.entity(1).material,
        Some(
            catalog
                .registration("models/test/tris.md2", profile)
                .and_then(|id| catalog.binding(id, 1, ModelMaterialPolicy::default()))
                .unwrap()
                .material
        )
    );
    assert!(packet.entity(1).depth_hack);
    assert_eq!(
        packet.entity(1).axes,
        qa_core::math::angle_vectors(player.view_angles).axes()
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
        reset_gun.append(&mut frame, &assets, &refdef, true).models,
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
        let (_fixture, assets, catalog, profile) = catalog(presentation);
        let mut configs = configs();
        configs
            .set(
                ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
                b"models/test/tris.md2",
            )
            .unwrap();
        let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
        assert_eq!(bindings.bind_all(&configs, &catalog, profile, None), 0);
        let player = PlayerState::default();
        let input = ClientSceneInput {
            client,
            current: Some(ReceivedFrame::QuakeWorld(current)),
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
            player_projection: &player_projection,
            bindings: &bindings,
            static_entities: &[],
            models: &catalog,
            player: &player,
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
            let counts = input.append(&mut frame, &assets, &Refdef::default(), software);
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
                scene.entity(0).material,
                Some(
                    catalog
                        .registration("models/test/tris.md2", profile)
                        .and_then(|id| catalog.binding(id, 1, ModelMaterialPolicy::default()))
                        .unwrap()
                        .material
                )
            );
        }
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
        clock.advance(
            EventTime::from_milliseconds(queued),
            ThinkTime::Milliseconds(current),
            Some(ThinkTime::Milliseconds(old)),
            Some(100),
        );
        assert_eq!(clock.sample().time, ThinkTime::Milliseconds(expected_time));
        assert_eq!(clock.sample().fraction, expected_fraction);
    }
    let mut qw = PresentationClock::default();
    qw.advance(
        EventTime::from_milliseconds(500),
        ThinkTime::Milliseconds(0),
        Some(ThinkTime::Milliseconds(0)),
        None,
    );
    assert_eq!(qw.sample().fraction, 1.0); // no native server-time field
    let mut q3 = PresentationClock::default();
    q3.advance(
        EventTime::from_milliseconds(500),
        ThinkTime::Milliseconds(100),
        Some(ThinkTime::Milliseconds(50)),
        None,
    );
    q3.advance(
        EventTime::from_milliseconds(525),
        ThinkTime::Milliseconds(100),
        Some(ThinkTime::Milliseconds(50)),
        None,
    );
    assert_eq!(q3.sample().fraction, 0.5);
    let mut nq = PresentationClock::default();
    nq.advance(
        EventTime::from_milliseconds(500),
        ThinkTime::Seconds(0.1),
        Some(ThinkTime::Seconds(0.05)),
        None,
    );
    nq.advance(
        EventTime::from_milliseconds(525),
        ThinkTime::Seconds(0.1),
        Some(ThinkTime::Seconds(0.05)),
        None,
    );
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
    for rules in RuleSetId::ALL {
        let mut clock =
            PresentationClock::load(qa_gameplay::rules::presentation::clock_rules(rules));
        clock.advance(
            EventTime::default(),
            ThinkTime::Milliseconds(100),
            Some(ThinkTime::Milliseconds(0)),
            Some(100),
        );
        for (ms, q2_bits, interval_bits) in rows {
            clock.advance(
                EventTime::from_milliseconds(ms),
                ThinkTime::Milliseconds(100),
                Some(ThinkTime::Milliseconds(0)),
                Some(100),
            );
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
    let values = qa_ui::values::ValueLayout::load(&[]).unwrap();
    let mut output = EventRing::load(8, 1, 4, 64).unwrap();
    let consumer = output.bind(OutputTarget::Client(client)).unwrap();
    let mut snapshot = SnapshotBinding::load(
        client,
        Some(SeatId::FIRST),
        consumer,
        Protocol::Quake2Repro1038,
        0,
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
            |model| Some(model.ordinal)
        ));
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(sequence) * 25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &words,
            entities: &[],
        })
        .unwrap();
        snapshot.apply(
            Incoming::Snapshot(ReceivedFrame::Quake2Repro(ring.frame(sequence).unwrap())),
            &mut player,
            &qa_app::output_namespace::NativeNumbers::load(Protocol::Quake2Repro1038),
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
    let (_fixture, assets, catalog, profile) = catalog(RuleSetId::Quake2Rerelease);
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
    assert_eq!(bindings.bind_all(&configs, &catalog, profile, None), 0);
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
            |reference| Some(reference.ordinal)
        ));
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(sequence) * 25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &words,
            entities: &[],
        })
        .unwrap();
    }
    let player = PlayerState::default();
    let input = ClientSceneInput {
        client,
        current: Some(ReceivedFrame::Quake2Repro(ring.frame(4).unwrap())),
        previous: Some(ReceivedFrame::Quake2Repro(ring.frame(3).unwrap())),
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &bindings,
        static_entities: &[],
        models: &catalog,
        player: &player,
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
    assert_eq!(input.append(&mut frame, &assets, &refdef, true).models, 1);
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
        gun.material,
        Some(
            catalog
                .registration("models/test/tris.md2", profile)
                .and_then(|id| catalog.binding(id, 1, ModelMaterialPolicy::default()))
                .unwrap()
                .material
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
        repeated.append(&mut frame, &assets, &refdef, true).models,
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
        teleported.append(&mut frame, &assets, &refdef, true).models,
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
    assert_eq!(event.append(&mut frame, &assets, &refdef, true).models, 1);
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
    assert_eq!(gap.append(&mut frame, &assets, &refdef, true).models, 1);
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
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal)
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
            },
        )],
    )
    .unwrap();
    let sound = bank.find("sound/test.wav").unwrap();
    let mut mixer = qa_audio::Mixer::load(std::sync::Arc::new(bank), 1, 1);
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
    assert!(mixer.sound(qa_core::primitives::SoundEvent {
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
    }));
    qa_app::audio::respatialize(&mut mixer, &[Some(entities), None, None, None]);
    let mut pcm = [0; 2];
    assert!(mixer.paint(&mut pcm));
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
    assert!(mixer.paint(&mut pcm));
    assert_eq!(pcm, [1020, 0]);
    assert_eq!(mixer.counts.started, 1);
    // A different client with the same ordinal cannot supply this voice's pose.
    let foreign = ClientEntities {
        client: ClientId(9),
        ..later
    };
    qa_app::audio::respatialize(&mut mixer, &[Some(foreign), None, None, None]);
    assert!(mixer.paint(&mut pcm));
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
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal)
    ));
    let rows = [Entity { number: 0, words }];
    let frame = Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(100),
        command: 0,
        flags: 0,
        qw_peers: 0,
        areas: &[],
        player: &player,
        entities: &rows,
    };
    let entities = ClientEntities {
        client,
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
