use super::registration_tests::support;
use super::*;

#[test]
fn model_dependencies_and_cached_sources_keep_their_selected_mounts() {
    let mut files =
        support::Fixture::new(&[("models/player.md2", support::md2_skin(b"skins/low.pcx"))]);
    let high =
        support::Fixture::new(&[("models/player.md2", support::md2_skin(b"skins/high.pcx"))]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [],
        [],
        Default::default(),
    )
    .unwrap();
    for (view, expected) in [
        (&low, b"skins/low.pcx".as_slice()),
        (&high, b"skins/high.pcx"),
    ] {
        assert_eq!(
            owner
                .source_image(files.vfs.read_view(Some(view)), b"models/player.md2", 0)
                .unwrap(),
            Some(expected)
        );
    }
    let low_id = owner
        .cached_model_source(&low, b"models/player.md2")
        .unwrap();
    let high_id = owner
        .cached_model_source(&high, b"models/player.md2")
        .unwrap();
    assert_ne!(low_id, high_id);
    assert_eq!(
        owner.catalog.sources.name(low_id),
        owner.catalog.sources.name(high_id)
    );
    assert_eq!(
        owner
            .source_image(files.vfs.read_view(None), b"models/player.md2", 0)
            .unwrap(),
        Some(b"skins/high.pcx".as_slice())
    );
    let default = files.vfs.default_view();
    let default_id = owner
        .cached_model_source(&default, b"models/player.md2")
        .unwrap();
    assert_ne!(default_id, high_id);
    let same_low = files.vfs.search_view([low_mount, low_mount]).unwrap();
    files.vfs.take_lookup_count();
    assert_eq!(
        owner
            .source_image(
                files.vfs.read_view(Some(&same_low)),
                b"MODELS\\PLAYER.MD2",
                1
            )
            .unwrap(),
        Some(b"skins/low.pcx".as_slice())
    );
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(
        owner.cached_model_source(&same_low, b"models/player.md2"),
        Some(low_id)
    );
    let foreign = Vfs::default();
    assert!(
        owner
            .source_image(foreign.read_view(Some(&low)), b"models/player.md2", 0)
            .is_err()
    );
    assert!(
        owner
            .model_source(foreign.read_view(Some(&high)), "models/player.md2")
            .is_err()
    );
    let empty = files.vfs.search_view([]).unwrap();
    assert_eq!(
        owner
            .source_image(files.vfs.read_view(Some(&empty)), b"models/player.md2", 0)
            .unwrap(),
        None
    );
    assert_eq!(
        owner.cached_model_source(&empty, b"models/player.md2"),
        None
    );
    assert_eq!(
        owner.cached_model_source(&default, b"models/player.md2"),
        Some(default_id)
    );
}

#[test]
fn every_model_lod_uses_the_same_source_selection() {
    let base = support::md3_named_surfaces(&[(b"base", &[b"textures/low"])]);
    let lod = support::md3_named_surfaces(&[(b"lod", &[b"textures/low-lod"])]);
    let mut files = support::Fixture::new(&[
        ("models/test.md3", base.clone()),
        ("models/test_1.md3", lod.clone()),
    ]);
    let high_base = support::md3_named_surfaces(&[(b"base", &[b"textures/high"])]);
    let high = support::Fixture::new(&[
        ("models/test.md3", high_base.clone()),
        ("models/test_2.md3", base.clone()),
    ]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [],
        [],
        Default::default(),
    )
    .unwrap();
    let low_lods = owner
        .model_lods(files.vfs.read_view(Some(&low)), "models/test.md3")
        .map_err(|e| e.message)
        .unwrap();
    let high_lods = owner
        .model_lods(files.vfs.read_view(Some(&high)), "models/test.md3")
        .map_err(|e| e.message)
        .unwrap();
    assert_eq!(low_lods.native_count, 2);
    assert_eq!(high_lods.native_count, 2);
    assert!(matches!(low_lods.slots[2], ModelLodSlot::Missing));
    assert!(matches!(high_lods.slots[1], ModelLodSlot::Missing));
    for (source, expected) in [
        (low_lods.source(0).unwrap(), base),
        (low_lods.source(1).unwrap(), lod),
        (high_lods.source(0).unwrap(), high_base),
    ] {
        let ModelSourceFile::Ready { bytes, .. } = &owner.catalog.sources[source] else {
            panic!("decoded LOD");
        };
        assert_eq!(bytes.as_ref(), expected);
    }
    assert_ne!(low_lods.source(0), high_lods.source(0));
}

#[test]
fn scoped_source_capacity_and_invalidation_preserve_healthy_records() {
    let files = support::Fixture::new(&[]);
    let mount = files.vfs.mounts().next().unwrap().id;
    let fixed = files.vfs.search_view([mount]).unwrap();
    let default = files.vfs.default_view();
    let empty = files.vfs.search_view([]).unwrap();
    let mut sources = ModelSources::load(2);
    let first = sources
        .intern(fixed.clone(), NameId(1))
        .map_err(|e| e.message)
        .unwrap();
    let second = sources
        .intern(default.clone(), NameId(1))
        .map_err(|e| e.message)
        .unwrap();
    for id in [first, second] {
        sources[id] = ModelSourceFile::Invalid {
            supports_lods: false,
            error: FileError::file("bad model".to_owned()),
        };
    }
    assert!(matches!(
        sources.intern(empty.clone(), NameId(1)),
        Err(FileError {
            kind: CallError::Capacity,
            ..
        })
    ));
    assert_eq!(sources.find(&empty, NameId(1)), None);
    assert_eq!(
        sources
            .intern(fixed.clone(), NameId(1))
            .map_err(|e| e.message)
            .unwrap(),
        first
    );
    assert!(sources.invalidate(NameId(1)));
    for id in [first, second] {
        assert!(matches!(sources[id], ModelSourceFile::Empty));
    }
    assert!(!sources.invalidate(NameId(1)));
    assert_eq!(sources.find(&fixed, NameId(1)), Some(first));
    assert_eq!(sources.find(&default, NameId(1)), Some(second));
}

#[test]
fn warm_renderer_resources_reject_a_different_vfs_before_cached_admission() {
    let files = support::Fixture::surfaces();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let model = owner
        .register_model(&files.vfs, b"models/test.md3", profile.source_rules)
        .unwrap();
    let shader = owner
        .register_shader(
            &files.vfs,
            b"textures/body",
            profile.source_rules,
            true,
            ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let skin = owner
        .register_skin(&files.vfs, b"models/test.skin", profile.source_rules)
        .unwrap();
    let counts = owner.assets.counts();
    let foreign = Vfs::default();
    assert_eq!(
        owner.register_model(&foreign, b"models/test.md3", profile.source_rules),
        Err(CallError::File)
    );
    assert_eq!(
        owner.precache(&foreign, b"models/test.md3", profile.source_rules),
        Err(CallError::File)
    );
    assert_eq!(
        owner.register_shader(
            &foreign,
            b"textures/body",
            profile.source_rules,
            true,
            ShaderLighting::TwoDimensional
        ),
        Err(CallError::File)
    );
    assert_eq!(
        owner.register_skin(&foreign, b"models/test.skin", profile.source_rules),
        Err(CallError::File)
    );
    let request = RendererRequest {
        search: None,
        name: b"models/test.md3".as_slice(),
        source: profile.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    };
    assert!(
        owner
            .prepare_registration(
                &foreign,
                &request,
                profile,
                RegistrationVariant::Retained,
                |_| panic!("must not stage a foreign cached handle"),
                |_, _: &()| panic!("must not prepare foreign resources")
            )
            .is_err()
    );
    assert_eq!(owner.assets.counts(), counts);
    files.vfs.take_lookup_count();
    assert_eq!(
        owner.register_model(&files.vfs, b"models/test.md3", profile.source_rules),
        Ok(model)
    );
    assert_eq!(
        owner.register_shader(
            &files.vfs,
            b"textures/body",
            profile.source_rules,
            true,
            ShaderLighting::TwoDimensional
        ),
        Ok(shader)
    );
    assert_eq!(
        owner.register_skin(&files.vfs, b"models/test.skin", profile.source_rules),
        Ok(skin)
    );
    assert_eq!(files.vfs.take_lookup_count(), 0);
}

#[test]
fn shader_catalog_cache_keeps_selected_definitions_and_bounds_new_scopes() {
    let low_script = b"low_only { { map textures/low } } shared { { map textures/low } }";
    let mut files = support::Fixture::new(&[("scripts/world.shader", low_script.to_vec())]);
    let high = support::Fixture::new(&[(
        "scripts/world.shader",
        b"high_only { { map textures/high } } shared { { map textures/high } }".to_vec(),
    )]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let empty = files.vfs.search_view([]).unwrap();
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [],
        [],
        ModelLimits {
            models: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let low_id = owner.shader_catalog(&files.vfs, Some(&low)).unwrap();
    let high_id = owner.shader_catalog(&files.vfs, Some(&high)).unwrap();
    let empty_id = owner.shader_catalog(&files.vfs, Some(&empty)).unwrap();
    let default_id = owner.shader_catalog(&files.vfs, None).unwrap();
    assert_ne!(low_id, high_id);
    assert_ne!(default_id, high_id);
    for (id, present, absent) in [
        (low_id, "low_only", "high_only"),
        (high_id, "high_only", "low_only"),
        (default_id, "high_only", "low_only"),
    ] {
        let catalog = owner.catalog.shader_catalogs.get(id).as_ref().unwrap();
        assert!(catalog.find_canonical(present).is_some());
        assert!(catalog.find_canonical(absent).is_none());
        assert_eq!(catalog.definitions.len(), 2);
        assert!(catalog.diagnostics.is_empty());
    }
    assert!(
        owner
            .catalog
            .shader_catalogs
            .get(empty_id)
            .as_ref()
            .unwrap()
            .definitions
            .is_empty()
    );
    let foreign = Vfs::default();
    assert_eq!(
        owner.shader_catalog(&foreign, Some(&low)),
        Err(CallError::File)
    );
    let other = files.vfs.search_view([high_mount, low_mount]).unwrap();
    assert_eq!(
        owner.shader_catalog(&files.vfs, Some(&other)),
        Err(CallError::Capacity)
    );
    let root = files.vfs.directory_path(low_mount).unwrap();
    qa_platform::files::write(
        root.join("scripts/world.shader"),
        vec![b' '; low_script.len()],
    )
    .unwrap();
    let same_low = files.vfs.search_view([low_mount, low_mount]).unwrap();
    assert_eq!(
        owner.shader_catalog(&files.vfs, Some(&same_low)).unwrap(),
        low_id
    );
    assert!(
        owner
            .catalog
            .shader_catalogs
            .get(low_id)
            .as_ref()
            .unwrap()
            .find_canonical("low_only")
            .is_some()
    );
}

#[test]
fn constructed_resources_reject_first_access_through_a_foreign_vfs() {
    let files = support::Fixture::surfaces();
    let other = support::Fixture::surfaces();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let counts = owner.assets.counts();
    assert_eq!(
        owner.register_model(&other.vfs, b"models/test.md3", profile.source_rules),
        Err(CallError::File)
    );
    assert_eq!(
        owner.register_shader(
            &other.vfs,
            b"textures/body",
            profile.source_rules,
            false,
            ShaderLighting::TwoDimensional
        ),
        Err(CallError::File)
    );
    assert_eq!(
        owner.register_skin(&other.vfs, b"models/test.skin", profile.source_rules),
        Err(CallError::File)
    );
    assert!(
        owner
            .model_source(other.vfs.read_view(None), "models/test.md3")
            .is_err()
    );
    assert!(
        owner
            .source_image(other.vfs.read_view(None), b"models/test.md3", 0)
            .is_err()
    );
    assert_eq!(owner.assets.counts(), counts);
    assert_eq!(owner.catalog.names.len(), 1);
    assert!(
        owner
            .register_model(&files.vfs, b"models/test.md3", profile.source_rules)
            .unwrap()
            > 0
    );
    let row = &owner.catalog.profiles[0];
    assert!(row.source.same_selection(&files.vfs.default_view()));
}

#[test]
fn image_policy_sharing_keeps_source_selection_independent_of_presentation() {
    let mut files = support::Fixture::new(&[(
        "textures/pic.tga",
        support::tga(4, 4, &[[255, 0, 0, 255]; 16]),
    )]);
    let high = support::Fixture::new(&[(
        "textures/pic.tga",
        support::tga(4, 4, &[[0, 0, 255, 255]; 16]),
    )]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut assets = Assets::load().unwrap();
    let mut policies = Vec::new();
    let catalog = qa_render::shader::parse_sources(&[]);
    for (view, expected) in [
        (&low, [255, 0, 0, 255]),
        (&high, [0, 0, 255, 255]),
        (&low, [255, 0, 0, 255]),
    ] {
        let (entry, _) = ProfileImages::new(profile, view.clone(), &policies, &mut assets).unwrap();
        let index = policies.len();
        policies.push(entry);
        let registered = with_image_owner(&mut policies, index, |images| {
            images.compile(
                &files.vfs,
                &mut assets,
                Ok(&catalog),
                b"textures/pic.tga",
                false,
                NamedShaderLighting {
                    source: LightSource::None,
                    lightmap: ImageId(0),
                },
                false,
            )
        })
        .unwrap();
        let binding = assets.shader_binding(registered.shader).unwrap();
        let material = assets.material(binding.material).unwrap();
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("source image");
        };
        assert_eq!(&assets.image(image).unwrap().rgba[..4], &expected);
    }
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let mut shaders = Vec::new();
    for (view, expected) in [
        (&low, [255, 0, 0, 255]),
        (&high, [0, 0, 255, 255]),
        (&low, [255, 0, 0, 255]),
    ] {
        let handle = owner
            .preload_shader(
                &files.vfs,
                Some(view),
                b"textures/pic.tga",
                RuleSetId::Quake3,
                false,
                ShaderLighting::TwoDimensional,
            )
            .unwrap();
        shaders.push((handle, expected));
    }
    assert_eq!(shaders[0].0, shaders[2].0);
    assert_ne!(shaders[0].0, shaders[1].0);
    let (owner, mut cpu) = super::registration_tests::publish(owner);
    for (handle, expected) in shaders {
        let binding = owner.catalog.shader_binding(handle, profile).unwrap();
        let pixels = super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected))
        );
    }
}

#[test]
fn retained_model_records_compile_and_paint_their_own_source_images() {
    let model = support::md3_named_surfaces(&[(b"body", &[b"textures/body"])]);
    let script = b"textures/body { cull none { map textures/pic.tga rgbGen identity } }";
    let mut files = support::Fixture::new(&[
        ("models/player.md3", model.clone()),
        (
            "scripts/player.shader",
            [
                script.as_slice(),
                b" textures/skin-a { cull none { map textures/skin-a.tga rgbGen identity } }",
            ]
            .concat(),
        ),
        ("models/player.skin", b"body,textures/skin-a\n".to_vec()),
        (
            "textures/skin-a.tga",
            support::tga(4, 4, &[[255, 0, 0, 255]; 16]),
        ),
        (
            "textures/pic.tga",
            support::tga(4, 4, &[[255, 0, 0, 255]; 16]),
        ),
    ]);
    let high = support::Fixture::new(&[
        ("models/player.md3", model),
        (
            "scripts/player.shader",
            [
                script.as_slice(),
                b" textures/skin-b { cull none { map textures/skin-b.tga rgbGen identity } }",
            ]
            .concat(),
        ),
        ("models/player.skin", b"body,textures/skin-b\n".to_vec()),
        (
            "textures/skin-b.tga",
            support::tga(4, 4, &[[0, 0, 255, 255]; 16]),
        ),
        (
            "textures/pic.tga",
            support::tga(4, 4, &[[0, 0, 255, 255]; 16]),
        ),
    ]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let request = ModelRequest::native("models/player.md3", RuleSetId::Quake3, None);
    let requests = [&low, &high].map(|view| ModelRequest {
        search: Some(view),
        ..request
    });
    let profile = request.profile();
    let mut owner = ModelAssets::load(
        &files.vfs,
        Assets::load().unwrap(),
        requests,
        [],
        Default::default(),
    )
    .unwrap();
    let mut records = [&low, &high]
        .into_iter()
        .zip([[255, 0, 0, 255], [0, 0, 255, 255]])
        .map(|(view, expected)| {
            (
                owner
                    .catalog
                    .file_id(request.path.as_bytes(), Some(view))
                    .unwrap(),
                expected,
            )
        })
        .collect::<Vec<_>>();
    let empty = files.vfs.search_view([]).unwrap();
    owner
        .register_request(
            &files.vfs,
            ModelRequest {
                search: Some(&empty),
                required: false,
                ..request
            },
        )
        .unwrap();
    assert!(
        owner
            .register_request(
                &files.vfs,
                ModelRequest {
                    search: Some(&empty),
                    ..request
                },
            )
            .is_err()
    );
    assert!(
        owner
            .catalog
            .metadata(request.path.as_bytes(), Some(&empty))
            .is_none()
    );
    let default_model = owner
        .register_model(&files.vfs, b"models/player.md3", RuleSetId::Quake3)
        .unwrap();
    assert!(
        records
            .iter()
            .all(|(file, _)| file.0 as i32 != default_model)
    );
    let mut shaders = Vec::new();
    let mut skins = Vec::new();
    for (view, expected) in [(&low, [255, 0, 0, 255]), (&high, [0, 0, 255, 255])] {
        let skin = owner
            .preload_skin(
                &files.vfs,
                b"models/player.skin",
                RuleSetId::Quake3,
                false,
                Some(view),
            )
            .unwrap();
        skins.push((skin, expected));
        let handle = owner
            .preload_shader(
                &files.vfs,
                Some(view),
                b"textures/body",
                RuleSetId::Quake3,
                true,
                ShaderLighting::TwoDimensional,
            )
            .unwrap();
        shaders.push((handle, expected));
    }
    let default_shader = owner
        .register_shader(
            &files.vfs,
            b"textures/body",
            RuleSetId::Quake3,
            true,
            ShaderLighting::TwoDimensional,
        )
        .unwrap();
    assert_ne!(shaders[0].0, shaders[1].0);
    assert!(shaders.iter().all(|&(handle, _)| handle != default_shader));
    shaders.push((default_shader, [0, 0, 255, 255]));
    assert_eq!(owner.catalog.profiles().collect::<Vec<_>>(), [profile]);
    let counts = owner.assets.counts();
    files.vfs.take_lookup_count();
    for request in requests {
        owner.register_request(&files.vfs, request).unwrap();
    }
    for (view, &(handle, _)) in [&low, &high].into_iter().zip(&shaders) {
        assert_eq!(
            owner
                .preload_shader(
                    &files.vfs,
                    Some(view),
                    b"textures/body",
                    RuleSetId::Quake3,
                    true,
                    ShaderLighting::TwoDimensional,
                )
                .unwrap(),
            handle
        );
    }
    for (view, &(handle, _)) in [&low, &high].into_iter().zip(&skins) {
        assert_eq!(
            owner
                .preload_skin(
                    &files.vfs,
                    b"models/player.skin",
                    RuleSetId::Quake3,
                    false,
                    Some(view)
                )
                .unwrap(),
            handle
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(owner.assets.counts(), counts);
    let foreign = Vfs::default().default_view();
    assert_eq!(
        owner.preload_shader(
            &files.vfs,
            Some(&foreign),
            b"textures/body",
            RuleSetId::Quake3,
            true,
            ShaderLighting::TwoDimensional,
        ),
        Err(CallError::File)
    );
    records.push((ModelFileId(default_model as u32), [0, 0, 255, 255]));
    assert!(
        owner
            .register_request(
                &files.vfs,
                ModelRequest {
                    search: Some(&foreign),
                    ..request
                }
            )
            .is_err()
    );
    assert_eq!(owner.assets.counts(), counts);
    let (mut owner, mut cpu) = super::registration_tests::publish(owner);
    let equivalent_low = files.vfs.search_view([low_mount, low_mount]).unwrap();
    for (view, (file, _)) in [&equivalent_low, &high].into_iter().zip(&records) {
        assert_eq!(
            owner
                .catalog
                .registration(request.path, profile, Some(view)),
            owner.catalog.renderer_registration(file.0 as i32, profile)
        );
        assert!(
            owner
                .catalog
                .metadata(request.path.as_bytes(), Some(view))
                .is_some()
        );
    }
    assert_eq!(
        owner.catalog.registration(request.path, profile, None),
        owner.catalog.renderer_registration(default_model, profile)
    );
    assert!(
        owner
            .catalog
            .registration(request.path, profile, Some(&foreign))
            .is_none()
    );
    assert!(
        owner
            .catalog
            .metadata(request.path.as_bytes(), Some(&foreign))
            .is_none()
    );
    for &(file, expected) in &records {
        let registration = owner
            .catalog
            .renderer_registration(file.0 as i32, profile)
            .unwrap();
        let binding = owner
            .catalog
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next()
            .unwrap();
        let pixels =
            super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding.shader);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected)),
            "selected model source image {expected:?}"
        );
    }
    let shader = owner
        .catalog
        .shader_binding(default_shader, profile)
        .unwrap();
    let pixels = super::registration_tests::paint_shader(&mut cpu, owner.assets(), shader);
    assert!(
        pixels
            .iter()
            .all(|pixel| *pixel == u32::from_le_bytes([0, 0, 255, 255]))
    );
    for &(handle, expected) in &shaders {
        let binding = owner.catalog.shader_binding(handle, profile).unwrap();
        let pixels = super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected))
        );
    }
    for &(skin, expected) in &skins {
        let registration = owner
            .catalog
            .renderer_registration(records[0].0.0 as i32, profile)
            .unwrap();
        let binding = owner
            .catalog
            .bindings(
                registration,
                0,
                ModelMaterialPolicy {
                    custom_skin: skin,
                    ..Default::default()
                },
            )
            .unwrap()
            .next()
            .unwrap();
        let pixels =
            super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding.shader);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected))
        );
    }
    let mut settings = [None; 5];
    settings[RuleSetId::Quake3 as usize] = Some(ImageSettings {
        picmip: 0,
        ..profile.image_settings
    });
    let next = ModelProfile {
        image_settings: settings[RuleSetId::Quake3 as usize].unwrap(),
        ..profile
    };
    owner
        .prepare_profile_value(next, |assets| {
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    let retained = owner.assets().counts();
    let policies = owner.catalog.profiles.len();
    let rejected = owner.prepare_targets(
        &files.vfs,
        [RegistrationTarget::retained(
            records[0].0,
            profile.source_rules,
        )],
        next,
        RegistrationVariant::Guard(false),
        |_| Ok(()),
        |_, ()| Err("rejected selected-source admission".into()),
    );
    assert_eq!(rejected.unwrap_err(), "rejected selected-source admission");
    assert_eq!(owner.assets().counts(), retained);
    assert_eq!(owner.catalog.profiles.len(), policies);
    let rejected: Result<(), String> = owner.image_transaction(
        &files.vfs,
        None,
        ImageRefreshMode::Reconfigure,
        |owner, _| {
            owner.refresh_images(
                &files.vfs,
                &[profile],
                &[],
                settings,
                profile.finish,
                false,
                |_| Ok(()),
            )?;
            Err("rejected selected-source refresh".into())
        },
    );
    assert_eq!(rejected.unwrap_err(), "rejected selected-source refresh");
    assert_eq!(owner.assets().counts(), retained);
    assert_eq!(owner.catalog.profiles.len(), policies);
    let refresh = owner
        .image_transaction(
            &files.vfs,
            None,
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                owner.refresh_images(
                    &files.vfs,
                    &[profile],
                    &[],
                    settings,
                    profile.finish,
                    false,
                    |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
            },
        )
        .unwrap();
    let selected = refresh.profile(profile);
    assert_ne!(selected, profile);
    for &(file, expected) in &records {
        let original = owner
            .catalog
            .renderer_registration(file.0 as i32, profile)
            .unwrap();
        let registration = refresh.registration(original, owner.catalog()).unwrap();
        assert_eq!(
            owner.catalog.registrations[registration.0 as usize].file,
            file
        );
        let binding = owner
            .catalog
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next()
            .unwrap();
        let pixels =
            super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding.shader);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected))
        );
    }
    owner
        .image_transaction(&files.vfs, None, ImageRefreshMode::Reload, |owner, _| {
            owner.refresh_images(
                &files.vfs,
                &[selected],
                &[],
                [None; 5],
                selected.finish,
                false,
                |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
        })
        .unwrap();
    for &(handle, expected) in &shaders {
        let binding = owner.catalog.shader_binding(handle, selected).unwrap();
        let pixels = super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected))
        );
    }
    for &(skin, expected) in &skins {
        let registration = owner
            .catalog
            .renderer_registration(records[0].0.0 as i32, selected)
            .unwrap();
        let binding = owner
            .catalog
            .bindings(
                registration,
                0,
                ModelMaterialPolicy {
                    custom_skin: skin,
                    ..Default::default()
                },
            )
            .unwrap()
            .next()
            .unwrap();
        let pixels =
            super::registration_tests::paint_shader(&mut cpu, owner.assets(), binding.shader);
        assert!(
            pixels
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(expected))
        );
    }
}

#[test]
fn indexed_skins_and_model_colors_keep_the_selected_file_source() {
    let mut low_model = support::mdl();
    low_model[88..90].fill(16); // The authored two-pixel skin follows the 84-byte header and skin type.
    let mut high_model = low_model.clone();
    high_model[88..90].fill(96);
    let mut files = support::Fixture::new(&[
        ("progs/player.mdl", low_model),
        ("skins/player.pcx", support::pcx_dimensions(2, 1, 16)),
    ]);
    let high_files = support::Fixture::new(&[
        ("progs/player.mdl", high_model),
        ("skins/player.pcx", support::pcx_dimensions(2, 1, 96)),
    ]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high_files.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("progs/player.mdl", RuleSetId::Quake, Some(palette));
    let profile = request.profile();
    let mut owner = ModelAssets::load(
        &files.vfs,
        assets,
        [&low, &high].map(|view| ModelRequest {
            search: Some(view),
            ..request
        }),
        [],
        Default::default(),
    )
    .unwrap();
    let first_index = |owner: &ModelAssets, shader| {
        let material = owner.assets().shader_binding(shader).unwrap().material;
        let StageTexture::Image(image) =
            owner.assets().material(material).unwrap().stages[0].texture
        else {
            panic!("indexed source image");
        };
        owner
            .assets()
            .image(image)
            .unwrap()
            .indexed
            .as_ref()
            .unwrap()
            .mip(0)
            .unwrap()
            .indices()[0]
    };
    let colored_index = |owner: &ModelAssets, registration, colored| {
        let choices = owner
            .catalog
            .lod_bindings(
                registration,
                0,
                ModelMaterialPolicy {
                    fullbright: true,
                    ..Default::default()
                },
            )
            .unwrap()
            .with_skin(owner.catalog(), colored)
            .unwrap();
        let group = choices.base().unwrap().choices().next().unwrap();
        let shader = group.shaders().next().unwrap();
        let binding = group
            .resolve_shader(
                shader,
                qa_render::shader::reference::ShaderView::direct(),
                None,
            )
            .unwrap()
            .source(&())
            .unwrap();
        first_index(owner, binding)
    };
    let (initial, mut cpu) = super::registration_tests::publish(owner);
    owner = initial;
    let mut retained = Vec::new();
    for (view, expected) in [(&low, 16), (&high, 96)] {
        let registration = owner
            .catalog
            .registration(request.path, profile, Some(view))
            .unwrap();
        let colors = ModelSkinColors {
            indexed: [1, 6],
            rgba: [1, 6],
            upload_extent: None,
            rgba_crop: None,
        };
        let colored = owner
            .prepare_model_colors(&files.vfs, registration, colors, false, |assets, _| {
                cpu.prepare_assets(assets, None).map_err(str::to_owned)
            })
            .unwrap();
        assert_eq!(colored_index(&owner, registration, colored), expected);
        let translation = IndexedSkinTranslation {
            palette,
            target: Some(palette),
            indexed_colors: None,
            rgba_colors: None,
            crop: None,
            coordinate_extent: [2, 1],
            upload_extent: None,
            usage: ImageUse {
                role: ImageRole::Skin,
                allow_picmip: false,
                sampler: qa_render::assets::Sampler {
                    mipmaps: false,
                    ..Default::default()
                },
            },
        };
        let indexed = owner
            .prepare_indexed_skin(
                &files.vfs,
                "skins/player.pcx",
                profile,
                false,
                IndexedSkinLayout {
                    storage: Some([2, 1]),
                    padding: 0,
                    omit_last_row: false,
                },
                translation,
                Some(view),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        let binding = owner
            .catalog
            .indexed_skin(indexed, Default::default())
            .unwrap()
            .source(&())
            .unwrap();
        assert_eq!(first_index(&owner, binding), expected);
        retained.push((registration, colors, colored, indexed, translation));
    }
    assert_ne!(retained[0].2, retained[1].2);
    assert_ne!(retained[0].3, retained[1].3);
    files.vfs.take_lookup_count();
    for (view, &(registration, colors, colored, indexed, translation)) in
        [&low, &high].into_iter().zip(&retained)
    {
        assert_eq!(
            owner
                .prepare_model_colors(&files.vfs, registration, colors, false, |_, _| panic!(
                    "cached model colors prepared twice"
                ))
                .unwrap(),
            colored
        );
        assert_eq!(
            owner
                .prepare_indexed_skin(
                    &files.vfs,
                    "skins/player.pcx",
                    profile,
                    false,
                    IndexedSkinLayout {
                        storage: Some([2, 1]),
                        padding: 0,
                        omit_last_row: false
                    },
                    translation,
                    Some(view),
                    |_, _| panic!("cached indexed skin prepared twice")
                )
                .unwrap(),
            indexed
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    let mut current = profile;
    for mode in [ImageRefreshMode::Reconfigure, ImageRefreshMode::Reload] {
        let mut settings = [None; 5];
        if mode == ImageRefreshMode::Reconfigure {
            settings[current.rules as usize] = Some(ImageSettings {
                picmip: 1,
                ..current.image_settings
            });
        }
        let refresh = owner
            .image_transaction(&files.vfs, None, mode, |owner, _| {
                owner.refresh_images(
                    &files.vfs,
                    &[current],
                    &[],
                    settings,
                    current.finish,
                    false,
                    |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
            })
            .unwrap();
        current = refresh.profile(current);
        for (index, row) in retained.iter_mut().enumerate() {
            row.0 = refresh.registration(row.0, owner.catalog()).unwrap();
            row.2 = owner.catalog.refresh_skin(row.2, &refresh).unwrap();
            row.3 = owner.catalog.refresh_skin(row.3, &refresh).unwrap();
            let expected = [16, 96][index];
            assert_eq!(colored_index(&owner, row.0, row.2), expected);
            let binding = owner
                .catalog
                .indexed_skin(row.3, Default::default())
                .unwrap()
                .source(&())
                .unwrap();
            assert_eq!(first_index(&owner, binding), expected);
        }
    }
}

#[test]
fn palette_registration_keeps_source_aliases_and_rollback_in_the_owned_vfs() {
    use crate::render_settings::{PaletteRegistrations, palette_resource};
    let mut low_bytes = support::pcx_dimensions(256, 320, 16);
    let mut high_bytes = low_bytes.clone();
    let first = low_bytes.len() - 768 + 16 * 3;
    low_bytes[first..first + 3].copy_from_slice(&[201, 1, 2]);
    high_bytes[first..first + 3].copy_from_slice(&[3, 5, 207]);
    let mut files = support::Fixture::new(&[("pics/colormap.pcx", low_bytes)]);
    let high_files = support::Fixture::new(&[("pics/colormap.pcx", high_bytes)]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high_files.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let mut registered = PaletteRegistrations::new(&files.vfs);
    let mut assets = Assets::load().unwrap();
    let low_id = palette_resource(
        files.vfs.read_view(Some(&low)),
        &mut assets,
        RuleSetId::Quake2,
        &mut registered,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        assets.palette(low_id).unwrap().color(16).to_le_bytes(),
        [201, 1, 2, 255]
    );
    let prefix = assets.counts();
    let checkpoint = registered.checkpoint();
    let high_id = palette_resource(
        files.vfs.read_view(Some(&high)),
        &mut assets,
        RuleSetId::Quake2Rerelease,
        &mut registered,
    )
    .unwrap()
    .unwrap();
    assert_ne!(low_id, high_id);
    assert_eq!(
        assets.palette(high_id).unwrap().color(16).to_le_bytes(),
        [3, 5, 207, 255]
    );
    registered.restore(checkpoint);
    assets.discard_registration_suffix(prefix).unwrap();
    assert_eq!(registered.get(RuleSetId::Quake2, Some(&low)), Some(low_id));
    assert_eq!(registered.get(RuleSetId::Quake2, Some(&high)), None);
    let high_id = palette_resource(
        files.vfs.read_view(Some(&high)),
        &mut assets,
        RuleSetId::Quake2,
        &mut registered,
    )
    .unwrap()
    .unwrap();
    let default_id = palette_resource(
        files.vfs.read_view(None),
        &mut assets,
        RuleSetId::Quake2,
        &mut registered,
    )
    .unwrap()
    .unwrap();
    assert_ne!(high_id, default_id);
    assert_eq!(
        assets.palette(default_id).unwrap().color(16).to_le_bytes(),
        [3, 5, 207, 255]
    );
    let equivalent = files.vfs.search_view([low_mount, low_mount]).unwrap();
    let counts = assets.counts();
    files.vfs.take_lookup_count();
    for (view, expected) in [(&equivalent, low_id), (&high, high_id)] {
        assert_eq!(
            palette_resource(
                files.vfs.read_view(Some(view)),
                &mut assets,
                RuleSetId::Quake2Rerelease,
                &mut registered
            )
            .unwrap(),
            Some(expected)
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(assets.counts(), counts);
    let foreign = Vfs::default();
    for read in [
        foreign.read_view(None),
        files.vfs.read_view(Some(&foreign.default_view())),
    ] {
        assert!(matches!(
            palette_resource(read, &mut assets, RuleSetId::Quake2, &mut registered),
            Err(ResourceError::Vfs(qa_content::vfs::VfsError::Handle))
        ));
    }
    assert_eq!(assets.counts(), counts);
    let mut owner = ModelAssets::new(&files.vfs, assets, [], [], Default::default()).unwrap();
    assert!(
        owner
            .adopt_palettes(PaletteRegistrations::from_registered(
                &foreign,
                [Some(low_id); 5]
            ))
            .is_err()
    );
    owner.adopt_palettes(registered).unwrap();
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake2, Some(&low)),
        Some(low_id)
    );
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake2, None),
        Some(default_id)
    );
    let (mut owner, mut cpu) = super::registration_tests::publish(owner);
    for (view, expected) in [(&low, low_id), (&high, high_id)] {
        let (profile, _) = owner
            .prepare_profile(
                &files.vfs,
                RuleSetId::Quake2,
                RuleSetId::Quake2,
                ImageSettings::native(RuleSetId::Quake2),
                MaterialFinishPolicy::default(),
                Some(view),
                |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        assert_eq!(profile.source_palette, Some(expected));
        assert_eq!(profile.palette, Some(expected));
    }
    let extra = files.vfs.search_view([low_mount, high_mount]).unwrap();
    let prefix = owner.assets().counts();
    let rejected = owner.prepare_profile(
        &files.vfs,
        RuleSetId::Quake2,
        RuleSetId::Quake2,
        ImageSettings::native(RuleSetId::Quake2),
        MaterialFinishPolicy::default(),
        Some(&extra),
        |_| Err("reject selected palette profile".into()),
    );
    assert_eq!(rejected.unwrap_err(), "reject selected palette profile");
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(owner.palettes().get(RuleSetId::Quake2, Some(&extra)), None);
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake2, Some(&low)),
        Some(low_id)
    );
}

#[test]
fn world_presentations_keep_their_source_through_owner_refresh_and_rejection() {
    let mut files = support::Fixture::new(&[
        ("maps/selected.bsp", support::world_map(8.0)),
        (
            "scripts/source.shader",
            b"textures/wall { cull none { map textures/low.tga } }".to_vec(),
        ),
        ("textures/low.tga", support::tga(1, 1, &[[211, 0, 0, 255]])),
    ]);
    let high_files = support::Fixture::new(&[
        (
            "scripts/source.shader",
            b"textures/wall { cull none { map textures/high.tga } }".to_vec(),
        ),
        ("textures/high.tga", support::tga(1, 1, &[[0, 0, 211, 255]])),
    ]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high_files.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let mut profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let presentation = qa_render::material::world_load::WorldPresentationProfile::native(
        profile.rules,
        profile.palette,
    );
    let mut geometry = qa_world::collision::CollisionStore::new();
    let loaded = crate::map::read(files.vfs.read_view(None), "selected")
        .unwrap()
        .load(
            &files.vfs,
            &mut assets,
            &mut geometry,
            Some(palette),
            &[qa_render::material::world_load::WorldPresentationRequest {
                profile: presentation,
                native_default: None,
                search: Some(&low),
            }],
            Default::default(),
            false,
        )
        .unwrap();
    let world = loaded.render.world;
    let mesh = assets.world(world).unwrap().mesh();
    let first = loaded.render.presentations[0];
    let owner =
        ModelAssets::new(&files.vfs, assets, [profile], [world], Default::default()).unwrap();
    let (mut owner, mut cpu) = super::registration_tests::publish(owner);
    assert!(
        owner
            .view()
            .world_presentation(world, Some(profile), None, None)
            .is_none()
    );
    assert_eq!(
        owner
            .view()
            .world_presentation(world, None, Some(first.id), None),
        Some(first)
    );
    let color = |assets: &Assets, id| {
        let shader = assets
            .shader_binding(assets.surface_shader(id, 0).unwrap())
            .unwrap();
        let material = assets.material(shader.material).unwrap();
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("world image")
        };
        assets.image(image).unwrap().rgba[..4].to_vec()
    };
    assert_eq!(color(owner.assets(), first.id), [211, 0, 0, 255]);
    let before = owner.assets().counts();
    assert!(
        owner
            .prepare_world_presentation(
                &files.vfs,
                world,
                profile,
                presentation,
                false,
                Some(&high),
                |_, _| Ok(()),
                |_, ()| Err("rejected selected world".to_owned())
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(
        owner.assets().world(world).unwrap().presentations(),
        &[first.id]
    );
    assert_eq!(color(owner.assets(), first.id), [211, 0, 0, 255]);
    let (second, (), _) = owner
        .prepare_world_presentation(
            &files.vfs,
            world,
            profile,
            presentation,
            false,
            Some(&high),
            |_, _| Ok(()),
            |assets, ()| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let mut rows = [first, second.presentations[0]];
    assert_eq!(
        owner
            .view()
            .world_presentation(world, Some(profile), None, Some(&low)),
        Some(rows[0])
    );
    assert_eq!(
        owner
            .view()
            .world_presentation(world, Some(profile), None, Some(&high)),
        Some(rows[1])
    );
    assert!(
        owner
            .view()
            .world_presentation(world, None, Some(rows[0].id), Some(&high))
            .is_none()
    );
    assert_eq!(
        owner.renderer_presentation(world, profile, Some(rows[1].id), Some(&low)),
        Some(rows[0])
    );
    assert_eq!(color(owner.assets(), rows[1].id), [0, 0, 211, 255]);
    for mode in [ImageRefreshMode::Reconfigure, ImageRefreshMode::Reload] {
        let mut settings = [None; 5];
        if mode == ImageRefreshMode::Reconfigure {
            settings[profile.rules as usize] = Some(ImageSettings {
                picmip: 1,
                ..profile.image_settings
            });
        }
        let refresh = owner
            .image_transaction(&files.vfs, None, mode, |owner, _| {
                owner.refresh_images(
                    &files.vfs,
                    &[profile],
                    &rows.map(|row| (world, row.id)),
                    settings,
                    profile.finish,
                    false,
                    |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
            })
            .unwrap();
        profile = refresh.profile(profile);
        for (index, (row, search)) in rows.iter_mut().zip([&low, &high]).enumerate() {
            *row = refresh.presentation(world, row.id).unwrap();
            assert_eq!(
                owner
                    .view()
                    .world_presentation(world, Some(profile), None, Some(search)),
                Some(*row)
            );
            assert_eq!(
                color(owner.assets(), row.id),
                [[211, 0, 0, 255], [0, 0, 211, 255]][index]
            );
            assert!(
                owner
                    .assets()
                    .world_presentation(row.id)
                    .unwrap()
                    .search()
                    .unwrap()
                    .same_selection(search)
            );
        }
        assert_eq!(owner.assets().world(world).unwrap().mesh(), mesh);
        assert!(
            owner
                .view()
                .world_presentation(world, Some(profile), None, None)
                .is_none()
        );
    }
}

#[test]
fn selected_maps_keep_geometry_palette_and_loaded_world_name_ownership() {
    let entries = |size, color: [u8; 3]| {
        let mut palette = (0..=255u8).flat_map(|v| [v; 3]).collect::<Vec<_>>();
        palette[33..36].copy_from_slice(&color);
        let mut shades = (0..64).flat_map(|_| 0..=255u8).collect::<Vec<_>>();
        shades.push(32);
        vec![
            ("maps/selected.bsp", support::world_map(size)),
            ("gfx/palette.lmp", palette),
            ("gfx/colormap.lmp", shades),
        ]
    };
    let mut files = support::Fixture::new(&entries(8., [211, 0, 0]));
    let high_files = support::Fixture::new(&entries(16., [0, 0, 211]));
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high_files.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let (mut owner, mut cpu) = super::registration_tests::publish(owner);
    let mut geometry = qa_world::collision::CollisionStore::new();
    let mut file_ids = Vec::new();
    let mut worlds = Vec::new();
    for (search, material_search, size, expected) in [
        (&low, &high, 8., [211, 0, 0]),
        (&high, &low, 16., [0, 0, 211]),
    ] {
        let read = || crate::map::read(files.vfs.read_view(Some(search)), "selected").unwrap();
        let input = read();
        assert!(input.search().same_selection(search));
        let checksum = input.native_checksums();
        let registration = || WorldRegistration {
            search: Some(material_search.clone()),
            profile,
            presentation: qa_render::material::world_load::WorldPresentationProfile::native(
                profile.rules,
                profile.palette,
            ),
            options: Default::default(),
            ui_fullscreen: false,
            trace_rules: qa_world::collision::TraceRules::ARENA,
        };
        let prefix = owner.assets().counts();
        let geometries = geometry.geometries().collect::<Vec<_>>();
        assert!(
            owner
                .prepare_world(
                    &files.vfs,
                    &mut geometry,
                    read(),
                    registration(),
                    |_, _, _| Ok(()),
                    |_, ()| Err("reject selected map".to_owned())
                )
                .is_err()
        );
        assert_eq!(owner.assets().counts(), prefix);
        assert_eq!(geometry.geometries().collect::<Vec<_>>(), geometries);
        assert!(
            owner
                .published_palette(RuleSetId::Quake, Some(search))
                .is_none()
        );
        let (loaded, (), _) = owner
            .prepare_world(
                &files.vfs,
                &mut geometry,
                input,
                registration(),
                |_, _, _| Ok(()),
                |assets, ()| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        assert!(loaded.search.same_selection(search));
        let palette = owner
            .published_palette(RuleSetId::Quake, Some(search))
            .unwrap();
        assert_eq!(
            &owner
                .assets()
                .palette(palette)
                .unwrap()
                .color(11)
                .to_le_bytes()[..3],
            &expected
        );
        let world = owner.assets().world(loaded.render.world).unwrap();
        assert_eq!(
            world
                .geometry()
                .vertices
                .iter()
                .map(|v| v.vertex.position.0[1].abs())
                .fold(0., f32::max),
            size
        );
        assert!(
            owner
                .assets()
                .world_presentation(loaded.render.presentations[0].id)
                .unwrap()
                .search()
                .unwrap()
                .same_selection(material_search)
        );
        let file = owner
            .catalog
            .file_id(loaded.virtual_path.as_bytes(), Some(search))
            .unwrap();
        let ModelFile::LoadedWorld {
            binding,
            collision,
            native_checksums,
            ..
        } = owner.catalog.files.get(file).unwrap()
        else {
            panic!("loaded world")
        };
        assert_eq!(binding.world, loaded.render.world);
        assert_eq!(*collision, Some(loaded.collision));
        assert_eq!(*native_checksums, Some(checksum));
        let named = owner
            .catalog()
            .loaded_world(loaded.virtual_path.as_bytes(), Some(search))
            .unwrap();
        assert_eq!(named.render, loaded.render.world);
        assert_eq!(named.collision, Some(loaded.collision));
        file_ids.push(file);
        worlds.push(loaded.render.world);
    }
    assert_ne!(file_ids[0], file_ids[1]);
    assert_ne!(worlds[0], worlds[1]);
    assert!(owner.catalog.file_id(b"maps/selected.bsp", None).is_none());
    let foreign = Vfs::default();
    assert!(crate::map::read(foreign.read_view(Some(&low)), "selected").is_err());
    let foreign_view = foreign.default_view();
    assert!(
        owner
            .catalog()
            .loaded_world(b"maps/selected.bsp", Some(&foreign_view))
            .is_none()
    );
    assert!(
        owner
            .catalog()
            .loaded_world(b"maps/selected.bsp", None)
            .is_none()
    );
    assert!(crate::map::read(files.vfs.read_view(Some(&foreign_view)), "selected").is_err());
    let mut assets = Assets::load().unwrap();
    let prefix = assets.counts();
    let mut empty = qa_world::collision::CollisionStore::new();
    assert!(
        crate::map::read(files.vfs.read_view(Some(&low)), "selected")
            .unwrap()
            .load(
                &foreign,
                &mut assets,
                &mut empty,
                None,
                &[],
                Default::default(),
                false
            )
            .is_err()
    );
    assert_eq!(assets.counts(), prefix);
    assert_eq!(empty.geometries().count(), 0);
}
