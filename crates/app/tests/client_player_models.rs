#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    Runtime,
    client_resources::ClientResources,
    client_scene::{ClientSceneInput, StaticEntity},
    host::FrameHost,
    model_assets::{ModelAssets, ModelMaterialPolicy, ModelProfile, ModelRequest},
};
use qa_compat::services::{ModelLoader, ModelLookup};
use qa_console::{commands::Console, views::Context};
use qa_core::{configstrings::ConfigLayout, primitives::*};
use qa_gameplay::rules::appearance::AliasRoll;
use qa_network::{
    commands::packet::Protocol,
    projection::{EntityProjection, PlayerProjection},
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_render::{Assets, FrontEnd, Limits, Refdef, SceneEntity};
use qa_session::timing::TickRate;

fn loading(
    rules: RuleSetId,
    player: &[u8],
    partial_base: bool,
) -> (support::Fixture, FrameHost, ModelProfile) {
    let mut files = vec![
        ("players/male/tris.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx(11)),
        ("models/test/skin1.pcx", support::pcx(22)),
        ("models/items/shell/tris.md2", support::md2()),
        ("models/ordinary.md2", support::md2()),
    ];
    if !partial_base {
        files.extend([
            ("players/male/weapon.md2", support::md2()),
            ("players/male/w1.md2", support::md2()),
            ("players/male/grunt.pcx", support::pcx(33)),
            ("players/male/grunt_i.pcx", support::pcx(34)),
            ("players/female/tris.md2", support::md2()),
            ("players/female/weapon.md2", support::md2()),
            ("players/female/w1.md2", support::md2()),
            ("players/female/red.pcx", support::pcx(55)),
            ("players/female/red_i.pcx", support::pcx(56)),
            ("players/female/athena.pcx", support::pcx(66)),
        ]);
    }
    let mut fixture = support::Fixture::new(&files);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", rules, Some(palette)).profile();
    let builder = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    let protocol = if rules == RuleSetId::Quake2 {
        Protocol::Quake2_34
    } else {
        Protocol::Quake2Repro1038
    };
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    runtime.model_assets = Some(builder);
    let mut resources = ClientResources::load(ClientId(0), protocol, rules)
        .unwrap()
        .unwrap();
    let layout = ConfigLayout::for_rules(rules);
    assert!(resources.apply(
        layout.resources[0].get(1).unwrap(),
        b"players/male/tris.md2",
        None,
        None
    ));
    assert!(resources.apply(layout.resources[0].get(2).unwrap(), b"#w1.md2", None, None));
    assert!(resources.apply(layout.players.get(0).unwrap(), player, None, None));
    if !partial_base {
        // Preload a second native player row. Frozen policy changes may select
        // these numeric assets without registering a new file during a frame.
        assert!(resources.apply(
            layout.players.get(1).unwrap(),
            b"Other\\female/athena",
            None,
            None
        ));
    }
    runtime.client_resources[0] = Some(resources);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let context = Context {
        source: rules,
        ..Default::default()
    };
    for (name, value) in [("cl_noskins", "0"), ("cl_vwep", "1")] {
        let view = host.console.cvars.bind(name, context).unwrap();
        host.console.cvars.force_write(view, value).unwrap();
    }
    host.register_client_model_assets();
    (fixture, host, profile)
}
fn bind(host: &mut FrameHost, profile: ModelProfile) {
    host.bind_client_model_assets([(ClientId(0), profile, None)]);
}
fn player_record(rules: RuleSetId, skin: i32) -> EntityRecord {
    EntityRecord {
        position: Vec3([32., 0., 0.]),
        appearance: EntityAppearance {
            source: rules,
            skin,
            models: [
                ModelRef::native(ResourceNamespace::Client(ClientId(0)), 255),
                ModelRef::native(ResourceNamespace::Client(ClientId(0)), 255),
                None,
                None,
            ],
            ..Default::default()
        },
        ..Default::default()
    }
}
fn draw(
    host: &FrameHost,
    rules: RuleSetId,
    record: EntityRecord,
    statics: Option<&[StaticEntity]>,
) -> (qa_app::client_scene::SceneCounts, Vec<SceneEntity>) {
    let assets = host.runtime.model_assets.as_ref().unwrap().assets();
    let protocol = if rules == RuleSetId::Quake2 {
        Protocol::Quake2_34
    } else {
        Protocol::Quake2Repro1038
    };
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
        |model, _| Some(model.ordinal),
        |_| None,
        |reference| Some(reference.ordinal())
    ));
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
    let resources = host.runtime.client_resources[0].as_ref().unwrap();
    let player = PlayerState::default();
    let input = ClientSceneInput {
        client: ClientId(0),
        current: statics.is_none().then_some(current),
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(rules),
        player_projection: &player_projection,
        bindings: &resources.state.models,
        models: host.runtime.model_assets.as_ref().unwrap().catalog(),
        static_entities: statics.unwrap_or(&[]),
        player: &player,
        weapon_view_angles: Vec3::default(),
        alias_roll: AliasRoll::default(),
        interpolation: Default::default(),
        gun_animation: None,
        gun_previous: None,
        view_entity: None,
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let counts = input.append(&mut frame, assets, &Refdef::default(), true, None);
    let packet = frame.finish();
    let entities = (0..counts.models)
        .map(|index| *packet.entity(index))
        .collect();
    (counts, entities)
}
fn model(host: &FrameHost, profile: ModelProfile, path: &[u8]) -> qa_render::SceneModel {
    let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
    let handle = catalog.file_handle(path).unwrap();
    let registration = catalog.renderer_registration(handle, profile).unwrap();
    catalog
        .bindings(registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap()
        .model
        .into()
}

#[test]
fn configured_player_models_and_weapons_survive_initial_registration_and_static_capture() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let (_fixture, mut host, profile) = loading(rules, b"Name\\female/red", false);
        let skin = if rules == RuleSetId::Quake2 {
            1 << 8
        } else {
            (0x31 << 24) | (0x42 << 16) | (1 << 8)
        };
        let record = player_record(rules, skin);
        {
            let runtime = &mut host.runtime;
            let resources = runtime.client_resources[0].as_mut().unwrap();
            assert!(
                resources.state.receive_static_entity(
                    record,
                    runtime
                        .model_assets
                        .as_ref()
                        .map(|owner| owner.catalog() as &dyn ModelLookup),
                    None
                )
            );
        }
        bind(&mut host, profile);
        let (counts, entities) = draw(&host, rules, record, None);
        assert_eq!((counts.models, counts.missing_models), (2, 0));
        assert_eq!(
            entities[0].model,
            model(&host, profile, b"players/female/tris.md2")
        );
        assert_eq!(
            entities[1].model,
            model(&host, profile, b"players/female/w1.md2")
        );
        let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
        let skin_handle = catalog
            .skin_handle(b"players/female/red.pcx", rules)
            .unwrap();
        let registration = catalog
            .renderer_registration(
                catalog.file_handle(b"players/female/tris.md2").unwrap(),
                profile,
            )
            .unwrap();
        let expected = catalog
            .bindings(
                registration,
                0,
                ModelMaterialPolicy {
                    custom_skin: skin_handle,
                    ..Default::default()
                },
            )
            .unwrap()
            .next()
            .unwrap()
            .material;
        assert_eq!(entities[0].material, Some(expected));
        let row = ConfigLayout::for_rules(rules).players.get(0).unwrap();
        assert!(host.runtime.client_resources[0].as_mut().unwrap().apply(
            row,
            b"Other\\male/grunt",
            Some(catalog),
            None
        ));
        let (_, changed) = draw(&host, rules, record, None);
        assert_eq!(
            changed[0].model,
            model(&host, profile, b"players/male/tris.md2")
        );
        let retained = host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .static_entities();
        let (counts, static_entities) = draw(&host, rules, record, Some(retained));
        assert_eq!((counts.models, counts.missing_models), (2, 0));
        assert_eq!(static_entities[0].model, entities[0].model);
        assert_eq!(static_entities[0].material, entities[0].material);
        assert_eq!(static_entities[1].model, entities[1].model);
    }
}

#[test]
fn source_player_fallbacks_and_cached_skin_weapon_cvars_remain_independent() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let (_fixture, mut host, profile) = loading(rules, b"\xff\\female/red", false);
        bind(&mut host, profile);
        let record = player_record(rules, 1 << 8);
        let (_, before) = draw(&host, rules, record, None);
        assert_eq!(
            before[0].model,
            model(&host, profile, b"players/female/tris.md2")
        );
        let context = Context {
            source: rules,
            ..Default::default()
        };
        let view = host.console.cvars.bind("cl_vwep", context).unwrap();
        host.console.cvars.force_write(view, "0").unwrap();
        host.register_client_model_assets();
        let (_, no_vwep) = draw(&host, rules, record, None);
        assert_eq!(
            no_vwep[1].model,
            model(&host, profile, b"players/female/weapon.md2")
        );
        let view = host.console.cvars.bind("cl_noskins", context).unwrap();
        host.console.cvars.force_write(view, "2").unwrap();
        host.register_client_model_assets();
        let (_, restricted) = draw(&host, rules, record, None);
        let expected = if rules == RuleSetId::Quake2 {
            b"players/male/tris.md2".as_slice()
        } else {
            b"players/female/tris.md2".as_slice()
        };
        assert_eq!(restricted[0].model, model(&host, profile, expected));
    }
}

#[test]
fn rerelease_partial_base_preserves_a_body_without_skin_or_weapon() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let (_fixture, mut host, profile) = loading(rules, b"Name\\male/grunt", true);
        bind(&mut host, profile);
        let (counts, entities) = draw(&host, rules, player_record(rules, 0), None);
        assert!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .rejected
                > 0
        );
        if rules == RuleSetId::Quake2Rerelease {
            assert_eq!((counts.models, counts.missing_models), (1, 1));
            assert_eq!(
                entities[0].model,
                model(&host, profile, b"players/male/tris.md2")
            );
        } else {
            assert_eq!((counts.models, counts.missing_models), (0, 2));
        }
    }
}

#[test]
fn rerelease_named_shell_features_are_cached_per_cell_and_frozen_for_statics() {
    let rules = RuleSetId::Quake2Rerelease;
    let (_fixture, mut host, profile) = loading(rules, b"Name\\female/red", false);
    for path in [
        b"models/items/shell/tris.md2".as_slice(),
        b"models/ordinary.md2",
    ] {
        let runtime = &mut host.runtime;
        assert!(
            runtime
                .model_assets
                .as_mut()
                .unwrap()
                .register_model(&runtime.vfs, path, rules)
                .unwrap()
                > 0
        );
    }
    let layout = ConfigLayout::for_rules(rules);
    for ordinal in [50, 51, 52] {
        assert!(host.runtime.client_resources[0].as_mut().unwrap().apply(
            layout.resources[0].get(ordinal).unwrap(),
            b"MODELS/ITEMS/SHELL/TRIS.MD2",
            None,
            None
        ));
    }
    bind(&mut host, profile);
    let record = EntityRecord {
        position: Vec3([32., 0., 0.]),
        appearance: EntityAppearance {
            source: rules,
            alpha: 0.5,
            models: [50, 51, 52, 0]
                .map(|ordinal| ModelRef::native(ResourceNamespace::Client(ClientId(0)), ordinal)),
            ..Default::default()
        },
        ..Default::default()
    };
    let projection = EntityProjection::load(Protocol::Quake2Repro1038);
    let mut words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &Default::default(),
        &mut words,
        |model, _| Some(model.ordinal),
        |_| None,
        |reference| Some(reference.ordinal())
    ));
    let mut accepted = EntityRecord::default();
    assert!(projection.apply(
        &words,
        &mut accepted,
        ResourceNamespace::Client(ClientId(0))
    ));
    let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
    let retained = [host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .models
        .capture_static(
            accepted,
            &host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .configs,
            Some(catalog),
            None,
        )];
    let (counts, before) = draw(&host, rules, record, None);
    assert_eq!((counts.models, counts.missing_models), (3, 0));
    // Rerelease projects alpha through its native unsigned byte before CLIENT
    // applies the cell's cached shell multiplier.
    let alpha = 127.0f32 / 255.0;
    assert_eq!(
        before.iter().map(|entity| entity.alpha).collect::<Vec<_>>(),
        [Some(alpha), Some(alpha * 0.32), Some(alpha)]
    );
    assert!(host.runtime.client_resources[0].as_mut().unwrap().apply(
        layout.resources[0].get(51).unwrap(),
        b"models/ordinary.md2",
        Some(catalog),
        None
    ));
    let (_, changed) = draw(&host, rules, record, None);
    assert_eq!(changed[1].alpha, Some(alpha));
    let (_, frozen) = draw(&host, rules, record, Some(&retained));
    assert_eq!(frozen[1].alpha, Some(alpha * 0.32));
    assert_eq!(frozen[1].model, before[1].model);
    assert_eq!(frozen[1].material, before[1].material);
}
