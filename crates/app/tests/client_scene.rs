#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    client_scene::{ClientModelBindings, ClientSceneInput},
    model_assets::{ModelCatalog, ModelLimits, ModelMaterialPolicy, ModelProfile, ModelRequest},
    snapshots::{GunAnimation, Interpolation, LocalSnapshot, PresentationClock},
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
use qa_formats::archive::ArchiveReader;
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
    let catalog = ModelCatalog::load(
        &fixture.vfs,
        &mut ArchiveReader::default(),
        &mut assets,
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
    ModelRef {
        namespace: ResourceNamespace::Client(client),
        ordinal: 50,
    }
}

#[test]
fn connection_model_names_bind_once_and_reset_with_their_owner() {
    let (_fixture, _assets, catalog, profile) = catalog(RuleSetId::Quake2);
    let client = ClientId(7);
    let mut configs = configs();
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    configs.set(index, b"models/test/tris.md2").unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, &catalog, profile), 0);
    let policy = ModelMaterialPolicy::default();
    let selected = bindings
        .resolve(reference(client), 1, &catalog, policy)
        .unwrap();
    assert_ne!(selected.model.0, 50);
    assert_eq!(
        bindings.resolve(
            ModelRef {
                namespace: ResourceNamespace::Module(ModuleId(7)),
                ordinal: 50
            },
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
    let catalog = ModelCatalog::load(
        &fixture.vfs,
        &mut ArchiveReader::default(),
        &mut assets,
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
    assert_eq!(bindings.bind_all(&configs, &catalog, native.profile()), 0);
    let native_binding = bindings
        .resolve(reference(client), 1, &catalog, policy)
        .unwrap();
    assert_eq!(bindings.bind_all(&configs, &catalog, foreign.profile()), 0);
    let foreign_binding = bindings
        .resolve(reference(client), 1, &catalog, policy)
        .unwrap();
    assert_eq!(native_binding.model, foreign_binding.model);
    assert_ne!(native_binding.material, foreign_binding.material);
    assert_eq!(foreign_binding.rules, RuleSetId::Quake);
    configs.set(index, b"MODELS\\TEST\\TRIS.MD2").unwrap();
    assert!(bindings.config_changed(index, &configs, &catalog));
    assert_eq!(
        bindings.resolve(reference(client), 1, &catalog, policy),
        Some(foreign_binding)
    );
    let mut absent = foreign.profile();
    absent.source_rules = RuleSetId::Quake3;
    assert_eq!(bindings.bind_all(&configs, &catalog, absent), 1);
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
    assert_eq!(bindings.bind_all(&configs, &catalog, profile), 0);
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
        assert!(projection.reduce(&record, &mut words, |reference| Some(reference.ordinal)));
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
            areas: &[0b0101],
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
        player_projection: &player_projection,
        bindings: &bindings,
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
    assert_eq!(packet.entity(1).back_lerp, 0.0);
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
    let mut commands = Commands::load(Protocol::QuakeWorld28);
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
        assert_eq!(bindings.bind_all(&configs, &catalog, profile), 0);
        let player = PlayerState::default();
        let input = ClientSceneInput {
            client,
            current: Some(ReceivedFrame::QuakeWorld(current)),
            previous: None,
            projection: &projection,
            player_projection: &player_projection,
            bindings: &bindings,
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
    let mut clock = PresentationClock::default();
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
            Protocol::Quake2_34,
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
        Protocol::QuakeWorld28,
    );
    assert_eq!(qw.sample().fraction, 1.0); // no native server-time field
    let mut q3 = PresentationClock::default();
    q3.advance(
        EventTime::from_milliseconds(500),
        ThinkTime::Milliseconds(100),
        Some(ThinkTime::Milliseconds(50)),
        None,
        Protocol::Quake3_68,
    );
    q3.advance(
        EventTime::from_milliseconds(525),
        ThinkTime::Milliseconds(100),
        Some(ThinkTime::Milliseconds(50)),
        None,
        Protocol::Quake3_68,
    );
    assert_eq!(q3.sample().fraction, 0.5);
    let mut nq = PresentationClock::default();
    nq.advance(
        EventTime::from_milliseconds(500),
        ThinkTime::Seconds(0.1),
        Some(ThinkTime::Seconds(0.05)),
        None,
        Protocol::NetQuake15,
    );
    nq.advance(
        EventTime::from_milliseconds(525),
        ThinkTime::Seconds(0.1),
        Some(ThinkTime::Seconds(0.05)),
        None,
        Protocol::NetQuake15,
    );
    assert!((nq.sample().fraction - 0.5).abs() < 0.00001);
}

#[test]
fn every_accepted_gun_change_is_captured_before_one_client_phase() {
    use qa_network::snapshots::Q2ReproRing;
    let client = ClientId(3);
    let values = qa_ui::values::ValueLayout::load(&[]).unwrap();
    let mut output = EventRing::load(8, 1, 4, 64).unwrap();
    let consumer = output.bind(OutputTarget::Client(client)).unwrap();
    let mut snapshot = LocalSnapshot::load(
        client,
        SeatId::FIRST,
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
        );
        assert_eq!(player.view_weapon.frame, gunframe);
        // No CLIENT phase has advanced yet, even after two changes in a drain.
        assert_eq!(snapshot.interpolation(), Interpolation::default());
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
    assert_eq!(bindings.bind_all(&configs, &catalog, profile), 0);
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
        player_projection: &player_projection,
        bindings: &bindings,
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
