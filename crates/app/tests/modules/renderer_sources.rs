use super::*;
use crate::model_assets::{ModelAssets, ModelRequest, RegistrationVariant};
use qa_compat::services::{RendererRegistration, RendererRequest, ShaderLighting};
use qa_core::primitives::{ClientId, EntityId, RuleSetId};
use qa_render::{
    Assets, Draw2d, FrontEnd,
    cpu::{CpuBackend, CpuLimits},
    shader::reference::{NativeShaders, ShaderRef},
};

#[allow(dead_code)]
#[path = "../model_assets/support.rs"]
mod support;

fn entries(color: [u8; 4]) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "models/player.md3",
            support::md3_named_surfaces(&[(b"body", &[b"textures/body"])]),
        ),
        ("models/player.skin", b"body,textures/body\n".to_vec()),
        ("textures/body.tga", support::tga(1, 1, &[color])),
        (
            "scripts/body.shader",
            b"textures/body { cull none { map textures/body.tga } }".to_vec(),
        ),
    ]
}

fn pixels(
    cpu: &mut CpuBackend,
    assets: &Assets,
    contexts: &Contexts,
    reference: NativeShaderRef,
) -> Vec<u32> {
    let mut frontend = FrontEnd::load(Default::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.draw_2d(Draw2d {
        rect: [0., 0., 8., 8.],
        texcoords: [0., 0., 1., 1.],
        color: [255; 4],
        shader: ShaderRef::Native(reference),
    }));
    frame.resolve_draw_shaders(assets, contexts);
    assert_eq!(cpu.render(&frame.finish(), assets).rejected, 0);
    cpu.pixels().to_vec()
}

#[test]
fn native_contexts_and_public_requests_isolate_sources_including_warm_hits_and_engine_graphics() {
    let colors = [[211, 0, 0, 255], [0, 0, 211, 255], [0, 211, 0, 255]];
    let mut files = support::Fixture::new(&entries(colors[0]));
    let blue = support::Fixture::new(&entries(colors[1]));
    let green = support::Fixture::new(&entries(colors[2]));
    let red_mount = files.vfs.mounts().next().unwrap().id;
    let blue_root = blue.vfs.mounts().next().unwrap().path.clone();
    let green_root = green.vfs.mounts().next().unwrap().path.clone();
    let blue_mount = files.vfs.mount_directory(&blue_root, 10).unwrap();
    files.vfs.mount_directory(&green_root, 20).unwrap();
    let red = files.vfs.search_view([red_mount]).unwrap();
    let blue = files.vfs.search_view([blue_mount]).unwrap();
    let default = files.vfs.default_view();
    let equivalent_red = files.vfs.search_view([red_mount, red_mount]).unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut models = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        8,
        8,
        models.assets(),
        CpuLimits {
            cache_bytes: 1_048_576,
            ..Default::default()
        },
    )
    .unwrap();
    models
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    models.end_initial_registration();
    let mut contexts = Contexts::load(default.clone(), 0);
    let owner = Owner::Client(
        ClientId(0),
        EntityId {
            slot: 1,
            generation: 2,
        },
    );
    let mut retained = Vec::new();
    let mut model_handles = Vec::new();
    let mut skin_handles = Vec::new();
    for (search, color) in [(&red, colors[0]), (&blue, colors[1]), (&default, colors[2])] {
        let context = contexts
            .ensure_owner(
                owner,
                profile,
                &files.vfs.bind_search(Some(search)).unwrap(),
                models.assets(),
                models.shader_occurrence_budget(),
                None,
            )
            .unwrap();
        let mut shader_handle = 0;
        for (name, kind) in [
            (b"models/player.md3".as_slice(), RendererRegistration::Model),
            (b"models/player.skin".as_slice(), RendererRegistration::Skin),
            (
                b"textures/body".as_slice(),
                RendererRegistration::Shader {
                    mipmaps: true,
                    lighting: ShaderLighting::TwoDimensional,
                },
            ),
        ] {
            let request = RendererRequest {
                search: Some(search.clone()),
                name,
                source: profile.source_rules,
                kind,
                world: None,
            };
            let (handle, append, _) = models
                .prepare_registration(
                    &files.vfs,
                    &request,
                    profile,
                    RegistrationVariant::Guard(false),
                    |admission| contexts.stage(context, admission),
                    |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
                .unwrap();
            assert!(handle > 0);
            shader_handle = append.shader_handle();
            append.commit();
            match kind {
                RendererRegistration::Model => model_handles.push(handle),
                RendererRegistration::Skin => skin_handles.push(handle),
                _ => {}
            }
            assert!(
                models
                    .catalog()
                    .registration_cached_variant(
                        kind,
                        name,
                        profile,
                        None,
                        RegistrationVariant::Retained,
                        Some(search)
                    )
                    .is_some()
            );
        }
        let reference = contexts.shader(context, shader_handle).unwrap();
        let image = pixels(&mut cpu, models.assets(), &contexts, reference);
        assert!(image.iter().all(|p| p.to_le_bytes() == color));
        retained.push((reference, image));
    }
    assert!(model_handles.windows(2).all(|pair| pair[0] != pair[1]));
    assert!(skin_handles.windows(2).all(|pair| pair[0] != pair[1]));
    assert_eq!(
        contexts
            .ensure_owner(
                owner,
                profile,
                &files.vfs.bind_search(Some(&equivalent_red)).unwrap(),
                models.assets(),
                models.shader_occurrence_budget(),
                None
            )
            .unwrap(),
        retained[0].0.context
    );
    assert_eq!(
        NativeShaders::context(&contexts, owner, profile, None),
        Some(retained[2].0.context)
    );
    assert_ne!(retained[0].0.context, retained[1].0.context);
    let published = models.assets().counts();
    files.vfs.take_lookup_count();
    for (view, handle) in [(&red, model_handles[0]), (&blue, model_handles[1])] {
        let request = RendererRequest {
            search: Some(view.clone()),
            name: b"models/player.md3".as_slice(),
            source: profile.source_rules,
            kind: RendererRegistration::Model,
            world: None,
        };
        let (cached, (), _) = models
            .prepare_registration(
                &files.vfs,
                &request,
                profile,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("warm registration reached backend"),
            )
            .unwrap();
        assert_eq!(cached, handle);
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(models.assets().counts(), published);
    let native = NativeShaders::source(&contexts, retained[2].0)
        .copied()
        .unwrap();
    let default_files = files.vfs.bind_search(None).unwrap();
    let graphic = crate::renderer::GraphicRegistration {
        profile,
        files: &default_files,
        registration: &native,
    };
    for append in contexts.stage_graphics(std::iter::once(graphic)).unwrap() {
        append.commit();
    }
    contexts
        .replay_graphics(retained[0].0.context, std::iter::once(graphic))
        .unwrap();
    for (reference, expected) in &retained {
        assert_eq!(
            pixels(&mut cpu, models.assets(), &contexts, *reference),
            *expected
        );
    }
    let foreign = qa_content::vfs::Vfs::default();
    let foreign_request = RendererRequest {
        search: Some(foreign.default_view()),
        name: b"models/player.md3".as_slice(),
        source: profile.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    };
    assert!(
        models
            .prepare_registration(
                &files.vfs,
                &foreign_request,
                profile,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("foreign source reached backend")
            )
            .is_err()
    );
    assert_eq!(models.assets().counts(), published);
}

#[test]
fn same_rule_client_huds_keep_source_pixels_and_rejected_replacement_keeps_old_slot() {
    use crate::renderer::{SceneComposer, hud::HudView};
    use qa_core::{
        configstrings::ConfigStrings,
        events::TextStore,
        primitives::{HudLine, HudState, ThinkTime},
        sys_events::{EventTime, SeatId},
    };
    use qa_render::{
        Refdef, Viewport,
        material::{ShaderCompilePolicy, resources::ImageSettings},
    };
    let colors = [[211, 0, 0, 255], [0, 0, 211, 255]];
    let font_files = |color| {
        [(
            "gfx/2d/bigchars.tga",
            support::tga(128, 128, &vec![color; 128 * 128]),
        )]
    };
    let mut files = support::Fixture::new(&font_files(colors[0]));
    let other = support::Fixture::new(&font_files(colors[1]));
    let first = files.vfs.mounts().next().unwrap().id;
    let path = other.vfs.mounts().next().unwrap().path.clone();
    let second = files.vfs.mount_directory(&path, 10).unwrap();
    let searches = [first, second].map(|mount| files.vfs.search_view([mount]).unwrap());
    let sources = searches
        .each_ref()
        .map(|view| files.vfs.bind_search(Some(view)).unwrap());
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut models = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let (mut scene, limits) =
        SceneComposer::load(Default::default(), models.assets_mut(), vec![]).unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        32,
        16,
        models.assets(),
        CpuLimits {
            scene: limits,
            cache_bytes: 1_048_576,
            ..Default::default()
        },
    )
    .unwrap();
    models
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    models.end_initial_registration();
    let mut contexts = Contexts::load(files.vfs.default_view(), 0);
    let lifetime = EntityId {
        slot: 1,
        generation: 1,
    };
    for seat in 0..2 {
        let id = SeatId::new(seat as u8).unwrap();
        let reuse = scene.hud_font(RuleSetId::Quake3, Some(SeatId::FIRST), sources[0].id());
        let (append, _) = models
            .prepare_hud(
                &files.vfs,
                RuleSetId::Quake3,
                ImageSettings::native(RuleSetId::Quake3),
                ShaderCompilePolicy::default(),
                32,
                reuse,
                Some(&searches[seat]),
                |view, candidate| {
                    let authority = Owner::Client(ClientId(seat as u32), lifetime);
                    contexts.ensure_owner(
                        authority,
                        candidate.profile(),
                        &candidate.font().files,
                        view.assets,
                        (0, 0),
                        None,
                    )?;
                    let font = candidate.font();
                    let appends = contexts.stage_graphics(
                        candidate
                            .registrations()
                            .map(move |registration| font.registered(registration)),
                    )?;
                    Ok((scene.stage_hud(candidate, Some(id))?, appends))
                },
                |assets, (append, _)| {
                    cpu.prepare_assets(assets, Some(append.limits()))
                        .map_err(str::to_owned)
                },
            )
            .unwrap();
        for graph in append.1 {
            graph.commit();
        }
        append.0.commit();
    }
    let before = models.assets().counts();
    let rejected = models.prepare_hud(
        &files.vfs,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        ShaderCompilePolicy::default(),
        32,
        None,
        Some(&searches[1]),
        |_, candidate| scene.stage_hud(candidate, Some(SeatId::FIRST)),
        |_, _| Err("backend rejects slot replacement".to_owned()),
    );
    assert!(rejected.is_err());
    drop(rejected);
    assert_eq!(models.assets().counts(), before);
    assert!(scene.has_hud(RuleSetId::Quake3, Some(SeatId::FIRST), sources[0].id()));
    assert!(!scene.has_hud(RuleSetId::Quake3, Some(SeatId::FIRST), sources[1].id()));
    // Video recreation must reload each slot from its own view, even with
    // matching native profiles and a higher-priority conflicting font.
    let refresh = crate::model_assets::ImageRefresh::default();
    let graphics = scene
        .load_video_graphics(&files.vfs, &mut models, &refresh, false, 32, 16)
        .unwrap();
    let append = scene
        .stage_video_graphics(
            models.assets(),
            graphics,
            &refresh,
            models.catalog().max_choice_nodes(),
        )
        .unwrap();
    let graphs = contexts
        .stage_graphics(append.graphic_registrations())
        .unwrap();
    cpu.prepare_assets(models.assets(), Some(append.limits()))
        .unwrap();
    for graph in graphs {
        graph.commit();
    }
    append.commit(crate::renderer::Kind::Cpu);
    let values = qa_ui::values::ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let mut texts = TextStore::load(1, 32).unwrap();
    hud.notify[0] = Some(HudLine {
        text: texts.insert(b"A\n").unwrap(),
        started_at: 0.9,
        until: 2.0,
    });
    let layout = qa_network::commands::packet::Protocol::Quake3_68.resource_layout();
    let configs =
        ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
            .unwrap();
    let localization = qa_content::localization::Catalog::default();
    let mut front = FrontEnd::load(scene.limits()).unwrap();
    files.vfs.take_lookup_count();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    for seat in 0..2 {
        let view = HudView {
            seat: SeatId::new(seat as u8),
            file_source: sources[seat].id(),
            hud: &hud,
            texts: &texts,
            configs: &configs,
            layout,
            values: &values,
            localization: &localization,
            rules: RuleSetId::Quake3,
            time: EventTime::from_milliseconds(1000),
            style_time: ThinkTime::Milliseconds(1000),
            server_frame: 0,
            client: ClientId(seat as u32),
            client_lifetime: lifetime,
            style_interpolation: 0,
            show_text: true,
        };
        let stats = scene.append_hud(
            &mut frame,
            models.assets(),
            view,
            Refdef {
                viewport: Viewport {
                    x: seat as u32 * 16,
                    y: 0,
                    width: 16,
                    height: 16,
                },
                ..Default::default()
            },
            &contexts,
            Owner::Client(ClientId(seat as u32), lifetime),
        );
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.glyphs, 1);
    }
    frame.resolve_draw_shaders(models.assets(), &contexts);
    let packet = frame.finish();
    assert_eq!(packet.rejected, 0);
    assert_eq!(cpu.render(&packet, models.assets()).rejected, 0);
    for seat in 0..2 {
        let painted: Vec<_> = cpu
            .pixels()
            .chunks_exact(32)
            .flat_map(|row| row[seat * 16..seat * 16 + 16].iter().copied())
            .filter(|color| *color != u32::from_le_bytes([3, 5, 7, 255]))
            .collect();
        assert!(!painted.is_empty());
        assert!(
            painted
                .iter()
                .all(|color| *color == u32::from_le_bytes(colors[seat]))
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
}
