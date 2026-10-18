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
                Some(search),
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
                Some(&equivalent_red),
                models.assets(),
                models.shader_occurrence_budget(),
                None
            )
            .unwrap(),
        retained[0].0.context
    );
    assert_eq!(
        NativeShaders::context(&contexts, owner, profile),
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
    for append in contexts
        .stage_graphics(std::iter::once((profile, &native)))
        .unwrap()
    {
        append.commit();
    }
    contexts
        .replay_graphics(retained[0].0.context, std::iter::once(&native))
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
