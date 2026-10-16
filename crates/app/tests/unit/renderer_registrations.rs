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
    let kind = RendererRegistration::Shader { mipmaps: false };
    let first = request(b"textures/first.tga", profile.source_rules, kind);
    let second = request(b"textures/second.tga", profile.source_rules, kind);
    let prepares = Cell::new(0);
    let (handles, counts) = owner
        .prepare_registrations(&fixture.vfs, [&first, &second], profile, |assets| {
            prepares.set(prepares.get() + 1);
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
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
    }
    let capacity = owner.catalog().capacity();
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registrations(&fixture.vfs, [&first, &second], profile, |_| {
                panic!("cached pair reached backend")
            })
            .unwrap(),
        (handles, counts)
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
            .register_shader(&fixture.vfs, name, peer.source_rules, false)
            .unwrap()
    });
    let retained = handles.map(|handle| owner.catalog().shader_binding(handle, peer).unwrap());
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let kind = RendererRegistration::Shader { mipmaps: false };
    let requests = names.map(|name| request(name, target.source_rules, kind));
    assert!(
        owner
            .prepare_registrations(&fixture.vfs, requests.each_ref(), target, |_| {
                Err("backend rejected both candidates".to_owned())
            })
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
    let retry_kind = RendererRegistration::Shader { mipmaps: true };
    let retries = names.map(|name| request(name, target.source_rules, retry_kind));
    let (retried, counts) = owner
        .prepare_registrations(&fixture.vfs, retries.each_ref(), target, |assets| {
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
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
            owner.assets().material(material).unwrap().stages[0]
                .sampler
                .mipmaps
        );
        assert!(
            !owner.assets().material(retained[index]).unwrap().stages[0]
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
        )
        .unwrap();
    let retained_material = owner.catalog().shader_binding(retained, peer).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let kind = RendererRegistration::Shader { mipmaps: false };
    let first = request(b"textures/first.tga", profile.source_rules, kind);
    let missing = request(b"textures/missing.tga", profile.source_rules, kind);
    assert!(
        owner
            .prepare_registrations(&fixture.vfs, [&first, &missing], profile, |_| {
                panic!("failed second resource reached backend")
            })
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
    let first_name = owner.catalog.names.find(shader_key(first.name)).unwrap();
    let capacity = owner.catalog().capacity();
    let diagnostics = owner.catalog.diagnostics.len();
    fixture.vfs.take_lookup_count();
    for _ in 0..2 {
        assert!(
            owner
                .prepare_registrations(&fixture.vfs, [&first, &missing], profile, |_| {
                    panic!("cached failed member reached backend")
                })
                .is_err()
        );
        assert_eq!(
            owner
                .prepare_registration(&fixture.vfs, &missing, profile, |_| {
                    panic!("cached negative single reached backend")
                })
                .unwrap(),
            (0, before)
        );
    }
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(owner.catalog.diagnostics.len(), diagnostics);
    let retry = request(
        first.name,
        profile.source_rules,
        RendererRegistration::Shader { mipmaps: true },
    );
    let (handle, counts) = owner
        .prepare_registration(&fixture.vfs, &retry, profile, |assets| {
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    assert_eq!(handle, retained);
    assert_eq!(
        owner.catalog.names.find(shader_key(first.name)),
        Some(first_name)
    );
    let material = owner.catalog().shader_binding(handle, profile).unwrap();
    assert!(
        owner.assets().material(material).unwrap().stages[0]
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
        )
        .unwrap();
    let retained = owner.catalog().shader_binding(handle, peer).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let before = owner.assets().counts();
    let first = request(
        b"textures/first.tga",
        target.source_rules,
        RendererRegistration::Shader { mipmaps: false },
    );
    let duplicate = request(
        b"TEXTURES/FIRST.jpg",
        target.source_rules,
        RendererRegistration::Shader { mipmaps: true },
    );
    assert!(
        owner
            .prepare_registrations(&fixture.vfs, [&first, &duplicate], target, |_| {
                Err("reject shared candidate".to_owned())
            })
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().shader_binding(handle, peer), Some(retained));
    let (handles, counts) = owner
        .prepare_registrations(&fixture.vfs, [&first, &duplicate], target, |assets| {
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    assert_eq!(handles, [handle, handle]);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    let material = owner.catalog().shader_binding(handle, target).unwrap();
    assert!(
        !owner.assets().material(material).unwrap().stages[0]
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
        .register_shader(&fixture.vfs, b"textures/body", profile.source_rules, false)
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
                .prepare_registrations(&fixture.vfs, [&model, &skin], profile, |_| {
                    Err("reject appended model and skin".to_owned())
                })
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
    let (handles, counts) = owner
        .prepare_registrations(&fixture.vfs, [&model, &skin], profile, |assets| {
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    assert_eq!(Some(NameId(handles[0] as u32)), model_name);
    assert!(
        owner
            .catalog()
            .renderer_registration(handles[0], profile)
            .is_some()
    );
    assert!(owner.catalog().renderer_skin(handles[1], profile));
    assert_eq!(owner.catalog.names.find(skin.name), skin_name);
    assert_eq!(
        owner.catalog().shader_binding(retained, profile),
        Some(retained_material)
    );
    assert_eq!(cpu.admitted_assets(), Some(counts));
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registrations(&fixture.vfs, [&model, &skin], profile, |_| {
                panic!("cached model/skin pair reached backend")
            })
            .unwrap(),
        (handles, counts)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}
