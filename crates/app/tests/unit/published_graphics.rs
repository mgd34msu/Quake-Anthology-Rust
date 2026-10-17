use super::super::*;
use super::*;
use crate::{
    Runtime, client_policy::ClientPolicy, host::FrameHost, map::SpawnAnchor,
    model_assets::ModelAssets,
};
use qa_console::{
    commands::Console,
    presentation::{ConsoleView, Scrollback},
    views::Context as ConsoleContext,
};
use qa_core::{events::TextStore, loopback::Endpoint, primitives::*, sys_events::EventTime};
use qa_network::commands::packet::Protocol;
use qa_render::material::resources::ImageSettings;
use qa_session::timing::TickRate;
use std::sync::Arc;
#[path = "../support/client_asset_frames.rs"]
mod frames;
#[path = "../model_assets/support.rs"]
#[allow(dead_code)]
mod support;

fn setup(rules: RuleSetId) -> (support::Fixture, FrameHost, frames::Source) {
    let mut files = support::Fixture::new(&[]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let mut owner = ModelAssets::new(assets, [], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            crate::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                Some(palette),
                Some(palette),
                Some(palette),
                None,
            ]),
        )
        .unwrap();
    let console = console::ConsoleAssets::load(
        &files.vfs,
        &mut owner,
        rules,
        ImageSettings::native(rules),
        Default::default(),
        64,
        48,
    )
    .unwrap();
    let font = console.font_assets();
    assert!(!font.native);
    let (limits, ui_commands) = scene_ui_limits(
        Limits::default(),
        0,
        ui_command_capacity(Some(&console), true).unwrap(),
    )
    .unwrap();
    let mut scene = SceneComposer::load(limits, owner.assets_mut(), vec![])
        .unwrap()
        .0;
    scene.ui_commands = ui_commands;
    scene.console = Some(console);
    scene.hud[rules as usize] = Some(hud::HudAssets::from_font(rules, font, 64).unwrap());
    let mut runtime = Runtime::load(1, []).unwrap();
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
                input: rules,
                client: rules,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::QuakeWorld,
            },
            Protocol::Quake2_34,
            0,
        )
        .unwrap();
    runtime.vfs = std::mem::take(&mut files.vfs);
    runtime.scene = Some(scene);
    runtime.model_assets = Some(owner);
    // Keep the native channel and ACKs while this fixture supplies SERVER bytes.
    runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .commands = None;
    let mut host = FrameHost::load(
        Console::new(ConsoleContext::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    host.bind_client_model_assets([]);
    let source = frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
    host.publish_model_admission(source.cpu.admitted_assets().unwrap())
        .unwrap();
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let profile = host
        .runtime
        .scene
        .as_ref()
        .unwrap()
        .console
        .as_ref()
        .unwrap()
        .profile();
    host.renderer_contexts
        .ensure_owner(
            ShaderOwner::Engine,
            profile,
            owner.assets(),
            owner.shader_occurrence_budget(),
            None,
        )
        .unwrap();
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(client, mount, mount, 65536, 256)
        .unwrap();
    let allow = host
        .console
        .cvars
        .bind(
            "cl_allowDownload",
            ConsoleContext {
                source: RuleSetId::Quake2,
                seat: SeatId::FIRST,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(allow, "1").unwrap();
    (files, host, source)
}

fn command(host: &mut FrameHost, source: &mut frames::Source, text: &str) {
    host.console
        .append(
            text,
            ConsoleContext {
                source: RuleSetId::Quake2,
                seat: SeatId::FIRST,
                ..Default::default()
            },
        )
        .unwrap();
    host.execute_commands();
    source.step(host);
}
fn download(host: &mut FrameHost, source: &mut frames::Source, path: &str, bytes: &[u8]) {
    command(host, source, &format!("download {path}\n"));
    for (index, part) in bytes.chunks(1024).enumerate() {
        let mut block = [0; 1100];
        let length = qa_network::downloads::write_chunk(
            Protocol::Quake2_34,
            qa_network::downloads::Chunk::Sequential {
                percent: (((index * 1024 + part.len()) * 100) / bytes.len()) as u8,
                bytes: Some(part),
                compressed: false,
            },
            &mut block,
        )
        .unwrap();
        frames::queue(host, [block[..length].to_vec()]);
        source.step(host);
    }
    assert!(host.runtime.vfs.open(path.as_bytes()).is_some());
}

fn paint(
    host: &mut FrameHost,
    source: &mut frames::Source,
    rules: RuleSetId,
) -> (console::ConsoleStats, Vec<u32>) {
    let mut editor = qa_console::editor::Editor::default();
    editor.toggle(SeatId::FIRST);
    editor.character(SeatId::FIRST, 'A').unwrap();
    let texts = TextStore::load(1, 32).unwrap();
    let log = Scrollback::default();
    let mut front = FrontEnd::load(Default::default()).unwrap();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    let mut scene = host.runtime.scene.take().unwrap();
    let assets = host.runtime.model_assets.as_ref().unwrap().assets();
    let stats = scene.append_console(
        &mut frame,
        assets,
        ConsoleView {
            editor: &editor,
            scrollback: &log,
            texts: &texts,
            rules,
            time: EventTime::default(),
        },
        64,
        48,
        true,
        host.native_shaders(),
    );
    let mut packet = frame.finish();
    packet.resolve_shaders(assets, host.native_shaders());
    assert_eq!(source.cpu.render(&packet, assets).rejected, 0);
    let pixels = source.cpu.pixels().to_vec();
    host.runtime.scene = Some(scene);
    (stats, pixels)
}

#[test]
fn queued_fonts_and_backgrounds_commit_together_with_the_shared_hud_owner() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let (_files, mut host, mut source) = setup(rules);
        source.step(&mut host);
        let old = host
            .runtime
            .scene
            .as_ref()
            .unwrap()
            .hud_font(rules)
            .unwrap();
        let prefix = host
            .runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts();
        let limits = host.runtime.scene.as_ref().unwrap().limits();
        let (stats, before) = paint(&mut host, &mut source, rules);
        assert_eq!(
            (stats.missing_font, stats.missing_background, stats.glyphs),
            (1, 1, 0)
        );
        source.reject = true;
        download(
            &mut host,
            &mut source,
            "pics/conchars.pcx",
            &support::pcx_dimensions(128, 128, 11),
        );
        assert_eq!(
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts(),
            prefix
        );
        assert_eq!(source.cpu.admitted_assets(), Some(prefix));
        assert!(Arc::ptr_eq(
            &old,
            &host
                .runtime
                .scene
                .as_ref()
                .unwrap()
                .hud_font(rules)
                .unwrap()
        ));
        assert_eq!(paint(&mut host, &mut source, rules).1, before);
        source.reject = false;
        command(&mut host, &mut source, "retrydownload\n");
        let scene = host.runtime.scene.as_ref().unwrap();
        let font = scene.hud_font(rules).unwrap();
        assert!(font.native);
        assert!(!Arc::ptr_eq(&old, &font));
        assert!(Arc::ptr_eq(
            &font,
            &scene.hud[rules as usize].as_ref().unwrap().font
        ));
        assert_eq!(scene.limits(), limits);
        let (stats, pixels) = paint(&mut host, &mut source, rules);
        assert!(stats.glyphs > 0);
        assert_eq!(stats.missing_font, 0);
        assert_ne!(pixels, before);
        let counts = host
            .runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts();
        let attempts = source.attempts;
        host.runtime.vfs.take_lookup_count();
        source.step(&mut host);
        assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
        assert_eq!(source.attempts, attempts);
        assert_eq!(
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts(),
            counts
        );

        source.reject = true;
        download(
            &mut host,
            &mut source,
            "pics/conback.pcx",
            &support::pcx_dimensions(16, 16, 22),
        );
        assert_eq!(
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts(),
            counts
        );
        assert_eq!(paint(&mut host, &mut source, rules).1, pixels);
        source.reject = false;
        command(&mut host, &mut source, "retrydownload\n");
        let scene = host.runtime.scene.as_ref().unwrap();
        assert!(Arc::ptr_eq(&font, &scene.hud_font(rules).unwrap()));
        assert!(Arc::ptr_eq(
            &font,
            &scene.hud[rules as usize].as_ref().unwrap().font
        ));
        let (stats, final_pixels) = paint(&mut host, &mut source, rules);
        assert_eq!(
            (
                stats.backgrounds,
                stats.missing_background,
                stats.missing_font,
                stats.rejected
            ),
            (1, 0, 0, 0)
        );
        assert_ne!(final_pixels, pixels);
        assert_eq!(
            source.cpu.admitted_assets(),
            Some(
                host.runtime
                    .model_assets
                    .as_ref()
                    .unwrap()
                    .assets()
                    .counts()
            )
        );
    }
}
