use super::*;
use qa_compat::services::{RendererRegistration, ShaderLighting};

#[test]
fn native_membership_keeps_all_model_shaders_and_skin_declaration_order() {
    let fixture = support::Fixture::surfaces();
    let mut assets = Assets::load().unwrap();
    // The renderer and native model catalog have distinct existing name arenas.
    assets
        .intern_resource_name(b"renderer-only-prefix")
        .unwrap();
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [], Default::default()).unwrap();
    let model = owner
        .register_model(&fixture.vfs, b"models/test.md3", profile.source_rules)
        .unwrap();
    let skin = owner
        .register_skin(&fixture.vfs, b"models/test.skin", profile.source_rules)
        .unwrap();
    let model_registrations = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Model, model, profile)
        .unwrap();
    assert_eq!(
        model_registrations
            .iter()
            .map(|entry| owner.assets().name(entry.key).unwrap())
            .collect::<Vec<_>>(),
        ["textures/body", "textures/alternate", "textures/glow"]
    );
    let registration = owner
        .catalog()
        .renderer_registration(model, profile)
        .unwrap();
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .len(),
        2
    );
    let skin_registrations = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Skin, skin, profile)
        .unwrap();
    // Two declarations address BODY. The mapping's sorted/deduplicated draw
    // table must not erase the later shader's original registration request.
    assert_eq!(
        skin_registrations
            .iter()
            .map(|entry| owner.assets().name(entry.key).unwrap())
            .collect::<Vec<_>>(),
        ["textures/alternate", "textures/glow", "textures/body"]
    );
    assert_eq!(skin_registrations[0], model_registrations[1]);
    assert_eq!(skin_registrations[1], model_registrations[2]);
    assert_eq!(skin_registrations[2], model_registrations[0]);
}

#[test]
fn ordinary_registration_keys_keep_raw_separator_identity_and_fold_only_ascii_case() {
    let fixture =
        support::Fixture::new(&[("ui/pic.tga", support::tga(1, 1, &[[31, 43, 59, 255]]))]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let kind = RendererRegistration::Shader {
        mipmaps: false,
        lighting: ShaderLighting::TwoDimensional,
    };
    let mut entries = Vec::new();
    for raw in [b"ui/pic.tga".as_slice(), b"ui\\pic.tga", b"UI/PIC.jpg"] {
        let handle = owner
            .register_shader(
                &fixture.vfs,
                raw,
                profile.source_rules,
                false,
                ShaderLighting::TwoDimensional,
            )
            .unwrap();
        entries.push(
            owner
                .catalog()
                .renderer_registrations(kind, handle, profile)
                .unwrap()[0],
        );
    }
    assert_eq!(entries[0], entries[2]);
    assert_ne!(entries[0].key, entries[1].key);
    assert_eq!(owner.assets().name(entries[0].key), Some("ui/pic"));
    assert_eq!(owner.assets().name(entries[1].key), Some("ui\\pic"));
    // Path-canonical compiled values may be shared; logical native names remain
    // distinct even when the same shader instantiation serves both requests.
    assert_eq!(entries[0].shader, entries[1].shader);
}

#[test]
fn implicit_named_default_retries_backend_then_publishes_zero_and_caches_all_lighting() {
    use qa_render::{NativeDefaultKind, ShaderBindingId, cpu::CpuBackend};
    use std::cell::Cell;
    let fixture =
        support::Fixture::new(&[("ui/retained.tga", support::tga(1, 1, &[[31, 43, 59, 255]]))]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut assets = Assets::load().unwrap();
    let world = support::world(&mut assets, 0.0);
    let presentation = assets.world(world).unwrap().presentations()[0];
    assets
        .register_world_presentation_metadata(
            world,
            &[qa_render::material::world_load::LoadedWorldPresentation {
                id: presentation,
                profile: world_registration(profile).presentation,
                lighting: Default::default(),
                compile_policy: Default::default(),
                defaults: Default::default(),
            }],
            None,
        )
        .unwrap();
    let mut owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [world], Default::default()).unwrap();
    let retained = owner
        .register_shader(
            &fixture.vfs,
            b"ui/retained.tga",
            profile.source_rules,
            false,
            ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let retained_shader = owner.catalog().shader_binding(retained, profile).unwrap();
    let retained_binding = *owner.assets().shader_binding(retained_shader).unwrap();
    let retained_image = match owner
        .assets()
        .material(retained_binding.material)
        .unwrap()
        .stages[0]
        .texture
    {
        StageTexture::Image(image) => image,
        _ => panic!("retained image"),
    };
    let mut cpu = CpuBackend::load_with_assets(8, 8, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let before = owner.assets().counts();
    let request = renderer_request(
        "ui/missing.tga",
        RendererRegistration::Shader {
            mipmaps: false,
            lighting: ShaderLighting::TwoDimensional,
        },
        profile.source_rules,
        None,
    );
    let attempts = Cell::new(0);
    fixture.vfs.take_lookup_count();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                profile,
                qa_app::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| {
                    attempts.set(attempts.get() + 1);
                    Err("backend rejected named body".to_owned())
                }
            )
            .is_err()
    );
    assert_eq!(attempts.get(), 1);
    assert!(fixture.vfs.take_lookup_count() > 0);
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert_eq!(
        owner
            .catalog()
            .registration_cached(request.kind, b"ui/missing.tga", profile, None),
        None
    );
    assert!(
        owner
            .catalog()
            .shader_outcome(
                b"ui/missing.tga",
                profile,
                ShaderLighting::TwoDimensional,
                None
            )
            .is_none()
    );
    let (handle, _, admitted) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            profile,
            qa_app::model_assets::RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| {
                attempts.set(attempts.get() + 1);
                cpu.prepare_assets(assets, None).map_err(str::to_owned)
            },
        )
        .unwrap();
    assert_eq!(handle, 0);
    assert_eq!(attempts.get(), 2);
    assert!(fixture.vfs.take_lookup_count() > 0);
    assert_eq!(cpu.admitted_assets(), Some(admitted));
    assert!(
        admitted.materials > before.materials && admitted.shader_bindings > before.shader_bindings
    );
    let outcome = *owner
        .catalog()
        .shader_outcome(
            b"ui/missing.tga",
            profile,
            ShaderLighting::TwoDimensional,
            None,
        )
        .unwrap();
    assert_eq!(outcome.defaulted, Some(NativeDefaultKind::MissingImage));
    assert_eq!(
        outcome.requested,
        qa_render::world::geometry::LightSource::TwoDimensional
    );
    assert_ne!(outcome.shader, ShaderBindingId(0));
    let body = owner
        .assets()
        .material(
            owner
                .assets()
                .shader_binding(outcome.shader)
                .unwrap()
                .material,
        )
        .unwrap();
    assert!(body.stages.is_empty());
    assert_eq!(body.settings.sort, 7.0);
    assert!(body.settings.sky.is_none() && body.settings.fog.is_none());
    assert!(owner.catalog().shader_binding(0, profile).is_none());
    let capacity = owner.catalog().capacity();
    fixture.vfs.take_lookup_count();
    for lighting in [
        ShaderLighting::TwoDimensional,
        ShaderLighting::World(Some(presentation)),
    ] {
        let cached = renderer_request(
            "UI/MISSING.jpg",
            RendererRegistration::Shader {
                mipmaps: true,
                lighting,
            },
            profile.source_rules,
            Some(world),
        );
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &cached,
                    profile,
                    qa_app::model_assets::RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |_, _| { panic!("published native default reached backend") }
                )
                .unwrap(),
            (0, (), admitted)
        );
        assert_eq!(
            owner
                .catalog()
                .shader_outcome(b"ui/missing", profile, lighting, None),
            Some(&outcome)
        );
    }
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(
        owner.catalog().shader_binding(retained, profile),
        Some(retained_shader)
    );
    assert_eq!(
        owner.assets().shader_binding(retained_shader),
        Some(&retained_binding)
    );
    assert_eq!(
        owner.assets().image(retained_image).unwrap().rgba.as_ref(),
        [31, 43, 59, 255]
    );
}

#[test]
fn optional_missing_md3_shader_keeps_model_and_skin_pointer_body_without_white_alias() {
    use qa_render::{NativeDefaultKind, ShaderBindingId, cpu::CpuBackend};
    let fixture = support::Fixture::new(&[
        ("models/test.md3", support::md3_surfaces()),
        (
            "models/test.skin",
            b"body,textures/alternate\nglow,textures/body\n".to_vec(),
        ),
        (
            "textures/alternate.tga",
            support::tga(2, 2, &[[10, 90, 20, 255]; 4]),
        ),
        (
            "textures/glow.tga",
            support::tga(2, 2, &[[20, 10, 90, 255]; 4]),
        ),
    ]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let model = owner
        .register_model(&fixture.vfs, b"models/test.md3", profile.source_rules)
        .unwrap();
    let skin = owner
        .register_skin(&fixture.vfs, b"models/test.skin", profile.source_rules)
        .unwrap();
    assert!(model > 0 && skin > 0);
    let cpu = CpuBackend::load_with_assets(8, 8, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();

    let model_rows = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Model, model, profile)
        .unwrap();
    assert_eq!(model_rows.len(), 3);
    let missing = model_rows[0];
    assert_eq!(missing.defaulted, Some(NativeDefaultKind::MissingImage));
    assert_ne!(missing.shader, ShaderBindingId(0));
    assert_eq!(
        missing.requested,
        qa_render::world::geometry::LightSource::None
    );
    let registration = owner
        .catalog()
        .renderer_registration(model, profile)
        .unwrap();
    let native: Vec<_> = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .collect();
    assert_eq!(native.len(), 2);
    assert_eq!(
        native[0].shader,
        owner.catalog().native_default(profile).unwrap(),
        "MD3 public index0 resolves to the native global default"
    );
    assert_ne!(native[1].shader, ShaderBindingId(0));
    let skin_rows = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Skin, skin, profile)
        .unwrap();
    assert_eq!(skin_rows[1], missing);
    let custom: Vec<_> = owner
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
        .collect();
    assert_eq!(
        custom[1].shader, missing.shader,
        "skin mapping retains the named shader pointer body"
    );
    let body = owner
        .assets()
        .material(
            owner
                .assets()
                .shader_binding(missing.shader)
                .unwrap()
                .material,
        )
        .unwrap();
    assert!(body.stages.is_empty());
    assert_eq!(body.settings.sort, 7.0);
    assert!(
        owner
            .catalog()
            .renderer_registration(model, profile)
            .is_some()
    );
    assert!(owner.catalog().renderer_skin(skin, profile));
}

#[test]
fn admitted_positive_profile_survives_new_default_sibling_and_keeps_source_handle() {
    use qa_render::{NativeDefaultKind, cpu::CpuBackend};
    let mut fixture =
        support::Fixture::new(&[("ui/sibling.tga", support::tga(1, 1, &[[31, 43, 59, 255]]))]);
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
        .register_shader(
            &fixture.vfs,
            b"ui/sibling.tga",
            profile.source_rules,
            false,
            ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let retained = owner.catalog().shader_binding(handle, profile).unwrap();
    let mut cpu = CpuBackend::load_with_assets(8, 8, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let mut settings = profile.image_settings;
    settings.gamma_exponent = 1.5;
    let (sibling, _) = owner
        .prepare_profile(
            &fixture.vfs,
            profile.source_rules,
            profile.rules,
            settings,
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_ne!(sibling, profile);
    let mount = fixture.vfs.mounts().next().unwrap().id;
    assert!(fixture.vfs.unmount(mount));
    assert!(fixture.vfs.open(b"ui/sibling.tga").is_none());
    assert_eq!(
        owner
            .register_shader(
                &fixture.vfs,
                b"ui/sibling.tga",
                profile.source_rules,
                true,
                ShaderLighting::TwoDimensional
            )
            .unwrap(),
        handle
    );
    assert_eq!(
        owner
            .catalog()
            .shader_handle(b"ui/sibling", profile.source_rules),
        Some(handle)
    );
    assert_eq!(
        owner.catalog().registration_cached(
            RendererRegistration::Shader {
                mipmaps: true,
                lighting: ShaderLighting::TwoDimensional,
            },
            b"ui/sibling",
            sibling,
            None
        ),
        None,
        "default body is still unadmitted"
    );
    assert!(
        owner
            .catalog()
            .shader_outcome(b"ui/sibling", sibling, ShaderLighting::TwoDimensional, None)
            .is_none()
    );
    cpu.prepare_assets(owner.assets(), None).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    let outcome = owner
        .catalog()
        .shader_outcome(b"ui/sibling", sibling, ShaderLighting::TwoDimensional, None)
        .unwrap();
    assert_eq!(outcome.defaulted, Some(NativeDefaultKind::MissingImage));
    assert_eq!(
        owner.catalog().shader_binding(handle, profile),
        Some(retained)
    );
    assert_eq!(owner.catalog().shader_binding(handle, sibling), None);
    assert_eq!(
        owner.catalog().registration_cached(
            RendererRegistration::Shader {
                mipmaps: true,
                lighting: ShaderLighting::TwoDimensional,
            },
            b"ui/sibling",
            sibling,
            None
        ),
        Some(0)
    );
}

#[test]
fn indexed_native_builtin_and_shaderless_md3_share_profile_default_image_and_keep_geometry() {
    use qa_render::cpu::CpuBackend;
    for empty_shaders in [false, true] {
        let mut bytes = support::md3();
        let surface = i32::from_le_bytes(bytes[100..104].try_into().unwrap()) as usize;
        // The shared format fixture is a collinear metadata mesh. Author a
        // front-facing triangle for this real CPU appearance proof.
        let vertices = surface
            + i32::from_le_bytes(bytes[surface + 100..surface + 104].try_into().unwrap()) as usize;
        let frame_count =
            i32::from_le_bytes(bytes[surface + 72..surface + 76].try_into().unwrap()) as usize;
        for frame in 0..frame_count {
            for (vertex, position) in [[32i16, 16, 28], [32, 32, 28], [32, 24, 44]]
                .into_iter()
                .enumerate()
            {
                for (axis, value) in position.into_iter().enumerate() {
                    let at = vertices + (frame * 3 + vertex) * 8 + axis * 2;
                    bytes[at..at + 2].copy_from_slice(&(value * 64).to_le_bytes());
                }
            }
        }
        let coordinates = surface
            + i32::from_le_bytes(bytes[surface + 96..surface + 100].try_into().unwrap()) as usize;
        for (vertex, uv) in [[0f32, 0.], [1., 0.], [0.5, 1.]].into_iter().enumerate() {
            for (axis, value) in uv.into_iter().enumerate() {
                let at = coordinates + vertex * 8 + axis * 4;
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }

        if empty_shaders {
            bytes[surface + 76..surface + 80].copy_from_slice(&0i32.to_le_bytes());
        } else {
            let shader = surface
                + i32::from_le_bytes(bytes[surface + 92..surface + 96].try_into().unwrap())
                    as usize;
            bytes[shader..shader + 64].fill(0);
            bytes[shader..shader + 8].copy_from_slice(b"*default");
        }
        let fixture = support::Fixture::new(&[("models/default.md3", bytes)]);
        let mut assets = Assets::load().unwrap();
        let palette = support::palette(&mut assets);
        let mut profile = shader_profile(RuleSetId::Quake);
        profile.palette = Some(palette);
        profile.image_settings.picmip = 2;
        let mut owner =
            ModelAssets::new(&fixture.vfs, assets, [profile], [], Default::default()).unwrap();
        let before = owner.assets().counts();
        let handle = owner
            .register_model(&fixture.vfs, b"models/default.md3", RuleSetId::Quake3)
            .unwrap();
        assert!(handle > 0);
        assert_eq!(
            owner.assets().counts().images,
            before.images,
            "the builtin uses the profile's first indexed image"
        );
        assert_eq!(owner.assets().counts().models, before.models + 1);
        let mut cpu = CpuBackend::load_with_assets(64, 48, owner.assets()).unwrap();
        owner
            .publish_admission(cpu.admitted_assets().unwrap())
            .unwrap();
        let fallback = owner.catalog().native_default(profile).unwrap();
        let StageTexture::Image(default_image) = owner
            .assets()
            .material(owner.assets().shader_binding(fallback).unwrap().material)
            .unwrap()
            .stages[0]
            .texture
        else {
            panic!("native default image")
        };

        let registration = owner
            .catalog()
            .renderer_registration(handle, profile)
            .unwrap();
        let draw = owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next()
            .unwrap();
        let StageTexture::Image(model_image) = owner
            .assets()
            .material(owner.assets().shader_binding(draw.shader).unwrap().material)
            .unwrap()
            .stages[0]
            .texture
        else {
            panic!("model image")
        };
        assert_eq!(model_image, default_image);
        if empty_shaders {
            assert_eq!(draw.shader, fallback);
        }
        let image = owner.assets().image(model_image).unwrap();
        assert!(image.indexed.is_some());
        assert_eq!(
            (
                image.prepared.as_ref().unwrap().levels[0].width,
                image.prepared.as_ref().unwrap().levels[0].height
            ),
            (16, 16)
        );
        let (pixels, stats) = render_indexed_on(&mut cpu, owner.assets(), draw, palette);
        assert_eq!(stats.rejected, 0);
        assert!(
            pixels
                .iter()
                .any(|&pixel| pixel != u32::from_le_bytes([3, 5, 7, 255])),
            "native default remains visible through the indexed CPU model path"
        );
    }
}
