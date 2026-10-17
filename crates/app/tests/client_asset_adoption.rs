#[path = "support/client_asset_frames.rs"]
mod frames;
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod support;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::FrameHost,
    map::SpawnAnchor,
    model_assets::{ModelAssets, ModelProfile, ModelRequest},
};
use qa_compat::services::ModelLookup;
use qa_console::{commands::Console, views::Context};
use qa_core::{configstrings::ConfigLayout, primitives::*, sys_events::SeatId};
use qa_network::commands::packet::Protocol;
use qa_render::Assets;
use qa_session::timing::TickRate;

fn setup(
    protocol: Protocol,
    source: RuleSetId,
    files: &[(&str, Vec<u8>)],
) -> (
    support::Fixture,
    FrameHost,
    frames::Source,
    ModelProfile,
    ModelProfile,
) {
    let mut fixture = support::Fixture::new(files);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", source, Some(palette)).profile();
    let mut failed = profile;
    failed.image_settings.max_dimension = 0;
    let owner = ModelAssets::new(
        &fixture.vfs,
        assets,
        [failed, profile],
        [],
        Default::default(),
    )
    .unwrap();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    runtime.model_assets = Some(owner);
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: protocol.rule_set(),
                client: protocol.rule_set(),
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            protocol,
            0,
        )
        .unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    host.bind_client_model_assets([(client, profile, None)]);
    let source = frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
    host.publish_model_admission(source.cpu.admitted_assets().unwrap())
        .unwrap();
    (fixture, host, source, profile, failed)
}
fn bound_model(host: &FrameHost, ordinal: u32) -> Option<ModelId> {
    let owner = host.runtime.model_assets.as_ref().unwrap();
    host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .models
        .resolve(
            ModelRef::native(ResourceNamespace::Client(ClientId(0)), ordinal).unwrap(),
            0,
            owner.catalog(),
            Default::default(),
        )?
        .next()
        .map(|binding| binding.model)
}
#[test]
fn queued_unbound_q3_config_uses_client_profile_in_the_same_frame_and_waits_for_a_backend() {
    for no_backend_first in [false, true] {
        let path = format!("models/{}.md2", "x".repeat(84));
        assert_eq!(path.len(), 95);
        let files = [
            (path.as_str(), support::md2()),
            ("models/test/skin0.pcx", support::pcx(11)),
            ("models/test/skin1.pcx", support::pcx(22)),
        ];
        let (_fixture, mut host, mut source, profile, failed) =
            setup(Protocol::Quake3_68, RuleSetId::Quake2, &files);
        source.step(&mut host);
        source.enabled = !no_backend_first;
        let index = ConfigLayout::QUAKE3.resources[0].get(1).unwrap();
        frames::queue(
            &mut host,
            [frames::config(Protocol::Quake3_68, index, path.as_bytes())],
        );
        host.runtime.vfs.take_lookup_count();
        let attempts = source.attempts;
        source.step(&mut host);
        assert_eq!(
            host.runtime.client_resources[0]
                .as_ref()
                .unwrap()
                .state
                .configs
                .config(index)
                .unwrap()
                .text,
            path.as_bytes()
        );
        if no_backend_first {
            assert_eq!(bound_model(&host, 1), None);
            assert_eq!(source.attempts, attempts);
            assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
            source.enabled = true;
            source.step(&mut host);
        }
        assert!(bound_model(&host, 1).is_some());
        let owner = host.runtime.model_assets.as_ref().unwrap();
        let handle = owner
            .catalog()
            .model_handle(path.as_bytes(), profile.source_rules)
            .unwrap();
        assert!(
            owner
                .catalog()
                .renderer_registration(handle, profile)
                .is_some()
        );
        assert!(
            owner
                .catalog()
                .renderer_registration(handle, failed)
                .is_none()
        );
        assert_eq!(source.cpu.admitted_assets(), Some(owner.assets().counts()));
        assert!(host.runtime.vfs.take_lookup_count() > 0);
        let attempts = source.attempts;
        source.step(&mut host);
        assert_eq!(source.attempts, attempts);
        assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
        frames::queue(
            &mut host,
            [frames::config(Protocol::Quake3_68, index, path.as_bytes())],
        );
        source.step(&mut host);
        assert_eq!(source.attempts, attempts);
        assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    }
}

#[test]
fn queued_qw_static_keeps_original_late_filename_when_the_live_alias_changes() {
    for failed_original in [false, true] {
        let files = [
            (
                "progs/a.mdl",
                if failed_original {
                    b"invalid mdl".to_vec()
                } else {
                    support::mdl()
                },
            ),
            ("progs/b.mdl", support::mdl()),
        ];
        let (_fixture, mut host, mut source, profile, _) =
            setup(Protocol::QuakeWorld28, RuleSetId::Quake, &files);
        source.step(&mut host);
        // Admit the initial peer information before adding model rows. Its
        // player-color registration then shares the model admission frame.
        host.drain(&mut Default::default());
        source.step(&mut host);
        host.drain(&mut Default::default());
        let index = ConfigLayout::QUAKEWORLD.resources[0].get(2).unwrap();
        let record = EntityRecord {
            position: Vec3([32.0, 0.0, 0.0]),
            appearance: EntityAppearance {
                source: RuleSetId::QuakeWorld,
                models: [
                    ModelRef::native(ResourceNamespace::Module(ModuleId(3)), 2),
                    None,
                    None,
                    None,
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bytes = vec![0; 128];
        let len = qa_network::outputs::static_entity(
            Protocol::QuakeWorld28,
            &record,
            |model| Some(model.ordinal),
            &mut bytes,
        )
        .unwrap();
        bytes.truncate(len);
        frames::queue(
            &mut host,
            [
                frames::config(Protocol::QuakeWorld28, index, b"progs/a.mdl"),
                bytes,
                frames::config(Protocol::QuakeWorld28, index, b"progs/b.mdl"),
            ],
        );
        source.step(&mut host);
        let live = bound_model(&host, 2).unwrap();
        let state = &host.runtime.client_resources[0].as_ref().unwrap().state;
        assert_eq!(state.configs.config(index).unwrap().text, b"progs/b.mdl");
        assert_eq!(state.static_entities().len(), 1);
        let (counts, entities) = frames::static_scene(&host, ClientId(0), Protocol::QuakeWorld28);
        if failed_original {
            assert_eq!(counts.models, 0);
            assert_eq!(counts.missing_models, 1);
        } else {
            assert_eq!(counts.models, 1);
            let owner = host.runtime.model_assets.as_ref().unwrap();
            let original = owner
                .catalog()
                .bindings(
                    owner
                        .catalog()
                        .registration("progs/a.mdl", profile, None)
                        .unwrap(),
                    0,
                    Default::default(),
                )
                .unwrap()
                .next()
                .unwrap()
                .model;
            assert_ne!(live, original);
            assert_eq!(entities[0].model, original.into());
        }
        host.runtime.vfs.take_lookup_count();
        let attempts = source.attempts;
        source.step(&mut host);
        assert_eq!(source.attempts, attempts);
        assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    }
}
