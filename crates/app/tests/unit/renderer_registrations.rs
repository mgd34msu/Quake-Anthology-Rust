use super::*;
use qa_render::cpu::{CpuBackend, CpuLimits};
use std::cell::Cell;

#[allow(dead_code)]
#[path = "../model_assets/support.rs"]
mod support;

fn profile(source: RuleSetId) -> ModelProfile {
    ModelRequest::native("", source, None).profile()
}

fn request(name: &[u8], source: RuleSetId, kind: RendererRegistration) -> RendererRequest<&[u8]> {
    RendererRequest {
        name,
        source,
        kind,
        world: None,
    }
}

fn publish(mut owner: ModelAssets) -> (ModelAssets, CpuBackend) {
    let cpu = CpuBackend::load_with_limits(
        8,
        8,
        owner.assets(),
        CpuLimits {
            cache_bytes: 1024 * 1024,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    (owner, cpu)
}

fn shader_fixture() -> support::Fixture {
    support::Fixture::new(&[
        ("textures/missing.tga", b"invalid image".to_vec()),
        (
            "textures/first.tga",
            support::tga(4, 4, &[[211, 17, 29, 255]; 16]),
        ),
        (
            "textures/second.tga",
            support::tga(4, 4, &[[31, 223, 47, 255]; 16]),
        ),
        (
            "textures/third.tga",
            support::tga(4, 4, &[[53, 67, 229, 255]; 16]),
        ),
    ])
}

#[test]
fn pair_publishes_once_then_uses_both_cached_handles_without_io() {
    let fixture = shader_fixture();
    let profile = profile(RuleSetId::Quake3);
    let owner =
        ModelAssets::new(Assets::load().unwrap(), [profile], [], Default::default()).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let kind = RendererRegistration::Shader {
        mipmaps: false,
        lighting: qa_compat::services::ShaderLighting::TwoDimensional,
    };
    let first = request(b"textures/first.tga", profile.source_rules, kind);
    let second = request(b"textures/second.tga", profile.source_rules, kind);
    let prepares = Cell::new(0);
    let (handles, _, counts) = owner
        .prepare_registrations(
            &fixture.vfs,
            [&first, &second],
            profile,
            |_| Ok(()),
            |assets, _| {
                prepares.set(prepares.get() + 1);
                cpu.prepare_assets(assets, None).map_err(str::to_owned)
            },
        )
        .unwrap();
    assert_eq!(prepares.get(), 1);
    assert!(handles.iter().all(|&handle| handle > 0));
    assert_ne!(handles[0], handles[1]);
    assert!(counts.images > before.images);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    for (&handle, request) in handles.iter().zip([&first, &second]) {
        assert_eq!(
            owner
                .catalog()
                .registration_cached(kind, request.name, profile, None),
            Some(handle)
        );
        let shader = owner.catalog().shader_binding(handle, profile).unwrap();
        assert_eq!(
            owner.assets().shader_binding(shader).unwrap().light_source,
            LightSource::TwoDimensional
        );
    }
    let capacity = owner.catalog().capacity();
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                [&first, &second],
                profile,
                |_| Ok(()),
                |_, _| { panic!("cached pair reached backend") }
            )
            .unwrap(),
        (handles, (), counts)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(owner.catalog().capacity(), capacity);
}

#[test]
fn pair_backend_rejection_restores_both_existing_shaders_and_admitted_peer() {
    let fixture = shader_fixture();
    let peer = profile(RuleSetId::Quake);
    let target = profile(RuleSetId::Quake2);
    let mut owner = ModelAssets::new(
        Assets::load().unwrap(),
        [peer, target],
        [],
        Default::default(),
    )
    .unwrap();
    let names: [&[u8]; 2] = [b"textures/first.tga", b"textures/second.tga"];
    let handles = names.map(|name| {
        owner
            .register_shader(
                &fixture.vfs,
                name,
                peer.source_rules,
                false,
                qa_compat::services::ShaderLighting::TwoDimensional,
            )
            .unwrap()
    });
    let retained = handles.map(|handle| owner.catalog().shader_binding(handle, peer).unwrap());
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let kind = RendererRegistration::Shader {
        mipmaps: false,
        lighting: qa_compat::services::ShaderLighting::TwoDimensional,
    };
    let requests = names.map(|name| request(name, target.source_rules, kind));
    assert!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                requests.each_ref(),
                target,
                |_| Ok(()),
                |_, _| { Err("backend rejected both candidates".to_owned()) }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(cpu.admitted_assets(), Some(before));
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(
            owner.catalog().shader_binding(handles[index], peer),
            Some(retained[index])
        );
        assert_eq!(
            owner
                .catalog()
                .registration_cached(kind, request.name, target, None),
            None
        );
    }
    let retry_kind = RendererRegistration::Shader {
        mipmaps: true,
        lighting: qa_compat::services::ShaderLighting::TwoDimensional,
    };
    let retries = names.map(|name| request(name, target.source_rules, retry_kind));
    let (retried, _, counts) = owner
        .prepare_registrations(
            &fixture.vfs,
            retries.each_ref(),
            target,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(retried, handles);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    for (index, handle) in handles.into_iter().enumerate() {
        assert_eq!(
            owner.catalog().shader_binding(handle, peer),
            Some(retained[index])
        );
        let material = owner.catalog().shader_binding(handle, target).unwrap();
        assert!(
            owner
                .assets()
                .material(owner.assets().shader_binding(material).unwrap().material)
                .unwrap()
                .stages[0]
                .sampler
                .mipmaps
        );
        assert!(
            !owner
                .assets()
                .material(
                    owner
                        .assets()
                        .shader_binding(retained[index])
                        .unwrap()
                        .material
                )
                .unwrap()
                .stages[0]
                .sampler
                .mipmaps
        );
    }
}

#[test]
fn pair_source_failure_keeps_only_negative_metadata_and_does_not_load_on_cached_failure() {
    let fixture = shader_fixture();
    let peer = profile(RuleSetId::Quake);
    let profile = profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        Assets::load().unwrap(),
        [peer, profile],
        [],
        Default::default(),
    )
    .unwrap();
    let retained = owner
        .register_shader(
            &fixture.vfs,
            b"textures/first.tga",
            peer.source_rules,
            false,
            qa_compat::services::ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let retained_material = owner.catalog().shader_binding(retained, peer).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let kind = RendererRegistration::Shader {
        mipmaps: false,
        lighting: qa_compat::services::ShaderLighting::TwoDimensional,
    };
    let first = request(b"textures/first.tga", profile.source_rules, kind);
    let missing = request(b"textures/missing.tga", profile.source_rules, kind);
    assert!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                [&first, &missing],
                profile,
                |_| Ok(()),
                |_, _| { panic!("failed second resource reached backend") }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert_eq!(
        owner.catalog().shader_binding(retained, peer),
        Some(retained_material)
    );
    assert_eq!(
        owner
            .catalog()
            .registration_cached(kind, first.name, profile, None),
        None
    );
    assert_eq!(
        owner
            .catalog()
            .registration_cached(kind, missing.name, profile, None),
        Some(0)
    );
    let first_name = owner
        .catalog
        .names
        .find(qa_render::shader::registration_key(first.name))
        .unwrap();
    let capacity = owner.catalog().capacity();
    let diagnostics = owner.catalog.diagnostics.len();
    fixture.vfs.take_lookup_count();
    for _ in 0..2 {
        assert!(
            owner
                .prepare_registrations(
                    &fixture.vfs,
                    [&first, &missing],
                    profile,
                    |_| Ok(()),
                    |_, _| { panic!("cached failed member reached backend") }
                )
                .is_err()
        );
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &missing,
                    profile,
                    |_| Ok(()),
                    |_, _| { panic!("cached negative single reached backend") }
                )
                .unwrap(),
            (0, (), before)
        );
    }
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(owner.catalog.diagnostics.len(), diagnostics);
    let retry = request(
        first.name,
        profile.source_rules,
        RendererRegistration::Shader {
            mipmaps: true,
            lighting: qa_compat::services::ShaderLighting::TwoDimensional,
        },
    );
    let (handle, _, counts) = owner
        .prepare_registration(
            &fixture.vfs,
            &retry,
            profile,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(handle, retained);
    assert_eq!(
        owner
            .catalog
            .names
            .find(qa_render::shader::registration_key(first.name)),
        Some(first_name)
    );
    let material = owner.catalog().shader_binding(handle, profile).unwrap();
    assert!(
        owner
            .assets()
            .material(owner.assets().shader_binding(material).unwrap().material)
            .unwrap()
            .stages[0]
            .sampler
            .mipmaps
    );
    assert_eq!(cpu.admitted_assets(), Some(counts));
    assert_eq!(
        owner.catalog().shader_binding(retained, peer),
        Some(retained_material)
    );
    assert_eq!(
        owner
            .catalog()
            .registration_cached(kind, missing.name, profile, None),
        Some(0)
    );
}

#[test]
fn duplicate_folded_shader_requests_preserve_first_flags_after_atomic_retry() {
    let fixture = shader_fixture();
    let peer = profile(RuleSetId::Quake);
    let target = profile(RuleSetId::Quake2);
    let mut owner = ModelAssets::new(
        Assets::load().unwrap(),
        [peer, target],
        [],
        Default::default(),
    )
    .unwrap();
    let handle = owner
        .register_shader(
            &fixture.vfs,
            b"textures/first.tga",
            peer.source_rules,
            false,
            qa_compat::services::ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let retained = owner.catalog().shader_binding(handle, peer).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let first = request(
        b"textures/first.tga",
        target.source_rules,
        RendererRegistration::Shader {
            mipmaps: false,
            lighting: qa_compat::services::ShaderLighting::TwoDimensional,
        },
    );
    let duplicate = request(
        b"TEXTURES/FIRST.jpg",
        target.source_rules,
        RendererRegistration::Shader {
            mipmaps: true,
            lighting: qa_compat::services::ShaderLighting::TwoDimensional,
        },
    );
    assert!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                [&first, &duplicate],
                target,
                |_| Ok(()),
                |_, _| { Err("reject shared candidate".to_owned()) }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().shader_binding(handle, peer), Some(retained));
    let (handles, _, counts) = owner
        .prepare_registrations(
            &fixture.vfs,
            [&first, &duplicate],
            target,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(handles, [handle, handle]);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    let material = owner.catalog().shader_binding(handle, target).unwrap();
    assert!(
        !owner
            .assets()
            .material(owner.assets().shader_binding(material).unwrap().material)
            .unwrap()
            .stages[0]
            .sampler
            .mipmaps
    );
}

#[test]
fn model_and_skin_pair_backend_retries_keep_name_ids_and_admitted_bindings() {
    let fixture = support::Fixture::surfaces();
    let profile = profile(RuleSetId::Quake3);
    let mut owner =
        ModelAssets::new(Assets::load().unwrap(), [profile], [], Default::default()).unwrap();
    let retained = owner
        .register_shader(
            &fixture.vfs,
            b"textures/body",
            profile.source_rules,
            false,
            qa_compat::services::ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let retained_material = owner.catalog().shader_binding(retained, profile).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let model = request(
        b"models/test.md3",
        profile.source_rules,
        RendererRegistration::Model,
    );
    let skin = request(
        b"models/test.skin",
        profile.source_rules,
        RendererRegistration::Skin,
    );
    let mut model_name = None;
    let mut skin_name = None;
    for _ in 0..2 {
        assert!(
            owner
                .prepare_registrations(
                    &fixture.vfs,
                    [&model, &skin],
                    profile,
                    |_| Ok(()),
                    |_, _| { Err("reject appended model and skin".to_owned()) }
                )
                .is_err()
        );
        assert_eq!(owner.assets().counts(), before);
        assert_eq!(cpu.admitted_assets(), Some(before));
        assert_eq!(
            owner.catalog().shader_binding(retained, profile),
            Some(retained_material)
        );
        assert_eq!(
            owner
                .catalog()
                .registration_cached(model.kind, model.name, profile, None),
            None
        );
        assert_eq!(
            owner
                .catalog()
                .registration_cached(skin.kind, skin.name, profile, None),
            None
        );
        let captured_model = owner.catalog.names.find(model.name).unwrap();
        let captured_skin = owner.catalog.names.find(skin.name).unwrap();
        if let Some(model_name) = model_name {
            assert_eq!(captured_model, model_name);
            assert_eq!(Some(captured_skin), skin_name);
        }
        model_name = Some(captured_model);
        skin_name = Some(captured_skin);
    }
    let (handles, _, counts) = owner
        .prepare_registrations(
            &fixture.vfs,
            [&model, &skin],
            profile,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(Some(NameId(handles[0] as u32)), model_name);
    assert!(
        owner
            .catalog()
            .renderer_registration(handles[0], profile)
            .is_some()
    );
    assert!(owner.catalog().renderer_skin(handles[1], profile));
    let registration = owner
        .catalog()
        .renderer_registration(handles[0], profile)
        .unwrap();
    for binding in owner
        .catalog()
        .bindings(
            registration,
            0,
            ModelMaterialPolicy {
                custom_skin: handles[1],
                ..Default::default()
            },
        )
        .unwrap()
    {
        assert_eq!(
            owner
                .assets()
                .shader_binding(binding.shader)
                .unwrap()
                .light_source,
            LightSource::None
        );
    }
    assert_eq!(owner.catalog.names.find(skin.name), skin_name);
    assert_eq!(
        owner.catalog().shader_binding(retained, profile),
        Some(retained_material)
    );
    assert_eq!(cpu.admitted_assets(), Some(counts));
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                [&model, &skin],
                profile,
                |_| Ok(()),
                |_, _| { panic!("cached model/skin pair reached backend") }
            )
            .unwrap(),
        (handles, (), counts)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}

#[test]
fn exact_world_lighting_requests_share_the_batch_transaction_and_keep_ordinary_handles() {
    use qa_render::{
        material::world_load::{LoadedWorldPresentation, WorldPresentationProfile},
        world::WorldPresentationInput,
    };
    let fixture = support::Fixture::new(&[
        (
            "textures/base.tga",
            support::tga(2, 2, &[[31, 43, 59, 255]; 4]),
        ),
        (
            "textures/implicit.tga",
            support::tga(2, 2, &[[61, 73, 89, 255]; 4]),
        ),
        (
            "scripts/light.shader",
            b"textures/lit { { map $lightmap } { map textures/base.tga blendFunc filter } }"
                .to_vec(),
        ),
    ]);
    let profile = profile(RuleSetId::Quake3);
    let mut assets = Assets::load().unwrap();
    let pages = [
        assets.register_image(1, 1, &[17, 19, 23, 255]).unwrap(),
        assets.register_image(1, 1, &[29, 31, 37, 255]).unwrap(),
    ];
    let (geometry, visibility) = support::world_inputs(0.0, |_| {});
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[
                WorldPresentationInput {
                    native_page0: Some(pages[0]),
                    ..WorldPresentationInput::new(&[])
                },
                WorldPresentationInput {
                    native_page0: Some(pages[1]),
                    ..WorldPresentationInput::new(&[])
                },
                WorldPresentationInput::new(&[]),
            ],
        )
        .unwrap();
    let presentations: [WorldPresentationId; 3] = assets
        .world(world)
        .unwrap()
        .presentations()
        .try_into()
        .unwrap();
    let entries = presentations.map(|id| LoadedWorldPresentation {
        id,
        profile: WorldPresentationProfile::native(profile.rules, profile.palette),
        defaults: Default::default(),
    });
    assets
        .register_world_presentation_metadata(world, &entries)
        .unwrap();
    let owner = ModelAssets::new(assets, [profile], [world], Default::default()).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let ordinary = request(
        b"textures/lit",
        profile.source_rules,
        RendererRegistration::Shader {
            mipmaps: false,
            lighting: ShaderLighting::TwoDimensional,
        },
    );
    let (handle, _, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &ordinary,
            profile,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let original = owner.catalog().shader_binding(handle, profile).unwrap();
    let original_material = owner.assets().shader_binding(original).unwrap().material;
    assert_eq!(
        owner.assets().material(original_material).unwrap().stages[0].texture,
        StageTexture::Image(ImageId(0))
    );
    let before = owner.assets().counts();
    let shader_index = handle as usize - 1;
    let profile_rows = owner.catalog.shaders[shader_index].profiles.len();
    let all_rows = owner.catalog.shader_bindings.len();
    let requests = [presentations[0], presentations[1]].map(|presentation| RendererRequest {
        name: b"textures/lit".as_slice(),
        source: profile.source_rules,
        kind: RendererRegistration::Shader {
            mipmaps: true,
            lighting: ShaderLighting::World(Some(presentation)),
        },
        world: Some(world),
    });
    assert!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                requests.each_ref(),
                profile,
                |_| Ok(()),
                |_, _| { Err("reject both lightmap variants".to_owned()) }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert_eq!(
        owner.catalog().shader_binding(handle, profile),
        Some(original)
    );
    assert_eq!(
        owner.catalog.shaders[shader_index].profiles.len(),
        profile_rows
    );
    assert_eq!(owner.catalog.shader_bindings.len(), all_rows);
    let ordinary_request = owner.catalog.shaders[shader_index].requests
        [profile.source_rules as usize]
        .as_ref()
        .unwrap();
    assert_eq!(ordinary_request.name.as_bytes(), ordinary.name);
    assert!(!ordinary_request.mipmaps);
    for request in &requests {
        assert_eq!(
            owner
                .catalog()
                .registration_cached(request.kind, request.name, profile, Some(world)),
            None
        );
        assert!(
            owner
                .catalog()
                .renderer_registrations(request.kind, handle, profile)
                .is_none()
        );
    }
    let (handles, _, counts) = owner
        .prepare_registrations(
            &fixture.vfs,
            requests.each_ref(),
            profile,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(handles, [handle; 2]);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    let mut registrations = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        let registration = owner
            .view()
            .renderer_registrations(request.kind, handle, profile, Some(presentations[index]))
            .unwrap()[0];
        let binding = owner.assets().shader_binding(registration.shader).unwrap();
        assert_eq!(binding.light_source, LightSource::Page(0));
        assert_eq!(binding.lightmap, pages[index]);
        assert_eq!(
            owner.assets().material(binding.material).unwrap().stages[0].texture,
            StageTexture::Lightmap
        );
        registrations.push(registration);
    }
    assert_eq!(registrations[0].key, registrations[1].key);
    assert_ne!(registrations[0].shader, registrations[1].shader);
    assert_eq!(
        owner.catalog().shader_binding(handle, profile),
        Some(original)
    );
    assert_eq!(
        owner.catalog.shaders[shader_index].profiles.len(),
        profile_rows + 2
    );
    let ordinary_request = owner.catalog.shaders[shader_index].requests
        [profile.source_rules as usize]
        .as_ref()
        .unwrap();
    assert_eq!(ordinary_request.name.as_bytes(), ordinary.name);
    assert!(!ordinary_request.mipmaps);
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registrations(
                &fixture.vfs,
                requests.each_ref(),
                profile,
                |_| Ok(()),
                |_, _| { panic!("cached lighting variants reached backend") }
            )
            .unwrap(),
        (handles, (), counts)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let mut wrong_world = requests[0].clone();
    wrong_world.world = None;
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &wrong_world,
                profile,
                |_| Ok(()),
                |_, _| { panic!("missing selected world reached backend") }
            )
            .is_err()
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);

    let vertex = RendererRequest {
        name: b"textures/implicit.tga".as_slice(),
        source: profile.source_rules,
        kind: RendererRegistration::Shader {
            mipmaps: true,
            lighting: ShaderLighting::World(Some(presentations[2])),
        },
        world: Some(world),
    };
    let (vertex_handle, _, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &vertex,
            profile,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registrations(vertex.kind, vertex_handle, profile)
        .unwrap()[0];
    let binding = owner.assets().shader_binding(registration.shader).unwrap();
    assert_eq!(binding.light_source, LightSource::Vertex);
    assert_eq!(binding.lightmap, ImageId(0));
    assert_eq!(
        owner.assets().material(binding.material).unwrap().stages[0].rgb_gen,
        RgbGen::ExactVertex
    );
    assert_eq!(
        owner
            .catalog()
            .shader_handle(vertex.name, profile.source_rules),
        None
    );
    assert_eq!(
        owner
            .view()
            .world_registrations(world, profile, Some(presentations[0]))
            .unwrap(),
        &[]
    );
    let inline = owner.register_inline(world, 1).unwrap();
    assert_eq!(
        owner
            .view()
            .renderer_registrations(
                RendererRegistration::Model,
                inline,
                profile,
                Some(presentations[0])
            )
            .unwrap(),
        &[]
    );

    let missing = RendererRequest {
        name: b"textures/absent".as_slice(),
        ..vertex
    };
    let (public, _, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &missing,
            profile,
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(public, 0);
    let outcome = *owner
        .catalog()
        .shader_outcome(
            missing.name,
            profile,
            ShaderLighting::World(Some(presentations[2])),
        )
        .unwrap();
    assert_eq!(
        outcome.defaulted,
        Some(qa_render::NativeDefaultKind::MissingImage)
    );
    assert_ne!(outcome.shader, qa_render::ShaderBindingId(0));
    assert_eq!(
        owner
            .catalog()
            .shader_outcome(missing.name, profile, ShaderLighting::TwoDimensional),
        Some(&outcome)
    );
    assert_eq!(
        owner
            .catalog()
            .registration_cached(missing.kind, missing.name, profile, Some(world)),
        Some(0)
    );
    assert_eq!(
        owner
            .catalog()
            .shader_handle(missing.name, profile.source_rules),
        Some(0)
    );
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &missing,
                profile,
                |_| Ok(()),
                |_, _| { panic!("cached missing world shader reached backend") }
            )
            .unwrap()
            .0,
        0
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let ordinary_missing = RendererRequest {
        kind: RendererRegistration::Shader {
            mipmaps: false,
            lighting: ShaderLighting::TwoDimensional,
        },
        world: None,
        ..missing
    };
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &ordinary_missing,
                profile,
                |_| Ok(()),
                |_, _| { panic!("wildcard default reached backend") }
            )
            .unwrap()
            .0,
        0
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(
        owner
            .catalog()
            .shader_outcome(missing.name, profile, ShaderLighting::TwoDimensional),
        Some(&outcome)
    );
    assert_eq!(
        owner.catalog().shader_binding(handle, profile),
        Some(original)
    );
}

#[test]
fn embedded_mdl_skin_choices_preserve_distinct_images_in_a_native_view() {
    use qa_render::shader::reference::{ShaderChoice, ShaderContextId, ShaderView};
    let mut bytes = support::mdl();
    bytes[48..52].copy_from_slice(&2i32.to_le_bytes());
    bytes.splice(90..90, [0, 0, 0, 0, 3, 4]);
    let fixture = support::Fixture::new(&[("progs/two.mdl", bytes)]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake3, Some(palette)).profile();
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    let handle = owner
        .register_model(&fixture.vfs, b"progs/two.mdl", profile.source_rules)
        .unwrap();
    let (owner, _cpu) = publish(owner);
    let catalog = owner.catalog();
    let registration = catalog.renderer_registration(handle, profile).unwrap();
    let receipts = catalog
        .renderer_registrations(RendererRegistration::Model, handle, profile)
        .unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].key, receipts[1].key);
    let view = ShaderView {
        native: &(),
        context: Some(ShaderContextId {
            slot: 0,
            generation: 1,
        }),
    };
    let bindings = [0, 1].map(|skin| {
        let choice = catalog
            .bindings(registration, skin, Default::default())
            .unwrap()
            .choices()
            .next()
            .unwrap();
        assert!(matches!(choice.shader, ShaderChoice::Direct(_)));
        let selected = view
            .choice(choice.shader, choice.registrations, None, choice.default)
            .unwrap()
            .source(&())
            .unwrap();
        assert_eq!(selected, choice.physical_binding().unwrap().shader);
        selected
    });
    assert_ne!(bindings[0], bindings[1]);
    for (binding, indices) in bindings.into_iter().zip([[1, 2], [3, 4]]) {
        let material = owner
            .assets()
            .material(owner.assets().shader_binding(binding).unwrap().material)
            .unwrap();
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("embedded skin image");
        };
        assert_eq!(
            owner
                .assets()
                .image(image)
                .unwrap()
                .indexed
                .as_ref()
                .unwrap()
                .mip(0)
                .unwrap()
                .indices(),
            indices
        );
    }
}
