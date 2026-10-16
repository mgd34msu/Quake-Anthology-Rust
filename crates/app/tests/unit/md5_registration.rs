use super::registration_tests::support;
use super::*;
use qa_core::primitives::Vec3;
use qa_render::cpu::{CpuBackend, CpuLimits};

const ANIMATION: &[u8] = br#"MD5Version 10
commandline "paired registration"
numFrames 2
numJoints 1
frameRate 24
numAnimatedComponents 1
hierarchy { "origin" -1 2 0 }
bounds { ( 0 -3 -1 ) ( 0 3 1 ) ( 0 -1 -1 ) ( 0 5 1 ) }
baseframe { ( 0 0 0 ) ( 0 0 0 ) }
frame 1 { 2 }
frame 0 { 0 }
"#;

fn mesh() -> Vec<u8> {
    let surface = |shader: &str, center: f32| {
        let left = center - 1.0;
        let right = center + 1.0;
        format!(
            r#"mesh {{ shader "{shader}"
numverts 4
vert 3 ( 0.5 0 ) 3 1
vert 0 ( 0 0 ) 0 1
vert 2 ( 1 1 ) 2 1
vert 1 ( 0 1 ) 1 1
numtris 1
tri 0 3 1 2
numweights 4
weight 2 0 1 ( 0 {right} -1 )
weight 0 0 1 ( 99 99 99 )
weight 1 0 1 ( 0 {left} -1 )
weight 3 0 1 ( 0 {center} 1 )
}}
"#
        )
    };
    format!(
        "MD5Version 10\ncommandline \"paired registration\"\nnumJoints 1\nnumMeshes 2\njoints {{ \"origin\" -1 ( 0 0 0 ) ( 0 0 0 ) }}\n{}{}",
        surface("textures/first", -1.5),
        surface("textures/second", 1.5),
    )
    .into_bytes()
}

fn files(scales: Option<&[u8]>) -> Vec<(&'static str, Vec<u8>)> {
    let mut files = vec![
        ("models/paired.md5mesh", mesh()),
        ("models/paired.md5anim", ANIMATION.to_vec()),
        ("models/paired.skin", b"mesh0,textures/second\nmesh1,textures/first\n".to_vec()),
        (
            "scripts/paired.shader",
            b"textures/first { cull none { map textures/first.tga rgbGen identity } } textures/second { cull none { map textures/second.tga rgbGen identity } }".to_vec(),
        ),
        (
            "textures/first.tga",
            support::tga(4, 4, &[[211, 17, 29, 255]; 16]),
        ),
        (
            "textures/second.tga",
            support::tga(4, 4, &[[31, 223, 47, 255]; 16]),
        ),
    ];
    if let Some(scales) = scales {
        files.push(("models/paired.md5scale", scales.to_vec()));
    }
    files
}

fn fixture(scales: Option<&[u8]>) -> support::Fixture {
    support::Fixture::new(&files(scales))
}

fn request(profile: ModelProfile) -> RendererRequest<&'static [u8]> {
    RendererRequest {
        name: b"models/paired.md5mesh",
        source: profile.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    }
}

fn bindings(owner: &ModelAssets, profile: ModelProfile) -> Vec<ModelBinding> {
    owner
        .catalog()
        .bindings(
            owner
                .catalog()
                .registration("models/paired.md5mesh", profile)
                .unwrap(),
            0,
            Default::default(),
        )
        .unwrap()
        .collect()
}

fn cpu(owner: &mut ModelAssets) -> CpuBackend {
    let cpu = CpuBackend::load_with_limits(
        64,
        64,
        owner.assets(),
        CpuLimits {
            cache_bytes: 1_048_576,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    cpu
}

fn paint(
    cpu: &mut CpuBackend,
    owner: &ModelAssets,
    selected: impl IntoIterator<Item = ModelBinding>,
    current: u32,
    previous: u32,
    back: f32,
) -> Vec<u32> {
    let limits = qa_render::Limits::default()
        .for_assets(owner.assets())
        .unwrap();
    let mut frontend = qa_render::FrontEnd::load(limits).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    for binding in selected {
        assert!(frame.add_entity(qa_render::SceneEntity {
            model: binding.model.into(),
            shader: Some(binding.shader.into()),
            frame: current,
            old_frame: previous,
            back_lerp: back,
            ..Default::default()
        }));
    }
    assert!(frame.render_scene(
        qa_render::Refdef {
            viewport: qa_render::Viewport {
                width: 64,
                height: 64,
                ..Default::default()
            },
            origin: Vec3([-8., 0., 0.]),
            near: 1.,
            fov: [90.; 2],
            ..Default::default()
        },
        &[],
        owner.assets(),
        ShaderView::direct(),
    ));
    let packet = frame.finish();
    let stats = cpu.render(&packet, owner.assets());
    assert_eq!(stats.rejected, 0);
    assert!(stats.triangles > 0);
    assert!(cpu.inverse_depth().iter().any(|&depth| depth > 0.));
    cpu.pixels().to_vec()
}

#[test]
fn paired_md5_meshes_share_one_skeleton_and_named_receipts_across_profiles() {
    let fixture = fixture(None);
    let first = ModelRequest::native("models/paired.md5mesh", RuleSetId::Quake3, None);
    let mut second = first;
    second.finish.detail_textures = false;
    let before = Assets::load().unwrap();
    let model_count = before.counts().models;
    let mut owner = ModelAssets::load(
        &fixture.vfs,
        before,
        [first, second],
        [],
        Default::default(),
    )
    .unwrap();
    assert!(owner.catalog().diagnostics.is_empty());
    assert_eq!(owner.assets().counts().models, model_count + 2);
    let original = bindings(&owner, first.profile());
    let other = bindings(&owner, second.profile());
    assert_eq!(original.len(), 2);
    assert_eq!(
        original.iter().map(|part| part.model).collect::<Vec<_>>(),
        other.iter().map(|part| part.model).collect::<Vec<_>>()
    );
    let skeleton = owner
        .assets()
        .model(original[0].model)
        .unwrap()
        .skeleton()
        .unwrap();
    assert_eq!((skeleton.frame_count(), skeleton.joint_count()), (2, 1));
    for binding in original.iter().chain(&other) {
        let model = owner.assets().model(binding.model).unwrap();
        assert_eq!((model.frame_count(), model.native_frame_count()), (2, 2));
        assert!(Arc::ptr_eq(skeleton, model.skeleton().unwrap()));
        // Unreferenced vertex zero must not enter the packed skeletal ranges.
        assert_eq!(model.vertices.len(), 3);
        assert!(
            model
                .vertices
                .iter()
                .all(|vertex| vertex.position.0[1].abs() <= 2.5)
        );
    }
    let registration = owner
        .catalog()
        .registration(first.path, first.profile())
        .unwrap();
    let lods = owner
        .catalog()
        .lod_bindings(registration, 0, Default::default())
        .unwrap();
    assert!(!lods.info().supports_lods());
    assert_eq!(lods.native_count(), 1);
    let handle = owner
        .catalog()
        .model_handle(first.path.as_bytes(), first.source_rules)
        .unwrap();
    let receipts = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Model, handle, first.profile())
        .unwrap();
    assert_eq!(receipts.len(), 2);
    assert_ne!(receipts[0].key, receipts[1].key);
    assert_eq!(original[0].shader, receipts[0].shader);
    assert_eq!(original[1].shader, receipts[1].shader);
    assert_eq!(
        owner.catalog().model_bounds(handle),
        Some(Bounds {
            mins: Vec3([0., -3., -1.]),
            maxs: Vec3([0., 5., 1.])
        })
    );
    let skin = owner
        .register_skin(&fixture.vfs, b"models/paired.skin", first.source_rules)
        .unwrap();
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
        .collect::<Vec<_>>();
    assert_eq!(skinned[0].shader, original[1].shader);
    assert_eq!(skinned[1].shader, original[0].shader);
    assert_eq!(skinned[0].model, original[0].model);
    assert_eq!(skinned[1].model, original[1].model);
    let mut cpu = cpu(&mut owner);
    let initial = paint(&mut cpu, &owner, original.clone(), 0, 0, 0.);
    let translated = paint(&mut cpu, &owner, original.clone(), 1, 1, 0.);
    let interpolated = paint(&mut cpu, &owner, original, 1, 0, 0.5);
    assert_ne!(initial, translated);
    assert_ne!(initial, interpolated);
    assert_ne!(translated, interpolated);
    assert_ne!(initial, paint(&mut cpu, &owner, skinned, 0, 0, 0.));
}

#[test]
fn empty_md5_surface_keeps_its_skin_ordinal_and_healthy_sibling_animation() {
    let mut files = files(None);
    let source = files
        .iter_mut()
        .find(|(path, _)| *path == "models/paired.md5mesh")
        .unwrap();
    source.1 = std::str::from_utf8(&source.1)
        .unwrap()
        .replacen("numtris 1\ntri 0 3 1 2", "numtris 0", 1)
        .into_bytes();
    let fixture = support::Fixture::new(&files);
    let request = ModelRequest::native("models/paired.md5mesh", RuleSetId::Quake3, None);
    let mut owner = ModelAssets::load(
        &fixture.vfs,
        Assets::load().unwrap(),
        [request],
        [],
        Default::default(),
    )
    .unwrap();
    let registration = owner
        .catalog()
        .registration(request.path, request.profile())
        .unwrap();
    let original = bindings(&owner, request.profile());
    assert_eq!(original.len(), 2);
    assert!(
        owner
            .assets()
            .model(original[0].model)
            .unwrap()
            .indices
            .is_empty()
    );
    assert_eq!(
        owner
            .assets()
            .model(original[0].model)
            .unwrap()
            .native_frame_count(),
        2
    );
    assert_eq!(
        owner
            .assets()
            .model(original[1].model)
            .unwrap()
            .indices
            .len(),
        3
    );
    let skin = owner
        .register_skin(&fixture.vfs, b"models/paired.skin", request.source_rules)
        .unwrap();
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
        .collect::<Vec<_>>();
    assert_eq!(skinned[1].shader, original[0].shader);
    let mut cpu = cpu(&mut owner);
    let base = paint(&mut cpu, &owner, original.clone(), 0, 0, 0.);
    assert_ne!(base, paint(&mut cpu, &owner, original, 1, 1, 0.));
    assert_ne!(base, paint(&mut cpu, &owner, skinned, 0, 0, 0.));
}

#[test]
fn required_md5_pair_failure_never_publishes_a_bind_pose_only_model() {
    let first = ModelRequest::native("models/paired.md5mesh", RuleSetId::Quake3, None);
    let cases = [None, Some(b"not an animation".as_slice()), Some(ANIMATION)];
    for (index, animation) in cases.into_iter().enumerate() {
        let mut files = vec![("models/paired.md5mesh", mesh())];
        if let Some(animation) = animation {
            let animation = if index == 2 {
                std::str::from_utf8(animation)
                    .unwrap()
                    .replace("\"origin\"", "\"other\"")
                    .into_bytes()
            } else {
                animation.to_vec()
            };
            files.push(("models/paired.md5anim", animation));
        }
        let fixture = support::Fixture::new(&files);
        let mut owner = ModelAssets::new(
            Assets::load().unwrap(),
            [first.profile()],
            [],
            Default::default(),
        )
        .unwrap();
        let mut cpu = cpu(&mut owner);
        let before = owner.assets().counts();
        assert!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &request(first.profile()),
                    first.profile(),
                    RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
                .is_err()
        );
        assert_eq!(owner.assets().counts(), before);
        assert!(
            owner
                .catalog()
                .registration(first.path, first.profile())
                .is_none()
        );
        assert_eq!(
            owner.catalog().registration_cached(
                RendererRegistration::Model,
                first.path.as_bytes(),
                first.profile(),
                None
            ),
            Some(0)
        );
    }
}

#[test]
fn optional_md5_scale_diagnostics_and_retained_pair_survive_reload_rollback() {
    let mut fixture = fixture(Some(br#"{"origin":{"1":2},"unknown":{"1":3}}"#));
    let request = ModelRequest::native("models/paired.md5mesh", RuleSetId::Quake3, None);
    let mut owner = ModelAssets::load(
        &fixture.vfs,
        Assets::load().unwrap(),
        [request],
        [],
        Default::default(),
    )
    .unwrap();
    assert_eq!(owner.catalog().diagnostics.len(), 1);
    assert!(
        owner.catalog().diagnostics[0]
            .error
            .contains("UnknownJoint")
    );
    let original = bindings(&owner, request.profile());
    let skeleton = Arc::clone(
        owner
            .assets()
            .model(original[0].model)
            .unwrap()
            .skeleton()
            .unwrap(),
    );
    let mut cpu = cpu(&mut owner);
    let old_pixels = paint(
        &mut cpu,
        &owner,
        bindings(&owner, request.profile()),
        1,
        0,
        0.5,
    );
    for path in [
        "models/paired.md5mesh",
        "models/paired.md5anim",
        "models/paired.md5scale",
    ] {
        fixture.remove(path);
    }
    let (mount, path) = {
        let mount = fixture.vfs.mounts().next().unwrap();
        (mount.id, mount.path.clone())
    };
    assert!(fixture.vfs.unmount(mount));
    fixture.vfs.mount_directory(&path, 0).unwrap();
    let mut finish = request.finish;
    finish.vertex_light = true;
    let original_prefix = owner.assets().counts();
    assert!(
        owner
            .prepare_profile(
                &fixture.vfs,
                request.source_rules,
                request.rules,
                request.image_settings,
                finish,
                |_| Err("reject paired profile".to_owned()),
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), original_prefix);
    assert_eq!(owner.catalog().profiles().count(), 1);
    let (new_profile, _) = owner
        .prepare_profile(
            &fixture.vfs,
            request.source_rules,
            request.rules,
            request.image_settings,
            finish,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let native = RendererRequest {
        name: request.path.as_bytes(),
        source: request.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    };
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &native,
                new_profile,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| Err("reject paired material".to_owned()),
            )
            .is_err()
    );
    owner
        .prepare_registration(
            &fixture.vfs,
            &native,
            new_profile,
            RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(
        bindings(&owner, new_profile)
            .iter()
            .map(|binding| binding.model)
            .collect::<Vec<_>>(),
        original
            .iter()
            .map(|binding| binding.model)
            .collect::<Vec<_>>()
    );
    assert_eq!(owner.assets().counts().models, original_prefix.models);
    let prefix = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    for reject in [true, false, false] {
        let result =
            owner.image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
                owner.refresh_images(
                    &fixture.vfs,
                    &[request.profile()],
                    &[],
                    [None; 5],
                    request.finish,
                    false,
                    |_| Ok(()),
                )?;
                let mut candidate =
                    CpuBackend::load_with_assets(64, 64, owner.assets()).map_err(str::to_owned)?;
                assert_eq!(
                    paint(
                        &mut candidate,
                        owner,
                        bindings(owner, request.profile()),
                        1,
                        0,
                        0.5
                    ),
                    old_pixels
                );
                if reject {
                    return Err("reject prepared paired model".to_owned());
                }
                cpu = candidate;
                Ok(())
            });
        assert_eq!(result.is_err(), reject);
        assert_eq!(owner.assets().counts().models, prefix.models);
        assert_eq!(owner.catalog().capacity(), capacity);
        assert_eq!(
            bindings(&owner, request.profile())
                .iter()
                .map(|binding| binding.model)
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|binding| binding.model)
                .collect::<Vec<_>>()
        );
        assert!(Arc::ptr_eq(
            &skeleton,
            owner
                .assets()
                .model(original[0].model)
                .unwrap()
                .skeleton()
                .unwrap()
        ));
        assert_eq!(
            paint(
                &mut cpu,
                &owner,
                bindings(&owner, request.profile()),
                1,
                0,
                0.5
            ),
            old_pixels
        );
    }
}
