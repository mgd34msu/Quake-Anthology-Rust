#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    client_scene::{ClientModelBindings, ClientSceneInput},
    model_assets::{ModelCatalog, ModelLimits, ModelMaterialPolicy, ModelRequest},
};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{
        ClientId, EntityAppearance, ModelRef, ModuleId, PlayerState, ResourceNamespace, RuleSetId,
        ThinkTime, Vec3, ViewWeapon,
    },
};
use qa_formats::archive::ArchiveReader;
use qa_network::{
    commands::packet::Protocol,
    projection::{EntityProjection, EntityRecord},
    snapshots::{Entity, Frame, Q2Ring, ReceivedFrame},
    states,
};
use qa_render::{Assets, FrontEnd, Limits, Refdef};

fn catalog() -> (support::Fixture, Assets, ModelCatalog) {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let catalog = ModelCatalog::load(
        &fixture.vfs,
        &mut ArchiveReader::default(),
        &mut assets,
        [ModelRequest::native(
            "models/test/tris.md2",
            RuleSetId::Quake2,
            Some(palette),
        )],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    (fixture, assets, catalog)
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
    let (_fixture, _assets, catalog) = catalog();
    let client = ClientId(7);
    let mut configs = configs();
    let index = ConfigLayout::QUAKE2.resources[0].get(50).unwrap();
    configs.set(index, b"models/test/tris.md2").unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, &catalog), 0);
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
fn owned_client_snapshots_and_view_weapon_fill_the_same_scene_packet() {
    let (_fixture, assets, catalog) = catalog();
    let client = ClientId(3);
    let mut configs = configs();
    configs
        .set(
            ConfigLayout::QUAKE2.resources[0].get(50).unwrap(),
            b"models/test/tris.md2",
        )
        .unwrap();
    let mut bindings = ClientModelBindings::load(client, ConfigLayout::QUAKE2);
    assert_eq!(bindings.bind_all(&configs, &catalog), 0);
    let projection = EntityProjection::load(Protocol::Quake2_34);
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
            appearance,
            ..Default::default()
        };
        let mut words = [0; states::Q2_ENTITY_WORDS];
        assert!(projection.reduce(&record, &mut words, |reference| Some(reference.ordinal)));
        let entities = [Entity { number: 12, words }];
        ring.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(800 + i64::from(sequence) * 100),
            command: 0,
            flags: 0,
            areas: &[0b0101],
            player: &[0; states::Q2_PLAYER_WORDS],
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
        ..Default::default()
    };
    let input = ClientSceneInput {
        client,
        current: Some(ReceivedFrame::Quake2(ring.frame(2).unwrap())),
        previous: Some(ReceivedFrame::Quake2(ring.frame(1).unwrap())),
        projection: &projection,
        bindings: &bindings,
        models: &catalog,
        player: &player,
        view_entity: Some(4),
    };
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    let refdef = Refdef {
        origin: Vec3([100.0, 200.0, 300.0]),
        time_ms: 950,
        ..Default::default()
    };
    let counts = input.append(&mut frame, &assets, &refdef);
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
    assert_eq!(packet.entity(1).origin, Vec3([102.0, 203.0, 304.0]));
    assert_eq!(packet.entity(1).frame, 1);
    assert!(packet.entity(1).depth_hack);
    assert_eq!(
        packet.entity(1).axes,
        qa_core::math::angle_vectors(player.view_angles + player.view_weapon.angles).axes()
    );
    let q3_projection = EntityProjection::load(Protocol::Quake3_68);
    let q3 = ClientSceneInput {
        current: Some(ReceivedFrame::Quake3(Frame {
            sequence: 3,
            time: ThinkTime::Milliseconds(1000),
            command: 0,
            flags: 0,
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
