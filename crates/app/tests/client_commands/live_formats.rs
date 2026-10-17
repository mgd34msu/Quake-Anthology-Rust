use super::*;

pub(crate) fn live_formats(native: bool, ui: bool) {
    let jpeg = include_bytes!("../../../render/tests/resources/blue.jpg");
    let mut fixture = model_support::Fixture::new(&[
        ("textures/shared.tga", model_support::tga(2, 2, &[[91, 37, 13, 255]; 4])),
        ("textures/shared.jpeg", jpeg.to_vec()),
        ("textures/fresh.tga", model_support::tga(2, 2, &[[91, 37, 13, 255]; 4])),
        ("textures/fresh.jpeg", jpeg.to_vec()),
        ("scripts/live.shader", b"textures/first { nomipmaps nopicmip { map textures/shared.tga } }\ntextures/alias { { map textures/shared.tga } }\ntextures/fresh { nomipmaps nopicmip { map textures/fresh.tga } }\n".to_vec()),
    ]);
    let mut initial = data();
    for (at, text) in [
        (208, b"r_texture_formats\0".as_slice()),
        (240, b"tga\0"),
        (248, b"jpeg\0"),
        (16, b"textures/first\0"),
        (880, b"textures/alias\0"),
        (900, b"textures/fresh\0"),
    ] {
        initial[at..at + text.len()].copy_from_slice(text);
    }
    let program = if native {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            native_scene_data(ui, &initial)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            qvm_scene_data(ui, &initial)
        }
    } else {
        qvm_scene_data(ui, &initial)
    };
    let mut profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    profile.image_settings.override_textures = 2;
    let (mut runtime, ids) = runtime(1);
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    runtime.model_assets = Some(
        ModelAssets::new(
            &runtime.vfs,
            qa_render::Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap(),
    );
    let mut request = request(&mut runtime, ModuleId(1), ids[0], program);
    request.entries = (0..31).collect();
    if ui {
        request.frame.client = None;
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.local_clients[0] = Some(ids[0]);
    host.configure_native_scenes(
        &[None, None, None, None, Some(profile)],
        &[[None, None, None, None, Some(profile)]; SeatId::COUNT],
        320,
        200,
    )
    .unwrap();
    let registered_image = |host: &FrameHost, name: &[u8]| {
        let owner = host.runtime.model_assets.as_ref().unwrap();
        let shader = owner
            .catalog()
            .shader_outcome(
                name,
                profile,
                qa_compat::services::ShaderLighting::TwoDimensional,
                None,
            )
            .unwrap()
            .shader;
        let stage = &owner
            .assets()
            .material(owner.assets().shader_binding(shader).unwrap().material)
            .unwrap()
            .stages[0];
        let qa_render::assets::StageTexture::Image(image) = stage.texture else {
            panic!("registered image")
        };
        (image, stage.sampler)
    };
    let first = probe_args(&mut host, 0, 30, &[240, 16]);
    assert!(first > 0);
    let (original, sampler) = registered_image(&host, b"textures/first");
    assert!(!sampler.mipmaps);
    assert_eq!(
        &host
            .runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .image(original)
            .unwrap()
            .rgba[..4],
        &[91, 37, 13, 255]
    );
    host.runtime.vfs.take_lookup_count();
    assert_eq!(probe_args(&mut host, 0, 30, &[248, 16]), first);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    let alias = probe_args(&mut host, 0, 30, &[248, 880]);
    assert!(alias > 0 && alias != first);
    assert_eq!(
        registered_image(&host, b"textures/alias"),
        (original, sampler)
    );
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    let fresh = probe_args(&mut host, 0, 30, &[248, 900]);
    assert!(fresh > 0 && fresh != first && fresh != alias);
    let (fresh_image, _) = registered_image(&host, b"textures/fresh");
    assert_ne!(fresh_image, original);
    let rgba = &host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .image(fresh_image)
        .unwrap()
        .rgba;
    assert!(rgba[2] > 230 && rgba[0] < 10);
    assert!(host.runtime.vfs.take_lookup_count() > 0);
    assert_eq!(
        registered_image(&host, b"textures/first"),
        (original, sampler)
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}
