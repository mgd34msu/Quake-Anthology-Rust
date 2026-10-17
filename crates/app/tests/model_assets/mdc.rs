use super::*;
use qa_app::model_assets::RegistrationVariant;
use qa_compat::services::RendererRegistration;
use qa_render::{Command, FrontEnd, Refdef, SceneEntity, cpu::CpuBackend};

fn mdc(shader: &[u8]) -> Vec<u8> {
    let mut bytes = include_bytes!("../../../formats/tests/fixtures/models/mesh.mdc").to_vec();
    let surface = i32::from_le_bytes(bytes[104..108].try_into().unwrap()) as usize;
    bytes[surface + 4..surface + 68].fill(0);
    bytes[surface + 4..surface + 10].copy_from_slice(b"BODY_1");
    let shaders = surface
        + i32::from_le_bytes(bytes[surface + 96..surface + 100].try_into().unwrap()) as usize;
    bytes[shaders..shaders + 64].fill(0);
    bytes[shaders..shaders + shader.len()].copy_from_slice(shader);
    bytes
}

fn resources() -> Vec<(&'static str, Vec<u8>)> {
    let mut files = vec![
        (
            "scripts/mdc.shader",
            br#"
            textures/base { cull none { map textures/base.tga rgbGen identity } }
            textures/alternate { cull none { map textures/alternate.tga rgbGen identity } }
            textures/coarse { cull none { map textures/coarse.tga rgbGen identity } }
            textures/skin { cull none { map textures/skin.tga rgbGen identity } }
        "#
            .to_vec(),
        ),
        ("models/packed.skin", b"body,textures/skin\n".to_vec()),
    ];
    for (path, color) in [
        ("textures/base.tga", [211, 17, 29, 255]),
        ("textures/alternate.tga", [31, 223, 47, 255]),
        ("textures/coarse.tga", [53, 67, 229, 255]),
        ("textures/skin.tga", [191, 113, 71, 255]),
    ] {
        files.push((path, support::tga(8, 8, &[color; 64])));
    }
    files
}

fn render(
    cpu: &mut CpuBackend,
    assets: &Assets,
    binding: qa_app::model_assets::ModelBinding,
) -> Vec<u32> {
    let mut frontend = FrontEnd::load(Default::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: binding.model.into(),
        shader: Some(binding.shader.into()),
        frame: 1,
        old_frame: 1,
        ..Default::default()
    }));
    assert!(frame.render_scene(
        Refdef {
            viewport: qa_render::Viewport {
                width: 64,
                height: 64,
                ..Default::default()
            },
            origin: Vec3([0., 0., -8.]),
            axes: [Vec3([0., 0., 1.]), Vec3([0., 1., 0.]), Vec3([-1., 0., 0.])],
            near: 1.,
            fov: [90.; 2],
            ..Default::default()
        },
        &[],
        assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!("MDC view")
    };
    assert_eq!(view.scene.draws.count, 1);
    let stats = cpu.render(&packet, assets);
    assert_eq!(stats.rejected, 0);
    assert!(stats.triangles > 0);
    assert!(cpu.inverse_depth().iter().any(|value| *value > 0.));
    cpu.pixels().to_vec()
}

#[test]
fn compressed_mdc_mesh_tags_and_skinned_pixels_use_independent_source_and_presentation_profiles() {
    let bytes = mdc(b"textures/base");
    let parsed = qa_formats::model::Model::parse(&bytes).unwrap();
    let mut files = resources();
    files.push(("models/packed.mdc", bytes.clone()));
    let fixture = support::Fixture::new(&files);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let before = assets.counts().models;
    let requests = [RuleSetId::Quake, RuleSetId::Quake2, RuleSetId::Quake3]
        .into_iter()
        .flat_map(|source| {
            [RuleSetId::Quake, RuleSetId::Quake2, RuleSetId::Quake3].map(move |target| {
                let mut request = ModelRequest::native("models/packed.mdc", source, Some(palette));
                request.rules = target;
                request.palette = (target != RuleSetId::Quake3).then_some(palette);
                request.image_settings =
                    qa_render::material::resources::ImageSettings::native(target);
                request
            })
        })
        .collect::<Vec<_>>();
    let mut owner = load(&fixture, assets, &requests, Default::default()).unwrap();
    assert!(owner.catalog().diagnostics.is_empty());
    assert_eq!(owner.assets().counts().models, before + 1);
    let mut model_id = None;
    let mut handle = None;
    for request in &requests {
        let profile = request.profile();
        let native = owner
            .catalog()
            .model_handle(request.path.as_bytes(), profile.source_rules)
            .unwrap();
        assert_eq!(*handle.get_or_insert(native), native);
        let registration = owner.catalog().registration(request.path, profile).unwrap();
        let binding = owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(*model_id.get_or_insert(binding.model), binding.model);
        let mesh = owner.assets().model(binding.model).unwrap();
        assert_eq!((mesh.frame_count(), mesh.native_frame_count()), (2, 2));
        let current = mesh.frames(1, 1, 0.).unwrap();
        assert_eq!(
            current.vertex(0).position,
            Vec3([0., -127. * 0.05, 128. * 0.05])
        );
        assert_eq!(current.vertex(0).normal, Vec3([1., 0., 0.]));
        assert_eq!(current.vertex(1).position, Vec3([1. + 0.05, 0., 0.]));
        let lods = owner
            .catalog()
            .lod_bindings(registration, 0, Default::default())
            .unwrap();
        assert!(lods.info().supports_lods());
        assert_eq!((lods.native_count(), lods.base_frames()), (1, 2));
        assert_eq!(lods.radius(0), Some(parsed.frames[0].bounds.radius()));
        assert_eq!(
            owner.catalog().model_bounds(native),
            Some(parsed.frames[0].bounds)
        );
        assert_eq!(
            owner
                .catalog()
                .metadata(request.path.as_bytes())
                .unwrap()
                .format,
            qa_formats::model::ModelFormat::Mdc
        );
    }
    let handle = handle.unwrap();
    let tag = owner
        .catalog()
        .lerp_tag(handle, 0, 1, 0.25, b"tag_weapon")
        .unwrap();
    assert_eq!(tag.origin, Vec3([1.25, 0., 0.]));
    assert_eq!(
        tag,
        parsed.tags[0]
            .orientation
            .interpolate(parsed.tags[1].orientation, 0.25)
    );
    assert_eq!(
        owner.catalog().lerp_tag(handle, 99, 99, 0., b"tag_weapon"),
        Some(parsed.tags[1].orientation)
    );
    assert!(
        owner
            .catalog()
            .lerp_tag(handle, 0, 1, 0.5, b"TAG_WEAPON")
            .is_none()
    );
    let mut skin_handle = None;
    for source in [RuleSetId::Quake, RuleSetId::Quake2, RuleSetId::Quake3] {
        let skin = owner
            .register_skin(&fixture.vfs, b"models/packed.skin", source)
            .unwrap();
        assert_eq!(*skin_handle.get_or_insert(skin), skin);
    }
    let skin = skin_handle.unwrap();
    for request in &requests {
        let profile = request.profile();
        let registration = owner.catalog().registration(request.path, profile).unwrap();
        let skinned = owner
            .catalog()
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
        let receipt = owner
            .catalog()
            .renderer_registrations(RendererRegistration::Skin, skin, profile)
            .unwrap()[0];
        assert_eq!(skinned.shader, receipt.shader);
        assert_eq!(skinned.model, model_id.unwrap());
    }
    let profile = requests.last().unwrap().profile();
    let registration = owner
        .catalog()
        .registration("models/packed.mdc", profile)
        .unwrap();
    let binding = owner
        .catalog()
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
    let mut cpu = CpuBackend::load_with_assets(64, 64, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    let pixels = render(&mut cpu, owner.assets(), binding);
    assert!(pixels.contains(&u32::from_le_bytes([191, 113, 71, 255])));
}

#[test]
fn requested_extension_prefers_its_format_and_missing_lod_alternates_preserve_sparse_slots() {
    for (requested, prefer_mdc, preferred_exists) in [
        ("models/mixed.mdc", true, true),
        ("models/mixed.mdc", true, false),
        ("models/mixed.md3", false, true),
        ("models/mixed.md3", false, false),
    ] {
        let mut files = resources();
        if !prefer_mdc || preferred_exists {
            files.push(("models/mixed.mdc", mdc(b"textures/base")));
        }
        if prefer_mdc || preferred_exists {
            files.push((
                "models/mixed.md3",
                support::md3_named_surfaces(&[(b"BODY_1", &[b"textures/alternate"])]),
            ));
        }
        let loaded_mdc = if preferred_exists {
            prefer_mdc
        } else {
            !prefer_mdc
        };
        files.push(if prefer_mdc {
            (
                "models/mixed_2.md3",
                support::md3_named_surfaces(&[(b"BODY_1", &[b"textures/coarse"])]),
            )
        } else {
            ("models/mixed_2.mdc", mdc(b"textures/coarse"))
        });
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
            .register_model(&fixture.vfs, requested.as_bytes(), profile.source_rules)
            .unwrap();
        let registration = owner
            .catalog()
            .renderer_registration(handle, profile)
            .unwrap();
        let lods = owner
            .catalog()
            .lod_bindings(registration, 0, Default::default())
            .unwrap();
        assert!(lods.info().supports_lods());
        assert_eq!(lods.native_count(), 2);
        let rows = lods.collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].each_ref().map(Option::is_some), [true, false, true]);
        assert_ne!(
            rows[0][0].as_ref().unwrap().model,
            rows[0][2].as_ref().unwrap().model
        );
        let metadata = owner.catalog().metadata(requested.as_bytes()).unwrap();
        assert_eq!(
            metadata.format,
            if loaded_mdc {
                qa_formats::model::ModelFormat::Mdc
            } else {
                qa_formats::model::ModelFormat::Md3
            }
        );
        let receipts = owner
            .catalog()
            .renderer_registrations(RendererRegistration::Model, handle, profile)
            .unwrap();
        assert_eq!(
            receipts
                .iter()
                .map(|receipt| owner.assets().name(receipt.key).unwrap())
                .collect::<Vec<_>>(),
            [
                "textures/coarse",
                if loaded_mdc {
                    "textures/base"
                } else {
                    "textures/alternate"
                }
            ]
        );
        assert_eq!(owner.catalog().capacity().registrations, 1);
        assert_eq!(
            owner
                .catalog()
                .model_handle(b"models/mixed_2.md3", profile.source_rules),
            None
        );
        assert_eq!(
            owner
                .catalog()
                .model_handle(b"models/mixed_2.mdc", profile.source_rules),
            None
        );
        fixture.vfs.take_lookup_count();
        assert_eq!(
            owner.register_model(&fixture.vfs, requested.as_bytes(), profile.source_rules),
            Ok(handle)
        );
        assert_eq!(fixture.vfs.take_lookup_count(), 0);
    }
}

#[test]
fn present_invalid_preferred_lod_never_uses_the_valid_alternate() {
    for failure in [None, Some(false), Some(true)] {
        let mut files = resources();
        files.extend([
            ("models/packed.mdc", mdc(b"textures/base")),
            ("models/packed_2.mdc", mdc(b"textures/coarse")),
            (
                "models/packed_1.md3",
                support::md3_named_surfaces(&[(b"BODY_1", &[b"textures/alternate"])]),
            ),
        ]);
        if let Some(unknown) = failure {
            let mut broken = mdc(b"textures/base");
            if unknown {
                broken[..4].copy_from_slice(b"NOPE");
            } else {
                broken[4..8].copy_from_slice(&999i32.to_le_bytes());
            }
            files.push(("models/packed_1.mdc", broken));
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
        let result = owner.register_model(&fixture.vfs, b"models/packed.mdc", profile.source_rules);
        if failure == Some(true) {
            assert_eq!(result, Err(qa_compat::services::CallError::File));
            fixture.vfs.take_lookup_count();
            assert_eq!(
                owner.register_model(&fixture.vfs, b"models/packed.mdc", profile.source_rules),
                Ok(0)
            );
            assert_eq!(fixture.vfs.take_lookup_count(), 0);
            continue;
        }
        let handle = result.unwrap();
        let registration = owner
            .catalog()
            .renderer_registration(handle, profile)
            .unwrap();
        let lods = owner
            .catalog()
            .lod_bindings(registration, 0, Default::default())
            .unwrap();
        assert_eq!(lods.native_count(), if failure.is_none() { 3 } else { 2 });
        assert_eq!(lods.base().is_some(), failure.is_none());
        let rows = lods.collect::<Vec<_>>();
        assert_eq!(
            rows[0].each_ref().map(Option::is_some),
            if failure.is_none() {
                [true; 3]
            } else {
                [false, false, true]
            }
        );
        let receipts = owner
            .catalog()
            .renderer_registrations(RendererRegistration::Model, handle, profile)
            .unwrap();
        assert_eq!(receipts.len(), if failure.is_none() { 3 } else { 1 });
    }
}

#[test]
fn retained_mdc_profile_rebuild_rolls_back_and_retries_after_model_and_skin_files_are_removed() {
    let mut files = resources();
    files.push(("models/packed.mdc", mdc(b"textures/base")));
    let mut fixture = support::Fixture::new(&files);
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
        .register_model(&fixture.vfs, b"models/packed.mdc", profile.source_rules)
        .unwrap();
    let skin = owner
        .register_skin(&fixture.vfs, b"models/packed.skin", profile.source_rules)
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let original = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    let bounds = owner.catalog().model_bounds(handle);
    let tag = owner.catalog().lerp_tag(handle, 0, 1, 0.5, b"tag_weapon");
    let mut cpu = CpuBackend::load_with_assets(64, 64, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let old_pixels = render(&mut cpu, owner.assets(), original);
    for path in ["models/packed.mdc", "models/packed.skin"] {
        fixture.remove(path);
    }
    let (mount, path) = {
        let mount = fixture.vfs.mounts().next().unwrap();
        (mount.id, mount.path.clone())
    };
    assert!(fixture.vfs.unmount(mount));
    fixture.vfs.mount_directory(&path, 0).unwrap();
    assert!(fixture.vfs.open(b"models/packed.mdc").is_none());
    assert!(fixture.vfs.open(b"models/packed.skin").is_none());
    let mut settings = profile.image_settings;
    settings.picmip += 1;
    let (next, _) = owner
        .prepare_profile(
            &fixture.vfs,
            profile.source_rules,
            profile.rules,
            settings,
            profile.finish,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = renderer_request(
        "models/packed.mdc",
        RendererRegistration::Model,
        profile.source_rules,
        None,
    );
    let prefix = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                next,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| Err("reject MDC material publication".to_owned())
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(cpu.admitted_assets(), Some(prefix));
    assert!(
        owner
            .catalog()
            .renderer_registration(handle, next)
            .is_none()
    );
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next(),
        Some(original)
    );
    assert_eq!(render(&mut cpu, owner.assets(), original), old_pixels);
    let (reused, _, counts) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            next,
            RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(reused, handle);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    let registration = owner.catalog().renderer_registration(handle, next).unwrap();
    let current = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(current.model, original.model);
    assert_eq!(owner.assets().counts().models, prefix.models);
    let material = owner
        .assets()
        .material(
            owner
                .assets()
                .shader_binding(current.shader)
                .unwrap()
                .material,
        )
        .unwrap();
    let StageTexture::Image(image) = material.stages[0].texture else {
        panic!("refreshed MDC image")
    };
    let image = owner.assets().image(image).unwrap();
    assert_eq!((image.width, image.height), (8, 8));
    let uploaded = &image.prepared.as_ref().unwrap().levels[0];
    assert_eq!((uploaded.width, uploaded.height), (2, 2));
    assert_eq!(owner.catalog().model_bounds(handle), bounds);
    assert_eq!(
        owner.catalog().lerp_tag(handle, 0, 1, 0.5, b"tag_weapon"),
        tag
    );
    let request = renderer_request(
        "models/packed.skin",
        RendererRegistration::Skin,
        profile.source_rules,
        None,
    );
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                next,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        skin
    );
    let skinned = owner
        .catalog()
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
    assert_eq!(skinned.model, original.model);
    assert_eq!(
        skinned.shader,
        owner
            .catalog()
            .renderer_registrations(RendererRegistration::Skin, skin, next)
            .unwrap()[0]
            .shader
    );
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &renderer_request(
                    "models/packed.mdc",
                    RendererRegistration::Model,
                    profile.source_rules,
                    None
                ),
                next,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("cached MDC must not prepare backend")
            )
            .unwrap()
            .0,
        handle
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}
