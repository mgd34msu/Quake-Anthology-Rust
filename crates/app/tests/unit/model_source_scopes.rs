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
    let mut owner = ModelAssets::new(Assets::load().unwrap(), [], [], Default::default()).unwrap();
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
    let mut owner = ModelAssets::new(Assets::load().unwrap(), [], [], Default::default()).unwrap();
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
    let mut owner =
        ModelAssets::new(Assets::load().unwrap(), [profile], [], Default::default()).unwrap();
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
