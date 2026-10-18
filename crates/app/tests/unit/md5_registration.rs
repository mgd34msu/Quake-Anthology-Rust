#[path = "model_colors.rs"]
mod model_colors;

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
        search: None,
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
                .registration("models/paired.md5mesh", profile, None)
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
        .registration(first.path, first.profile(), None)
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
        .registration(request.path, request.profile(), None)
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
            &fixture.vfs,
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
                .registration(first.path, first.profile(), None)
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
                None,
                |_| Err("reject paired profile".to_owned())
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
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let native = RendererRequest {
        search: None,
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

fn replacement_files(rules: RuleSetId) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    if rules == RuleSetId::Quake {
        entries.push((
            "models/paired.mdl".to_owned(),
            super::registration_tests::interleaved_mdl_frames(),
        ));
        entries.push(("models/paired.md5mesh".to_owned(), mesh()));
        entries.push(("models/paired.md5anim".to_owned(), ANIMATION.to_vec()));
        for (mesh, shader) in ["first", "second"].into_iter().enumerate() {
            for (group, count) in [1u8, 2, 3, 6].into_iter().enumerate() {
                for member in 0..count {
                    let index = 20 + mesh as u8 * 40 + group as u8 * 6 + member;
                    entries.push((
                        format!("progs/textures/{shader}_{group:02}_{member:02}.lmp"),
                        [
                            2u32.to_le_bytes().as_slice(),
                            &1u32.to_le_bytes(),
                            &[index; 2],
                        ]
                        .concat(),
                    ));
                }
            }
        }
    } else {
        let mut original = support::md2();
        let skin = b"MoDeLs\\TeSt\\SkIn1.PcX";
        original[132..196].fill(0);
        original[132..132 + skin.len()].copy_from_slice(skin);
        entries.extend([
            ("models/test/tris.md2".to_owned(), original),
            ("models/test/skin0.pcx".to_owned(), support::pcx(11)),
            ("models/test/skin1.pcx".to_owned(), support::pcx(22)),
            ("models/test/md5/tris.md5mesh".to_owned(), mesh()),
            (
                "models/test/md5/tris.md5anim".to_owned(),
                ANIMATION.to_vec(),
            ),
            ("models/test/md5/skin0.pcx".to_owned(), support::pcx(71)),
            ("models/test/md5/skin1.pcx".to_owned(), support::pcx(83)),
        ]);
    }
    entries
}

fn from_owned_files(entries: &[(String, Vec<u8>)]) -> support::Fixture {
    support::Fixture::new(
        &entries
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect::<Vec<_>>(),
    )
}

fn replacement_profile(assets: &mut Assets, rules: RuleSetId) -> ModelProfile {
    ModelRequest::native("", rules, Some(support::palette(assets))).profile()
}

fn replacement_owner(vfs: &Vfs, assets: Assets, profile: ModelProfile) -> ModelAssets {
    let mut owner = ModelAssets::new(vfs, assets, [profile], [], Default::default()).unwrap();
    owner.set_image_search_order(ImageSearchOrder::parse(b"").unwrap());
    owner
        .adopt_palettes(
            crate::render_settings::PaletteRegistrations::from_registered(
                vfs,
                [profile.source_palette; 5],
            ),
        )
        .unwrap();
    owner
}

fn captured_replacement(
    owner: &ModelAssets,
    handle: i32,
    profile: ModelProfile,
    skin: u32,
    pose: qa_render::SceneEntity,
    mut view: qa_render::Refdef,
) -> qa_render::scene::CommandList {
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let selected = owner
        .catalog()
        .lod_bindings(
            registration,
            skin,
            ModelMaterialPolicy {
                fullbright: true,
                ..Default::default()
            },
        )
        .unwrap();
    let mut frontend = qa_render::FrontEnd::load(
        qa_render::Limits::default()
            .for_assets(owner.assets())
            .unwrap()
            .for_model_choices(owner.catalog().max_choice_nodes())
            .unwrap(),
    )
    .unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(selected.capture(&mut frame, pose, ShaderView::direct(), None, None));
    view.viewport = qa_render::Viewport {
        width: 64,
        height: 64,
        ..Default::default()
    };
    view.origin = Vec3([-8., 0., 0.]);
    view.near = 1.;
    view.fov = [90.; 2];
    assert!(frame.render_scene(view, &[], owner.assets(), ShaderView::direct()));
    frame.finish()
}

fn captured_draws(packet: &qa_render::scene::CommandList) -> &[qa_render::scene::DrawItem] {
    let view = packet
        .commands()
        .iter()
        .find_map(|command| match command {
            qa_render::scene::Command::View(view) => Some(view),
            _ => None,
        })
        .unwrap();
    packet.draws(view.scene.draws)
}

#[test]
fn native_mdl_replacement_retains_parent_groups_and_requires_every_mesh_skin_member() {
    use qa_render::scene::{FrameTiming, SkeletalFrameMap};
    let files = replacement_files(RuleSetId::Quake);
    let fixture = from_owned_files(&files);
    let mut assets = Assets::load().unwrap();
    let profile = replacement_profile(&mut assets, RuleSetId::Quake);
    let mut owner = replacement_owner(&fixture.vfs, assets, profile);
    let handle = owner
        .register_model(&fixture.vfs, b"models/paired.mdl", profile.source_rules)
        .unwrap();
    let mut raster = cpu(&mut owner);
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let lods = owner
        .catalog()
        .lod_bindings(registration, 3, Default::default())
        .unwrap();
    assert_eq!(lods.base_frames(), 7);
    assert_eq!(lods.base().unwrap().len(), 1);
    assert_eq!(lods.len(), 2);
    let (replacement, mapping, rules) = lods.replacement().unwrap();
    assert_eq!(mapping, Some(SkeletalFrameMap::Elapsed { rate: 2. }));
    assert_eq!(rules, RuleSetId::Quake);
    assert_eq!(replacement.len(), 2);
    let native = lods.base().unwrap().choices().next().unwrap();
    let native_model = owner.assets().model(native.model).unwrap();
    assert_eq!(
        (
            native_model.native_frame_count(),
            native_model.frame_count()
        ),
        (4, 7)
    );
    assert_eq!(native_model.native_skin_count(), 4);
    assert_eq!(
        lods.info().frame_count(native_model.native_frame_count()),
        4
    );
    assert_eq!(
        owner
            .catalog()
            .metadata(b"models/paired.mdl", None)
            .unwrap()
            .format,
        ModelFormat::Mdl
    );
    assert!(
        owner
            .catalog()
            .model_handle(b"models/paired.md5mesh", profile.source_rules)
            .is_none()
    );
    let replacement_meshes = replacement
        .choices()
        .map(|part| part.model)
        .collect::<Vec<_>>();
    for timing in [FrameTiming::Cumulative, FrameTiming::UniformFirst] {
        let packet = captured_replacement(
            &owner,
            handle,
            profile,
            3,
            qa_render::SceneEntity {
                frame: 1,
                old_frame: 3,
                back_lerp: 0.75,
                sync_base: 0.125,
                ..Default::default()
            },
            qa_render::Refdef {
                model_time: Some(qa_core::primitives::ThinkTime::Seconds(0.125)),
                frame_timing: timing,
                ..Default::default()
            },
        );
        let draws = captured_draws(&packet);
        assert_eq!(draws.len(), 2);
        for draw in draws {
            assert!(replacement_meshes.contains(&draw.mesh.unwrap()));
            assert_eq!(
                draw.frames,
                Some(qa_render::scene::ModelFramePose {
                    current: 0,
                    previous: 0,
                    back_lerp: 0.
                })
            );
        }
        assert_eq!(raster.render(&packet, owner.assets()).rejected, 0);
        assert!(raster.inverse_depth().iter().any(|&depth| depth > 0.));
    }
    let fallback = owner
        .catalog()
        .lod_bindings(registration, u32::MAX, Default::default())
        .unwrap();
    assert_eq!(
        fallback
            .replacement()
            .unwrap()
            .0
            .choices()
            .next()
            .unwrap()
            .shaders()
            .count(),
        1
    );
    let mut incomplete = files;
    incomplete.retain(|(path, _)| path != "progs/textures/second_03_05.lmp");
    let fixture = from_owned_files(&incomplete);
    let mut assets = Assets::load().unwrap();
    let profile = replacement_profile(&mut assets, RuleSetId::Quake);
    let mut owner = replacement_owner(&fixture.vfs, assets, profile);
    let handle = owner
        .register_model(&fixture.vfs, b"models/paired.mdl", profile.source_rules)
        .unwrap();
    let _cpu = cpu(&mut owner);
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let lods = owner
        .catalog()
        .lod_bindings(registration, 0, Default::default())
        .unwrap();
    assert!(lods.replacement().is_none());
    assert_eq!(lods.base().unwrap().len(), 1);
}

#[test]
fn native_md2_replacement_uses_inserted_skin_directory_and_one_native_public_handle() {
    use qa_render::scene::{ModelFramePose, SkeletalFrameMap};
    let fixture = from_owned_files(&replacement_files(RuleSetId::Quake2));
    let mut assets = Assets::load().unwrap();
    let profile = replacement_profile(&mut assets, RuleSetId::Quake2);
    let mut owner = replacement_owner(&fixture.vfs, assets, profile);
    let handle = owner
        .register_model(&fixture.vfs, b"models/test/tris.md2", profile.source_rules)
        .unwrap();
    let mut cpu = cpu(&mut owner);
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let lods = owner
        .catalog()
        .lod_bindings(registration, 1, Default::default())
        .unwrap();
    assert_eq!(lods.base().unwrap().len(), 1);
    assert_eq!(lods.len(), 2);
    let (replacement, mapping, rules) = lods.replacement().unwrap();
    assert_eq!(
        (mapping, rules),
        (Some(SkeletalFrameMap::Wrap), RuleSetId::Quake2)
    );
    assert_eq!(replacement.len(), 2);
    assert!(
        owner
            .catalog()
            .model_handle(b"models/test/md5/tris.md5mesh", profile.source_rules)
            .is_none()
    );
    assert_eq!(
        owner
            .catalog()
            .metadata(b"models/test/tris.md2", None)
            .unwrap()
            .format,
        ModelFormat::Md2
    );
    let receipt = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Model, handle, profile)
        .unwrap();
    let names = receipt
        .iter()
        .map(|entry| owner.assets().name(entry.key).unwrap())
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| *name == "models/test/md5/skin1"));
    assert!(!names.iter().any(|name| *name == "textures/first"));
    let mut pixels = Vec::new();
    for skin in [0, 1] {
        let packet = captured_replacement(
            &owner,
            handle,
            profile,
            skin,
            qa_render::SceneEntity {
                frame: 1,
                old_frame: 0,
                back_lerp: 0.25,
                ..Default::default()
            },
            qa_render::Refdef::default(),
        );
        assert_eq!(captured_draws(&packet).len(), 2);
        assert!(captured_draws(&packet).iter().all(|draw| draw.frames
            == Some(ModelFramePose {
                current: 1,
                previous: 0,
                back_lerp: 0.25
            })));
        assert_eq!(cpu.render(&packet, owner.assets()).rejected, 0);
        assert!(cpu.inverse_depth().iter().any(|&depth| depth > 0.));
        pixels.push(cpu.pixels().to_vec());
    }
    assert_ne!(pixels[0], pixels[1]);
}

#[test]
fn optional_replacement_pair_failure_and_load_controls_keep_native_registration_healthy() {
    let mut native_prefix = None;
    for failure in [
        "disabled",
        "missing",
        "malformed",
        "surface_names",
        "skin_image",
    ] {
        let mut files = replacement_files(RuleSetId::Quake2);
        if failure == "missing" {
            files.retain(|(path, _)| path != "models/test/md5/tris.md5anim");
        } else if failure == "malformed" {
            files
                .iter_mut()
                .find(|(path, _)| path == "models/test/md5/tris.md5mesh")
                .unwrap()
                .1 = b"not an MD5 model".to_vec();
        } else if failure == "skin_image" {
            files
                .iter_mut()
                .find(|(path, _)| path == "models/test/md5/skin1.pcx")
                .unwrap()
                .1 = b"invalid PCX image".to_vec();
        }
        let fixture = from_owned_files(&files);
        let mut assets = Assets::load().unwrap();
        let profile = replacement_profile(&mut assets, RuleSetId::Quake2);
        let mut owner = if failure == "surface_names" {
            let mut owner = ModelAssets::new(
                &fixture.vfs,
                assets,
                [profile],
                [],
                ModelLimits {
                    name_bytes: [
                        "models/test/tris.md2",
                        "models/test/md5/tris.md5mesh",
                        "models/test/md5/skin0.pcx",
                        "models/test/md5/skin1.pcx",
                    ]
                    .iter()
                    .map(|path| path.len())
                    .sum(),
                    ..Default::default()
                },
            )
            .unwrap();
            owner.set_image_search_order(ImageSearchOrder::parse(b"").unwrap());
            owner
                .adopt_palettes(
                    crate::render_settings::PaletteRegistrations::from_registered(
                        &fixture.vfs,
                        [profile.source_palette; 5],
                    ),
                )
                .unwrap();
            owner
        } else {
            replacement_owner(&fixture.vfs, assets, profile)
        };
        if failure == "disabled" {
            owner.set_model_replacement_load(true, false);
        }
        let handle = owner
            .register_model(&fixture.vfs, b"models/test/tris.md2", profile.source_rules)
            .unwrap();
        let _cpu = cpu(&mut owner);
        let registration = owner
            .catalog()
            .renderer_registration(handle, profile)
            .unwrap();
        let lods = owner
            .catalog()
            .lod_bindings(registration, 0, Default::default())
            .unwrap();
        assert_eq!(lods.base().unwrap().len(), 1, "{failure}");
        assert!(lods.replacement().is_none(), "{failure}");
        assert_eq!(
            owner
                .catalog()
                .metadata(b"models/test/tris.md2", None)
                .unwrap()
                .format,
            ModelFormat::Md2
        );
        let capacity = owner.catalog().capacity();
        let prefix = (
            owner.assets().counts(),
            capacity.surfaces,
            capacity.bindings,
            capacity.frame_vertices,
            capacity.indices,
        );
        assert_eq!(prefix, *native_prefix.get_or_insert(prefix), "{failure}");
        if failure == "surface_names" {
            assert!(
                owner
                    .catalog()
                    .diagnostics
                    .iter()
                    .any(|entry| entry.error.contains("model surface names"))
            );
        } else if failure == "skin_image" {
            assert!(
                owner
                    .catalog()
                    .diagnostics
                    .iter()
                    .any(|entry| entry.error.contains("replacement material"))
            );
        }
        fixture.vfs.take_lookup_count();
        assert_eq!(
            owner
                .register_model(&fixture.vfs, b"models/test/tris.md2", profile.source_rules)
                .unwrap(),
            handle
        );
        assert_eq!(fixture.vfs.take_lookup_count(), 0, "{failure}");
    }
}

#[test]
fn replacement_discovery_compares_actual_mount_precedence_without_public_candidate_handles() {
    for priority in [-1, 0, 1] {
        let files = replacement_files(RuleSetId::Quake2);
        let (replacement, original): (Vec<_>, Vec<_>) = files
            .into_iter()
            .partition(|(path, _)| path.contains("/md5/"));
        let mut fixture = from_owned_files(&original);
        let candidate = from_owned_files(&replacement);
        let candidate_path = candidate.vfs.mounts().next().unwrap().path.clone();
        fixture
            .vfs
            .mount_directory(&candidate_path, priority)
            .unwrap();
        let mut assets = Assets::load().unwrap();
        let profile = replacement_profile(&mut assets, RuleSetId::Quake2);
        let mut owner = replacement_owner(&fixture.vfs, assets, profile);
        let handle = owner
            .register_model(&fixture.vfs, b"models/test/tris.md2", profile.source_rules)
            .unwrap();
        let _cpu = cpu(&mut owner);
        let registration = owner
            .catalog()
            .renderer_registration(handle, profile)
            .unwrap();
        let lods = owner
            .catalog()
            .lod_bindings(registration, 0, Default::default())
            .unwrap();
        assert_eq!(
            lods.replacement().is_some(),
            priority >= 0,
            "priority={priority}"
        );
        assert_eq!(lods.base().unwrap().len(), 1);
        assert!(
            owner
                .catalog()
                .model_handle(b"models/test/md5/tris.md5mesh", profile.source_rules)
                .is_none()
        );
    }
}

#[test]
fn retained_native_and_replacement_sources_survive_profile_reload_and_late_rejection() {
    let mut fixture = from_owned_files(&replacement_files(RuleSetId::Quake2));
    let mut assets = Assets::load().unwrap();
    let profile = replacement_profile(&mut assets, RuleSetId::Quake2);
    let mut owner = replacement_owner(&fixture.vfs, assets, profile);
    let handle = owner
        .register_model(&fixture.vfs, b"models/test/tris.md2", profile.source_rules)
        .unwrap();
    let mut raster = cpu(&mut owner);
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let lods = owner
        .catalog()
        .lod_bindings(registration, 0, Default::default())
        .unwrap();
    let native_mesh = lods.base().unwrap().choices().next().unwrap().model;
    let replacement_meshes = lods
        .replacement()
        .unwrap()
        .0
        .choices()
        .map(|part| part.model)
        .collect::<Vec<_>>();
    let skeleton = Arc::clone(
        owner
            .assets()
            .model(replacement_meshes[0])
            .unwrap()
            .skeleton()
            .unwrap(),
    );
    let packet = captured_replacement(
        &owner,
        handle,
        profile,
        0,
        Default::default(),
        Default::default(),
    );
    assert_eq!(raster.render(&packet, owner.assets()).rejected, 0);
    let pixels = raster.pixels().to_vec();
    let metadata = owner
        .catalog()
        .metadata(b"models/test/tris.md2", None)
        .unwrap();
    for path in [
        "models/test/tris.md2",
        "models/test/md5/tris.md5mesh",
        "models/test/md5/tris.md5anim",
    ] {
        fixture.remove(path);
    }
    let (mount, path) = {
        let mount = fixture.vfs.mounts().next().unwrap();
        (mount.id, mount.path.clone())
    };
    assert!(fixture.vfs.unmount(mount));
    fixture.vfs.mount_directory(&path, 0).unwrap();
    let mut finish = profile.finish;
    finish.detail_textures = false;
    let (next, _) = owner
        .prepare_profile(
            &fixture.vfs,
            profile.source_rules,
            profile.rules,
            profile.image_settings,
            finish,
            None,
            |assets| raster.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = RendererRequest {
        search: None,
        name: b"models/test/tris.md2".as_slice(),
        source: profile.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    };
    let prefix = owner.assets().counts();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                next,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| {
                    let _candidate =
                        CpuBackend::load_with_assets(64, 64, assets).map_err(str::to_owned)?;
                    Err("reject staged native replacement".to_owned())
                }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(
        owner.catalog().renderer_registration(handle, profile),
        Some(registration)
    );
    owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            next,
            RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| {
                raster = CpuBackend::load_with_assets(64, 64, assets).map_err(str::to_owned)?;
                Ok(())
            },
        )
        .unwrap();
    let next_registration = owner.catalog().renderer_registration(handle, next).unwrap();
    let next_lods = owner
        .catalog()
        .lod_bindings(next_registration, 0, Default::default())
        .unwrap();
    assert_eq!(
        next_lods.base().unwrap().choices().next().unwrap().model,
        native_mesh
    );
    assert_eq!(
        next_lods
            .replacement()
            .unwrap()
            .0
            .choices()
            .map(|part| part.model)
            .collect::<Vec<_>>(),
        replacement_meshes
    );
    assert_eq!(owner.assets().counts().models, prefix.models);
    let prefix = owner.assets().counts();
    for reject in [true, false] {
        let result =
            owner.image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
                let refresh = owner.refresh_images(
                    &fixture.vfs,
                    &[profile, next],
                    &[],
                    [None; 5],
                    finish,
                    false,
                    |_| Ok(()),
                )?;
                assert_eq!(refresh.profile(next), next);
                assert_eq!(
                    refresh.registration(next_registration, owner.catalog())?,
                    next_registration
                );
                let mut candidate =
                    CpuBackend::load_with_assets(64, 64, owner.assets()).map_err(str::to_owned)?;
                let packet = captured_replacement(
                    owner,
                    handle,
                    next,
                    0,
                    Default::default(),
                    Default::default(),
                );
                assert_eq!(candidate.render(&packet, owner.assets()).rejected, 0);
                assert_eq!(candidate.pixels(), pixels);
                if reject {
                    return Err("reject published replacement reload".to_owned());
                }
                raster = candidate;
                Ok(())
            });
        assert_eq!(result.is_err(), reject);
        if reject {
            assert_eq!(owner.assets().counts(), prefix);
        }
        assert_eq!(owner.assets().counts().models, prefix.models);
        assert_eq!(
            owner
                .catalog()
                .model_handle(b"models/test/tris.md2", profile.source_rules),
            Some(handle)
        );
        assert_eq!(
            owner.catalog().metadata(b"models/test/tris.md2", None),
            Some(metadata)
        );
        assert!(Arc::ptr_eq(
            &skeleton,
            owner
                .assets()
                .model(replacement_meshes[0])
                .unwrap()
                .skeleton()
                .unwrap()
        ));
        let packet = captured_replacement(
            &owner,
            handle,
            next,
            0,
            Default::default(),
            Default::default(),
        );
        assert_eq!(raster.render(&packet, owner.assets()).rejected, 0);
        assert_eq!(raster.pixels(), pixels);
    }
}

#[test]
fn selected_md5_source_keeps_animation_and_scale_companions_together() {
    let mut files = fixture(Some(b"origin 2"));
    let animation = std::str::from_utf8(ANIMATION)
        .unwrap()
        .replace("( 0 5 1 )", "( 0 9 1 )");
    let high = support::Fixture::new(&[
        ("models/paired.md5mesh", mesh()),
        ("models/paired.md5anim", animation.as_bytes().to_vec()),
        ("models/paired.md5scale", b"origin 3".to_vec()),
    ]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_root = high.vfs.mounts().next().unwrap().path.clone();
    let high_mount = files.vfs.mount_directory(&high_root, 10).unwrap();
    let mut owner = ModelAssets::new(
        &files.vfs,
        Assets::load().unwrap(),
        [],
        [],
        Default::default(),
    )
    .unwrap();
    let mut ids = Vec::new();
    let mut handles = Vec::new();
    for (mount, expected_animation, expected_scale, max_y) in [
        (low_mount, ANIMATION, b"origin 2".as_slice(), 5.0),
        (
            high_mount,
            animation.as_bytes(),
            b"origin 3".as_slice(),
            9.0,
        ),
    ] {
        let view = files.vfs.search_view([mount]).unwrap();
        let source = owner
            .model_source(files.vfs.read_view(Some(&view)), "models/paired.md5mesh")
            .map_err(|e| e.message)
            .unwrap()
            .unwrap();
        let ModelSourceFile::Ready {
            skeletal: Some(skeletal),
            metadata,
            ..
        } = &owner.catalog.sources[source]
        else {
            panic!("paired MD5 source");
        };
        assert_eq!(skeletal._animation_bytes.as_ref(), expected_animation);
        assert_eq!(skeletal._scale_bytes.as_deref(), Some(expected_scale));
        assert_eq!(metadata.bounds.maxs.0[1], max_y);
        assert_eq!(skeletal.skeleton.frame_count(), 2);
        let name = owner.catalog.sources.name(source).unwrap();
        let file = owner
            .catalog
            .files
            .admit(Some((name, view.clone())))
            .unwrap();
        owner.catalog.files[file] = ModelFile::Ready {
            lods: ModelLods {
                slots: [
                    ModelLodSlot::Source(source),
                    ModelLodSlot::Missing,
                    ModelLodSlot::Missing,
                ],
                native_count: 1,
                supports_lods: false,
            },
            renderer_sources: 1 << RuleSetId::Quake3 as u8,
        };
        assert_eq!(
            file_lookup(
                &owner.catalog.names,
                &owner.catalog.files,
                b"models/paired.md5mesh",
                Some(&view)
            )
            .unwrap()
            .bounds(&owner.catalog.sources)
            .unwrap()
            .maxs
            .0[1],
            max_y
        );
        assert_eq!(
            owner
                .catalog
                .files
                .admit(Some((name, view.clone())))
                .unwrap(),
            file
        );
        handles.push((view, file, max_y));
        ids.push(source);
    }
    assert_ne!(ids[0], ids[1]);
    assert_ne!(handles[0].1, handles[1].1);
    for (view, file, max_y) in &handles {
        let name = owner.catalog.sources.name(ids[0]).unwrap();
        assert_eq!(owner.catalog.files.find(view, name), Some(*file));
        assert_eq!(
            owner.catalog.files[*file]
                .bounds(&owner.catalog.sources)
                .unwrap()
                .maxs
                .0[1],
            *max_y
        );
    }
    let name = owner.catalog.sources.name(ids[0]).unwrap();
    assert_eq!(
        owner.catalog.files.find(&files.vfs.default_view(), name),
        None
    );
    for (_, file, _) in &handles {
        owner.catalog.files[*file] =
            ModelFile::Failed(FileError::file("source unavailable".to_owned()));
    }
    assert!(owner.retry_published_file(b"models/paired.md5mesh"));
    for (_, file, _) in &handles {
        assert!(matches!(owner.catalog.files[*file], ModelFile::Empty));
    }
    assert!(!owner.retry_published_file(b"models/paired.md5mesh"));
    for (view, file, max_y) in &handles {
        assert!(
            owner
                .catalog
                .files
                .source_view(*file)
                .unwrap()
                .same_selection(view)
        );
        let bounds = owner
            .ensure_file_record(&files.vfs, *file, None, false)
            .map_err(|error| error.message)
            .unwrap();
        assert_eq!(bounds.maxs.0[1], *max_y);
        assert_eq!(
            owner.catalog.files.name(*file),
            owner.catalog.sources.name(ids[0])
        );
    }
}
