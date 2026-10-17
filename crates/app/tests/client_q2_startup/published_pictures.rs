use super::*;
use qa_app::renderer::hud::{HudStats, HudView};
use qa_render::{
    Command, Draw2d, FrontEnd, Refdef, Viewport,
    shader::{
        reference::NativeShaderRef,
        registration::{ShaderOwner, ShaderResource},
    },
};

fn paint(host: &mut FrameHost, source: &mut Source) -> (HudStats, Vec<Draw2d>, Vec<u32>) {
    let values = qa_ui::values::ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let mut texts = qa_core::events::TextStore::load(1, 32).unwrap();
    hud.notify[0] = Some(HudLine {
        text: texts.insert(b"A\n").unwrap(),
        started_at: 0.9,
        until: 2.0,
    });
    let mut scene = host.runtime.scene.take().unwrap();
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let resources = &host.runtime.client_resources[0].as_ref().unwrap().state;
    let lifetime = host.runtime.server.clients[0].entity;
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    let stats = scene.append_hud(
        &mut frame,
        owner.assets(),
        HudView {
            hud: &hud,
            texts: &texts,
            configs: &resources.configs,
            layout: resources.layout,
            values: &values,
            localization: &qa_content::localization::Catalog::default(),
            rules: host.runtime.server.clients[0].client_rules,
            time: EventTime::from_milliseconds(1000),
            style_time: ThinkTime::Milliseconds(1000),
            server_frame: 0,
            client: ClientId(0),
            client_lifetime: lifetime,
            style_interpolation: 0,
            show_text: true,
        },
        Refdef {
            viewport: Viewport {
                width: 64,
                height: 48,
                ..Default::default()
            },
            ..Default::default()
        },
        host.native_shaders(),
        ShaderOwner::Client(ClientId(0), lifetime),
    );
    let mut packet = frame.finish();
    packet.resolve_shaders(owner.assets(), host.native_shaders());
    let draws = packet
        .commands()
        .iter()
        .filter_map(|command| {
            if let Command::Draw2d(draw) = command {
                Some(*draw)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(packet.rejected, 0);
    assert_eq!(
        source.backend.cpu.render(&packet, owner.assets()).rejected,
        0
    );
    let pixels = source.backend.cpu.pixels().to_vec();
    host.runtime.scene = Some(scene);
    (stats, draws, pixels)
}

fn verify(protocol: Protocol) {
    let rules = protocol.rule_set();
    let (_fixture, mut host, mut source) = setup(
        &[
            ("pics/conchars.pcx", support::pcx_dimensions(128, 128, 1)),
            ("pics/keep.pcx", support::pcx(11)),
        ],
        &[protocol],
    );
    source.hud_target = true;
    step(&mut host, &mut source);
    assert!(host.runtime.scene.as_ref().unwrap().has_hud(rules));
    let statusbar = layout(&host, 0).statusbar().unwrap();
    send(
        &mut host,
        &mut source,
        0,
        &config(
            protocol,
            statusbar,
            b"xl 4 yt 6 picn late xl 20 yt 6 picn keep",
        ),
    );
    let (before, old_draws, old_pixels) = paint(&mut host, &mut source);
    assert_eq!(
        (before.pictures, before.missing_picture, before.rejected),
        (1, 1, 0)
    );
    assert!(before.glyphs > 0);
    assert_eq!(old_pixels[6 * 64 + 4], u32::from_le_bytes([3, 5, 7, 255]));
    // Non-mipmapped Q2 pictures use gamma without the skin intensity scale.
    assert_eq!(
        old_pixels[6 * 64 + 20],
        u32::from_le_bytes([11, 244, 5, 255])
    );
    let old_keep = old_draws
        .iter()
        .find(|draw| draw.rect == [20.0, 6.0, 8.0, 4.0])
        .unwrap()
        .shader;
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let font = owner
        .assets()
        .find_shader_name(b"pics/conchars.pcx")
        .unwrap();
    // Locate the retained font receipt actually consumed by the glyph draw.
    // RR's live graphics policy can differ from its initial model profile.
    let fonts: Vec<_> = owner
        .catalog()
        .profiles()
        .filter_map(|profile| {
            let context = host
                .native_shaders()
                .context(ShaderOwner::Engine, profile)?;
            let node = *host
                .native_shaders()
                .resource_nodes(context, ShaderResource::Graphic(font))?
                .first()?;
            let receipt = host
                .native_shaders()
                .source(NativeShaderRef { context, node })?;
            old_draws
                .iter()
                .any(|draw| draw.shader.source(host.native_shaders()) == Some(receipt.shader))
                .then_some((profile, context))
        })
        .collect();
    assert_eq!(fonts.len(), 1);
    let (profile, engine_context) = fonts[0];
    assert_eq!(profile.source_rules, rules);
    let client = ShaderOwner::Client(ClientId(0), host.runtime.server.clients[0].entity);
    let (authority, context) = host
        .native_shaders()
        .context(client, profile)
        .map_or((ShaderOwner::Engine, engine_context), |context| {
            (client, context)
        });
    if protocol == RERELEASE {
        assert_eq!(authority, ShaderOwner::Engine);
    } else {
        assert_eq!(authority, client);
    }
    let font_node = host
        .native_shaders()
        .resource_nodes(context, ShaderResource::Graphic(font))
        .unwrap()[0];
    let font_receipt = *host
        .native_shaders()
        .source(NativeShaderRef {
            context,
            node: font_node,
        })
        .unwrap();
    let prefix = owner.assets().counts();
    let limits = host.runtime.scene.as_ref().unwrap().limits();
    let mount = host.runtime.vfs.mounts().next().unwrap().id;
    host.configure_client_downloads(ClientId(0), mount, mount, 8192, 64)
        .unwrap();
    let allow = host
        .console
        .cvars
        .bind(
            "cl_allowDownload",
            Context {
                source: rules,
                side: qa_console::catalog::Scope::Client,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(allow, "1").unwrap();
    command(&mut host, &mut source, 0, "download pics/late.tga\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download pics/late.tga"), 1);
    source.reject_assets = true;
    let attempts = source.backend.attempts;
    chunk(
        &mut host,
        &mut source,
        0,
        100,
        &support::tga(8, 4, &[[37, 218, 18, 255]; 32]),
    );
    settle(&mut host, &mut source);
    assert!(source.backend.attempts > attempts);
    assert!(host.runtime.vfs.open(b"pics/late.tga").is_some());
    let owner = host.runtime.model_assets.as_ref().unwrap();
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(source.backend.cpu.admitted_assets(), Some(prefix));
    assert_eq!(host.runtime.scene.as_ref().unwrap().limits(), limits);
    assert!(
        owner
            .assets()
            .find_shader_name(b"pics/late.pcx")
            .and_then(|key| {
                host.native_shaders()
                    .resource_nodes(context, ShaderResource::Graphic(key))
            })
            .is_none()
    );
    let (rejected, rejected_draws, rejected_pixels) = paint(&mut host, &mut source);
    assert_eq!(
        (rejected.pictures, rejected.missing_picture, rejected.glyphs),
        (before.pictures, before.missing_picture, before.glyphs)
    );
    assert_eq!(rejected_draws.len(), old_draws.len());
    for (after, before) in rejected_draws.iter().zip(&old_draws) {
        assert_eq!(
            (after.rect, after.texcoords, after.color, after.shader),
            (before.rect, before.texcoords, before.color, before.shader)
        );
    }
    assert_eq!(rejected_pixels, old_pixels);
    source.reject_assets = false;
    command(&mut host, &mut source, 0, "retrydownload\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download pics/late.tga"), 1);
    assert_eq!(source.servers[0].count(b"nextdl"), 0);
    let (after, draws, pixels) = paint(&mut host, &mut source);
    assert_eq!(
        (after.pictures, after.missing_picture, after.rejected),
        (2, 0, 0)
    );
    assert_eq!(after.glyphs, before.glyphs);
    assert_eq!(pixels[6 * 64 + 4], u32::from_le_bytes([37, 218, 18, 255]));
    assert_eq!(pixels[6 * 64 + 20], old_pixels[6 * 64 + 20]);
    assert_eq!(
        draws
            .iter()
            .find(|draw| draw.rect == [20.0, 6.0, 8.0, 4.0])
            .unwrap()
            .shader,
        old_keep
    );
    assert_eq!(
        host.native_shaders().context(authority, profile),
        Some(context)
    );
    assert_eq!(
        host.native_shaders().source(NativeShaderRef {
            context,
            node: font_node
        }),
        Some(&font_receipt)
    );
    let owner = host.runtime.model_assets.as_ref().unwrap();
    let picture = owner.assets().find_shader_name(b"pics/late.pcx").unwrap();
    let nodes = host
        .native_shaders()
        .resource_nodes(context, ShaderResource::Graphic(picture))
        .unwrap()
        .to_vec();
    assert_eq!(nodes.len(), 1);
    let admitted = owner.assets().counts();
    assert_eq!(source.backend.cpu.admitted_assets(), Some(admitted));
    assert_eq!(host.runtime.scene.as_ref().unwrap().limits(), limits);
    let attempts = source.backend.attempts;
    command(&mut host, &mut source, 0, "download pics/late.tga\n");
    settle(&mut host, &mut source);
    assert_eq!(source.servers[0].count(b"download pics/late.tga"), 1);
    assert_eq!(source.backend.attempts, attempts);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        admitted
    );
    assert_eq!(
        host.native_shaders()
            .resource_nodes(context, ShaderResource::Graphic(picture)),
        Some(nodes.as_slice())
    );
}

#[test]
fn published_picture_admits_into_the_existing_hud_only_after_backend_accepts() {
    for protocol in [CLASSIC, RERELEASE] {
        verify(protocol);
    }
}
