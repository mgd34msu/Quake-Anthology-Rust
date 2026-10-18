use super::*;
use qa_app::model_assets::RegistrationVariant;
use qa_compat::services::RendererRegistration;
use qa_render::cpu::CpuBackend;

fn family_files() -> Vec<(&'static str, Vec<u8>)> {
    let mut base = support::md3_named_surfaces(&[
        (b"BODY_1", &[b"textures/base", b"textures/base_alt"]),
        (b"trim_1", &[b"textures/trim"]),
    ]);
    let frames = i32::from_le_bytes(base[92..96].try_into().unwrap()) as usize;
    for (index, value) in [-3.0f32, -4.0, -12.0, 3.0, 4.0, 12.0]
        .into_iter()
        .enumerate()
    {
        base[frames + index * 4..frames + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    // R_ComputeLOD derives its radius from the selected base frame's bounds.
    base[frames + 36..frames + 40].copy_from_slice(&9.0f32.to_le_bytes());
    let middle = support::md3_named_surfaces(&[(b"middle_1", &[b"textures/middle"])]);
    let mut coarse = support::md3_named_surfaces(&[
        (b"BODY_1", &[b"textures/low", b"textures/low_alt"]),
        (b"cap_1", &[b"textures/cap"]),
        (b"tail_1", &[b"textures/tail"]),
    ]);
    let tags = i32::from_le_bytes(coarse[96..100].try_into().unwrap()) as usize;
    for frame in 0..2 {
        let first = tags + frame * 112;
        coarse[first..first + 64].fill(0);
        coarse[first..first + 7].copy_from_slice(b"tag_lod");
    }
    let mut files = vec![
        ("models/family.md3", base),
        ("models/family_1.md3", middle),
        ("models/family_2.md3", coarse),
        (
            "models/family.skin",
            b"body,textures/skin\ntrim,textures/trim\nmiddle,textures/middle\ncap,textures/cap\ntail,textures/tail\n".to_vec(),
        ),
    ];
    for name in [
        "textures/base.tga",
        "textures/base_alt.tga",
        "textures/trim.tga",
        "textures/middle.tga",
        "textures/low.tga",
        "textures/low_alt.tga",
        "textures/cap.tga",
        "textures/tail.tga",
        "textures/skin.tga",
    ] {
        files.push((name, support::tga(8, 8, &[[91, 37, 13, 255]; 64])));
    }
    files
}

#[test]
fn md3_lod_family_keeps_native_discovery_order_and_independent_surface_skin_lists() {
    let files = family_files();
    let parsed = qa_formats::model::Model::parse(&files[0].1).unwrap();
    let fixture = support::Fixture::new(&files);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let handle = owner
        .register_model(&fixture.vfs, b"models/family.md3", profile.source_rules)
        .unwrap();
    assert!(handle > 0);
    let catalog = owner.catalog();
    assert_eq!(catalog.capacity().registrations, 1);
    for name in [b"models/family_1.md3".as_slice(), b"models/family_2.md3"] {
        assert_eq!(catalog.model_handle(name, profile.source_rules, None), None);
        assert_eq!(catalog.metadata(name, None), None);
    }
    let receipts = catalog
        .renderer_registrations(RendererRegistration::Model, handle, profile)
        .unwrap();
    assert_eq!(
        receipts
            .iter()
            .map(|entry| owner.assets().name(entry.key).unwrap())
            .collect::<Vec<_>>(),
        [
            "textures/low",
            "textures/low_alt",
            "textures/cap",
            "textures/tail",
            "textures/middle",
            "textures/base",
            "textures/base_alt",
            "textures/trim",
        ]
    );
    let registration = catalog.renderer_registration(handle, profile).unwrap();
    let lods = catalog
        .lod_bindings(registration, 0, Default::default())
        .unwrap();
    assert_eq!(lods.native_count(), 3);
    assert_eq!(lods.base_frames(), 2);
    assert_eq!(parsed.frames[0].radius, 9.0);
    assert_eq!(lods.radius(0), Some(13.0));
    assert_eq!(lods.radius(1), Some(parsed.frames[1].bounds.radius()));
    assert_eq!(lods.radius(2), None);
    assert!(lods.info().supports_lods());
    let rows = lods.collect::<Vec<_>>();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .map(|row| row.each_ref().map(Option::is_some))
            .collect::<Vec<_>>(),
        [
            [true, true, true],
            [true, false, true],
            [false, false, true]
        ]
    );
    let selected = catalog
        .lod_bindings(registration, 1, Default::default())
        .unwrap()
        .collect::<Vec<_>>();
    let shader = |row: usize, level: usize| {
        selected[row][level]
            .as_ref()
            .unwrap()
            .physical_binding()
            .unwrap()
            .shader
    };
    assert_eq!(shader(0, 0), receipts[6].shader);
    assert_eq!(shader(1, 0), receipts[7].shader);
    assert_eq!(shader(0, 1), receipts[4].shader);
    assert_eq!(shader(0, 2), receipts[1].shader);
    assert_eq!(shader(1, 2), receipts[2].shader);
    assert_eq!(shader(2, 2), receipts[3].shader);
    assert_eq!(catalog.model_bounds(handle), Some(parsed.frames[0].bounds));
    assert_eq!(
        catalog.metadata(b"models/family.md3", None).unwrap().bounds,
        parsed.bounds
    );
    assert_eq!(
        catalog.lerp_tag(handle, 0, 1, 0.25, b"tag_weapon"),
        Some(
            parsed.tags[0]
                .orientation
                .interpolate(parsed.tags[1].orientation, 0.25)
        )
    );
    assert_eq!(catalog.lerp_tag(handle, 0, 0, 0.0, b"tag_lod"), None);
    let base_models = rows[0]
        .each_ref()
        .map(|choice| choice.as_ref().unwrap().model);
    assert_ne!(base_models[0], base_models[1]);
    assert_ne!(base_models[1], base_models[2]);
    let skin = owner
        .register_skin(&fixture.vfs, b"models/family.skin", profile.source_rules)
        .unwrap();
    let skin_shader = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Skin, skin, profile)
        .unwrap()[0]
        .shader;
    let custom = owner
        .catalog()
        .lod_bindings(
            registration,
            0,
            ModelMaterialPolicy {
                custom_skin: skin,
                ..Default::default()
            },
        )
        .unwrap()
        .collect::<Vec<_>>();
    for level in [0, 2] {
        let choice = custom[0][level]
            .as_ref()
            .unwrap()
            .physical_binding()
            .unwrap();
        assert_eq!(choice.shader, skin_shader);
        assert_eq!(choice.model, base_models[level]);
    }
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner.register_model(&fixture.vfs, b"MODELS\\FAMILY.MD3", profile.source_rules),
        Ok(handle)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}

#[test]
fn md3_missing_lods_preserve_holes_and_known_invalid_lod_stops_before_base() {
    for invalid_middle in [false, true] {
        let mut files = family_files();
        if invalid_middle {
            files[1].1[4..8].copy_from_slice(&999i32.to_le_bytes());
        } else {
            files.remove(1);
        }
        let fixture = support::Fixture::new(&files);
        let profile = shader_profile(RuleSetId::Quake3);
        let mut owner = ModelAssets::new(
            &fixture.vfs,
            Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap();
        let handle = owner
            .register_model(&fixture.vfs, b"models/family.md3", profile.source_rules)
            .unwrap();
        let catalog = owner.catalog();
        let registration = catalog.renderer_registration(handle, profile).unwrap();
        let lods = catalog
            .lod_bindings(registration, 0, Default::default())
            .unwrap();
        assert_eq!(lods.native_count(), 2);
        if invalid_middle {
            assert!(lods.base().is_none());
            assert_eq!(lods.base_frames(), 0);
            assert_eq!(lods.radius(0), None);
            assert_eq!(catalog.metadata(b"models/family.md3", None), None);
            assert_eq!(catalog.model_bounds(handle), Some(Bounds::default()));
            assert!(!catalog.model_has_tags(handle));
        } else {
            assert_eq!(lods.base().unwrap().len(), 2);
            assert_eq!(lods.base_frames(), 2);
            assert_eq!(lods.radius(0), Some(13.0));
            assert!(catalog.model_has_tags(handle));
        }
        let rows = lods.collect::<Vec<_>>();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row[1].is_none() && row[2].is_some()));
        assert_eq!(rows[0][0].is_some(), !invalid_middle);
        let receipts = catalog
            .renderer_registrations(RendererRegistration::Model, handle, profile)
            .unwrap();
        assert_eq!(receipts.len(), if invalid_middle { 4 } else { 7 });
        fixture.vfs.take_lookup_count();
        assert_eq!(
            owner.register_model(&fixture.vfs, b"models/family.md3", profile.source_rules),
            Ok(handle)
        );
        assert_eq!(fixture.vfs.take_lookup_count(), 0);
    }
}

#[test]
fn md3_unknown_magic_and_invalid_base_fail_the_family_without_publishing_suffix_handles() {
    for (slot, known_magic) in [(2, true), (1, false), (0, true)] {
        let mut files = family_files();
        if known_magic {
            files[slot].1[4..8].copy_from_slice(&999i32.to_le_bytes());
        } else {
            files[slot].1[..4].copy_from_slice(b"NOPE");
        }
        let fixture = support::Fixture::new(&files);
        let profile = shader_profile(RuleSetId::Quake3);
        let mut owner = ModelAssets::new(
            &fixture.vfs,
            Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            owner.register_model(&fixture.vfs, b"models/family.md3", profile.source_rules),
            Err(qa_compat::services::CallError::File)
        );
        assert_eq!(
            owner
                .catalog()
                .model_handle(b"models/family.md3", profile.source_rules, None),
            None
        );
        for suffix in [b"models/family_1.md3".as_slice(), b"models/family_2.md3"] {
            assert_eq!(
                owner
                    .catalog()
                    .model_handle(suffix, profile.source_rules, None),
                None
            );
        }
        fixture.vfs.take_lookup_count();
        assert_eq!(
            owner.register_model(&fixture.vfs, b"models/family.md3", profile.source_rules),
            Ok(0)
        );
        assert_eq!(fixture.vfs.take_lookup_count(), 0);
    }
}

#[test]
fn md3_profile_rebuild_retains_lod_sources_and_rolls_back_backend_rejection() {
    let fixture = support::Fixture::new(&family_files());
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let handle = owner
        .register_model(&fixture.vfs, b"models/family.md3", profile.source_rules)
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let original = owner
        .catalog()
        .lod_bindings(registration, 0, Default::default())
        .unwrap()
        .map(|row| row.map(|choice| choice.map(|choice| choice.physical_binding().unwrap())))
        .collect::<Vec<_>>();
    let bounds = owner.catalog().model_bounds(handle);
    let tag = owner.catalog().lerp_tag(handle, 0, 1, 0.5, b"tag_weapon");
    let mut cpu = CpuBackend::load_with_assets(8, 8, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    for path in [
        "models/family.md3",
        "models/family_1.md3",
        "models/family_2.md3",
    ] {
        fixture.remove(path);
    }
    let mut settings = profile.image_settings;
    settings.picmip += 1;
    let (new_profile, _) = owner
        .prepare_profile(
            &fixture.vfs,
            profile.source_rules,
            profile.rules,
            settings,
            profile.finish,
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = renderer_request(
        "models/family.md3",
        RendererRegistration::Model,
        profile.source_rules,
        None,
    );
    let before = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                new_profile,
                RegistrationVariant::Guard(false),
                |candidate| {
                    assert_eq!(candidate.registrations.len(), 8);
                    Ok(())
                },
                |_, _| Err("reject rebuilt LOD materials".to_owned())
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert!(
        owner
            .catalog()
            .renderer_registration(handle, new_profile)
            .is_none()
    );
    assert_eq!(
        owner.catalog().renderer_registration(handle, profile),
        Some(registration)
    );
    let (new_handle, _, counts) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            new_profile,
            RegistrationVariant::Guard(false),
            |candidate| {
                assert_eq!(candidate.registrations.len(), 8);
                Ok(())
            },
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(new_handle, handle);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    let new_registration = owner
        .catalog()
        .renderer_registration(handle, new_profile)
        .unwrap();
    let new_lods = owner
        .catalog()
        .lod_bindings(new_registration, 0, Default::default())
        .unwrap();
    assert_eq!(new_lods.native_count(), 3);
    assert_eq!(new_lods.base_frames(), 2);
    let rebuilt = new_lods
        .map(|row| row.map(|choice| choice.map(|choice| choice.physical_binding().unwrap())))
        .collect::<Vec<_>>();
    for (old_row, new_row) in original.iter().zip(&rebuilt) {
        for (old, new) in old_row.iter().zip(new_row) {
            assert_eq!(old.is_some(), new.is_some());
            if let (Some(old), Some(new)) = (old, new) {
                assert_eq!(old.model, new.model);
                assert_ne!(old.shader, new.shader);
            }
        }
    }
    assert_eq!(owner.catalog().model_bounds(handle), bounds);
    assert_eq!(
        owner.catalog().lerp_tag(handle, 0, 1, 0.5, b"tag_weapon"),
        tag
    );
    let old_again = owner
        .catalog()
        .lod_bindings(registration, 0, Default::default())
        .unwrap()
        .map(|row| row.map(|choice| choice.map(|choice| choice.physical_binding().unwrap())))
        .collect::<Vec<_>>();
    assert_eq!(old_again, original);
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                new_profile,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("retained LOD profile must not prepare backend")
            )
            .unwrap()
            .0,
        handle
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}
