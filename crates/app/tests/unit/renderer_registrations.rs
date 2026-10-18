use super::*;
use qa_render::cpu::{CpuBackend, CpuLimits};
use std::cell::Cell;

#[allow(dead_code)]
#[path = "../model_assets/support.rs"]
pub(super) mod support;

fn profile(source: RuleSetId) -> ModelProfile {
    ModelRequest::native("", source, None).profile()
}

fn request(name: &[u8], source: RuleSetId, kind: RendererRegistration) -> RendererRequest<&[u8]> {
    RendererRequest {
        search: None,
        name,
        source,
        kind,
        world: None,
    }
}

pub(super) fn publish(mut owner: ModelAssets) -> (ModelAssets, CpuBackend) {
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

pub(super) fn paint_shader(
    cpu: &mut CpuBackend,
    assets: &Assets,
    shader: ShaderBindingId,
) -> Vec<u32> {
    let mut front = qa_render::FrontEnd::load(Default::default()).unwrap();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.draw_2d(qa_render::Draw2d {
        rect: [0.0, 0.0, 8.0, 8.0],
        texcoords: [0.0, 0.0, 1.0, 1.0],
        color: [255; 4],
        shader: shader.into(),
    }));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 0);
    assert_eq!(cpu.render(&packet, assets).rejected, 0);
    cpu.pixels().to_vec()
}

#[test]
fn same_profile_reload_reselects_formats_and_rolls_back_late_rejection_without_derived_growth() {
    let jpeg = include_bytes!("../../../render/tests/resources/blue.jpg");
    let mut fixture = support::Fixture::new(&[
        ("models/test.md3", support::md3_surfaces()),
        (
            "models/test.skin",
            b"body,textures/body\nglow,textures/glow\n".to_vec(),
        ),
        (
            "textures/body.tga",
            support::tga(4, 4, &[[211, 17, 29, 255]; 16]),
        ),
        (
            "textures/alternate.tga",
            support::tga(4, 4, &[[31, 223, 47, 255]; 16]),
        ),
        (
            "textures/glow.tga",
            support::tga(4, 4, &[[53, 67, 229, 255]; 16]),
        ),
        ("textures/body.jpeg", jpeg.to_vec()),
        ("textures/alternate.jpeg", jpeg.to_vec()),
        ("textures/glow.jpeg", jpeg.to_vec()),
    ]);
    let mut profile = profile(RuleSetId::Quake3);
    profile.image_settings.override_textures = 2;
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    owner.set_image_search_order(ImageSearchOrder::parse(b"tga").unwrap());
    let shader = owner
        .register_shader(
            &fixture.vfs,
            b"textures/body.tga",
            profile.source_rules,
            false,
            ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let model = owner
        .register_model(&fixture.vfs, b"models/test.md3", profile.source_rules)
        .unwrap();
    let skin = owner
        .register_skin(&fixture.vfs, b"models/test.skin", profile.source_rules)
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(model, profile)
        .unwrap();
    let policy = ModelMaterialPolicy {
        custom_skin: skin,
        ..Default::default()
    };
    let original_bindings = owner
        .catalog()
        .bindings(registration, 0, policy)
        .unwrap()
        .collect::<Vec<_>>();
    let original_shader = owner.catalog().shader_binding(shader, profile).unwrap();
    let StageTexture::Image(original_image) = owner
        .assets()
        .material(
            owner
                .assets()
                .shader_binding(original_shader)
                .unwrap()
                .material,
        )
        .unwrap()
        .stages[0]
        .texture
    else {
        panic!("original image")
    };
    let original_rgba = owner.assets().image(original_image).unwrap().rgba.clone();
    let (mut owner, mut cpu) = publish(owner);
    let original_pixels = paint_shader(&mut cpu, owner.assets(), original_shader);
    assert!(
        original_pixels
            .iter()
            .all(|pixel| *pixel == u32::from_le_bytes([211, 17, 29, 255]))
    );
    let prefix = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    fixture.remove("models/test.md3");
    fixture.remove("models/test.skin");
    let (mount, path) = {
        let mount = fixture.vfs.mounts().next().unwrap();
        (mount.id, mount.path.clone())
    };
    assert!(fixture.vfs.unmount(mount));
    fixture.vfs.mount_directory(&path, 0).unwrap();
    assert!(fixture.vfs.open(b"models/test.md3").is_none());
    assert!(fixture.vfs.open(b"models/test.skin").is_none());
    owner.set_image_search_order(ImageSearchOrder::parse(b"jpeg").unwrap());
    let mut prepared = None;
    let failed: Result<(), String> =
        owner.image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
            let refresh = owner.refresh_images(
                &fixture.vfs,
                &[profile],
                &[],
                [None; 5],
                profile.finish,
                false,
                |_| Ok(()),
            )?;
            prepared =
                Some(CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?);
            assert_eq!(refresh.profile(profile), profile);
            assert_eq!(
                refresh.registration(registration, owner.catalog())?,
                registration
            );
            let selected = owner.catalog().shader_binding(shader, profile).unwrap();
            assert_ne!(selected, original_shader);
            let pixels = paint_shader(prepared.as_mut().unwrap(), owner.assets(), selected);
            assert!(pixels.iter().all(|pixel| {
                let [r, _, b, _] = pixel.to_le_bytes();
                b > 230 && r < 10
            }));
            Err("reject after complete backend preparation".to_owned())
        });
    assert!(failed.is_err());
    drop(prepared);
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(cpu.admitted_assets(), Some(prefix));
    assert_eq!(
        owner.catalog().shader_binding(shader, profile),
        Some(original_shader)
    );
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, policy)
            .unwrap()
            .collect::<Vec<_>>(),
        original_bindings
    );
    assert_eq!(
        paint_shader(&mut cpu, owner.assets(), original_shader),
        original_pixels
    );
    fixture.vfs.take_lookup_count();
    let retained = request(
        b"textures/body.tga",
        profile.source_rules,
        RendererRegistration::Shader {
            mipmaps: false,
            lighting: ShaderLighting::TwoDimensional,
        },
    );
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &retained,
                profile,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("rollback must retain original cache")
            )
            .unwrap()
            .0,
        shader
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    for _ in 0..2 {
        let mut prepared = None;
        let refresh = owner
            .image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
                let refresh = owner.refresh_images(
                    &fixture.vfs,
                    &[profile],
                    &[],
                    [None; 5],
                    profile.finish,
                    false,
                    |_| Ok(()),
                )?;
                prepared = Some(
                    CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?,
                );
                Ok(refresh)
            })
            .unwrap();
        cpu = prepared.unwrap();
        assert_eq!(refresh.profile(profile), profile);
        assert_eq!(
            refresh.registration(registration, owner.catalog()).unwrap(),
            registration
        );
        assert_eq!(
            owner.catalog().renderer_registration(model, profile),
            Some(registration)
        );
        assert!(owner.catalog().renderer_skin(skin, profile));
        assert_eq!(owner.catalog().capacity(), capacity);
        assert_eq!(owner.assets().counts().models, prefix.models);
        assert_eq!(
            owner.assets().image(original_image).unwrap().rgba,
            original_rgba
        );
        let bindings = owner
            .catalog()
            .bindings(registration, 0, policy)
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(
            bindings
                .iter()
                .map(|binding| binding.model)
                .collect::<Vec<_>>(),
            original_bindings
                .iter()
                .map(|binding| binding.model)
                .collect::<Vec<_>>()
        );
        let selected = owner.catalog().shader_binding(shader, profile).unwrap();
        assert_ne!(selected, original_shader);
        assert_eq!(cpu.admitted_assets(), Some(owner.assets().counts()));
        let pixels = paint_shader(&mut cpu, owner.assets(), selected);
        assert_ne!(pixels, original_pixels);
        assert!(pixels.iter().all(|pixel| {
            let [r, _, b, _] = pixel.to_le_bytes();
            b > 230 && r < 10
        }));
    }
}

#[test]
fn lighting_only_refresh_replaces_exact_presentations_and_rolls_back_rejected_resources() {
    use qa_render::{
        material::world_load::WorldPresentationProfile,
        world::{WorldLighting, WorldRenderSettings},
    };
    let fixture = support::Fixture::new(&[("maps/light.bsp", support::world_map(16.0))]);
    let profile = profile(RuleSetId::Quake3);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let mut collision = qa_world::collision::CollisionStore::new();
    let loaded = crate::map::read(&fixture.vfs, "light")
        .unwrap()
        .load(
            &fixture.vfs,
            &mut assets,
            &mut collision,
            Some(palette),
            &[WorldPresentationProfile::native(profile.rules, profile.palette).into()],
            Default::default(),
            false,
        )
        .unwrap();
    let world = loaded.render.world;
    let previous = loaded.render.presentations[0];
    let owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [world], Default::default()).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let retained = owner.assets().counts();
    let mesh = owner.assets().world(world).unwrap().mesh();
    let settings = WorldRenderSettings {
        lighting: WorldLighting::from_requests(1, 1, None).unwrap(),
        ..WorldRenderSettings::default()
    };
    let rejected: Result<(), String> = owner.image_transaction(
        &fixture.vfs,
        Some(settings),
        ImageRefreshMode::Reconfigure,
        |owner, _| {
            owner.refresh_images(
                &fixture.vfs,
                &[profile],
                &[(world, previous.id)],
                [None; 5],
                qa_render::material::MaterialFinishPolicy::default(),
                false,
                |assets| {
                    assert!(assets.counts().presentations > retained.presentations);
                    Err("candidate backend rejected".to_owned())
                },
            )?;
            Ok(())
        },
    );
    assert!(rejected.unwrap_err().contains("candidate backend rejected"));
    assert_eq!(owner.assets().counts(), retained);
    assert_eq!(owner.catalog().admitted_assets, Some(retained));
    assert_eq!(owner.assets().world(world).unwrap().mesh(), mesh);
    assert_eq!(
        owner
            .assets()
            .world(world)
            .unwrap()
            .render_settings()
            .unwrap()
            .lighting,
        previous.lighting
    );
    assert_eq!(
        owner.assets().world_presentation_registration(previous.id),
        Some(previous)
    );
    let refresh = owner
        .image_transaction(
            &fixture.vfs,
            Some(settings),
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                owner.refresh_images(
                    &fixture.vfs,
                    &[profile],
                    &[(world, previous.id)],
                    [None; 5],
                    qa_render::material::MaterialFinishPolicy::default(),
                    false,
                    |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
            },
        )
        .unwrap();
    let selected = refresh.presentation(world, previous.id).unwrap();
    assert_eq!(refresh.profile(profile), profile);
    assert_eq!(selected.profile, previous.profile);
    assert_eq!(selected.lighting, settings.lighting);
    assert_ne!(selected.id, previous.id);
    assert_eq!(owner.assets().world(world).unwrap().mesh(), mesh);
    assert_eq!(
        owner.assets().world_presentation_registration(previous.id),
        Some(previous)
    );
    assert_eq!(
        owner.view().world_presentation(world, Some(profile), None),
        Some(selected)
    );
    assert_eq!(
        owner
            .view()
            .world_presentation(world, Some(profile), Some(previous.id)),
        Some(previous)
    );
}

#[test]
fn finish_refresh_reuses_world_images_after_backend_rejection() {
    use qa_render::material::{MaterialFinishPolicy, world_load::WorldPresentationProfile};

    let bytes = support::world_map_with_lumps(16.0, |lumps| {
        lumps[7][12] = 0;
        lumps[7][16..20].copy_from_slice(&0i32.to_le_bytes());
        lumps[8] = vec![96; 9];
    });
    let fixture = support::Fixture::new(&[("maps/finish.bsp", bytes)]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelProfile {
        source_rules: RuleSetId::Quake,
        rules: RuleSetId::Quake3,
        source_palette: Some(palette),
        palette: None,
        image_settings: ImageSettings::native(RuleSetId::Quake3),
        finish: MaterialFinishPolicy::default(),
    };
    let loaded = crate::map::read(&fixture.vfs, "finish")
        .unwrap()
        .load(
            &fixture.vfs,
            &mut assets,
            &mut qa_world::collision::CollisionStore::new(),
            Some(palette),
            &[WorldPresentationProfile::native(profile.rules, None).into()],
            Default::default(),
            false,
        )
        .unwrap();
    let world = loaded.render.world;
    let previous = loaded.render.presentations[0];
    let owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [world], Default::default()).unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let retained = owner.assets().counts();
    let finish = MaterialFinishPolicy {
        detail_textures: false,
        ..profile.finish
    };
    let candidate = Cell::new(None);
    let rejected: Result<(), String> = owner.image_transaction(
        &fixture.vfs,
        None,
        ImageRefreshMode::Reconfigure,
        |owner, _| {
            owner.refresh_images(
                &fixture.vfs,
                &[profile],
                &[(world, previous.id)],
                [None; 5],
                finish,
                false,
                |assets| {
                    if assets.counts().presentations == retained.presentations {
                        return Ok(());
                    }
                    assert_eq!(assets.counts().images, retained.images);
                    candidate.set(Some(assets.world(world).unwrap().presentations()[1]));
                    let prepared = CpuBackend::load_with_assets(8, 8, assets).unwrap();
                    assert_eq!(prepared.admitted_assets(), Some(assets.counts()));
                    Err("reject staged renderer".to_owned())
                },
            )?;
            Ok(())
        },
    );
    assert!(rejected.unwrap_err().contains("reject staged renderer"));
    assert_eq!(owner.assets().counts(), retained);
    assert_eq!(cpu.admitted_assets(), Some(retained));
    assert_eq!(
        owner.assets().world(world).unwrap().presentations(),
        &[previous.id]
    );

    let refresh = owner
        .image_transaction(
            &fixture.vfs,
            None,
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                owner.refresh_images(
                    &fixture.vfs,
                    &[profile],
                    &[(world, previous.id)],
                    [None; 5],
                    finish,
                    false,
                    |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
            },
        )
        .unwrap();
    let selected = refresh.presentation(world, previous.id).unwrap();
    assert_eq!(Some(selected.id), candidate.get());
    assert_eq!(selected.profile.finish, finish);
    assert_eq!(owner.assets().counts().images, retained.images);
    assert_eq!(cpu.admitted_assets(), Some(owner.assets().counts()));
    assert_eq!(
        owner.assets().world_presentation_registration(previous.id),
        Some(previous)
    );
}

#[test]
fn initial_worlds_keep_their_own_default_image_when_the_catalog_adopts_a_peer() {
    use qa_render::material::world_load::WorldPresentationProfile;

    let fixture = support::Fixture::new(&[
        ("maps/default.bsp", support::world_map(16.0)),
        (
            "textures/wall.tga",
            support::tga(2, 2, &[[19, 37, 71, 255]; 4]),
        ),
        (
            "scripts/default.shader",
            b"textures/wall { { map textures/wall.tga } }".to_vec(),
        ),
    ]);
    let profile = profile(RuleSetId::Quake3);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let mut collision = qa_world::collision::CollisionStore::new();
    let worlds = std::array::from_fn::<_, 2, _>(|_| {
        crate::map::read(&fixture.vfs, "default")
            .unwrap()
            .load(
                &fixture.vfs,
                &mut assets,
                &mut collision,
                Some(palette),
                &[WorldPresentationProfile::native(profile.rules, None).into()],
                Default::default(),
                false,
            )
            .unwrap()
            .render
    });
    let default_image = |assets: &Assets, world: WorldPresentationId| {
        let default = assets
            .world_presentation(world)
            .unwrap()
            .native_default()
            .unwrap();
        assets
            .material(assets.shader_binding(default).unwrap().material)
            .unwrap()
            .stages[0]
            .texture
    };
    let images = worlds
        .each_ref()
        .map(|world| default_image(&assets, world.presentations[0].id));
    assert_ne!(images[0], images[1]);
    let owner = ModelAssets::new(
        &fixture.vfs,
        assets,
        [profile],
        worlds.each_ref().map(|world| world.world),
        Default::default(),
    )
    .unwrap();
    let (mut owner, mut cpu) = publish(owner);
    let world = worlds[1].world;
    let previous = worlds[1].presentations[0];
    let mut finish = profile.finish;
    finish.detail_textures = false;
    let refresh = owner
        .image_transaction(
            &fixture.vfs,
            None,
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                owner.refresh_images(
                    &fixture.vfs,
                    &[profile],
                    &[(world, previous.id)],
                    [None; 5],
                    finish,
                    false,
                    |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
                )
            },
        )
        .unwrap();
    let selected = refresh.presentation(world, previous.id).unwrap();
    assert_eq!(default_image(owner.assets(), selected.id), images[1]);
    assert_eq!(
        default_image(owner.assets(), worlds[0].presentations[0].id),
        images[0]
    );
}

#[test]
fn pair_publishes_once_then_uses_both_cached_handles_without_io() {
    let fixture = shader_fixture();
    let profile = profile(RuleSetId::Quake3);
    let owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
        &fixture.vfs,
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
        &fixture.vfs,
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
                    crate::model_assets::RegistrationVariant::Guard(false),
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
                    crate::model_assets::RegistrationVariant::Guard(false),
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
        &fixture.vfs,
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
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
        let error = owner
            .prepare_registrations(
                &fixture.vfs,
                [&model, &skin],
                profile,
                crate::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| Err("reject appended model and skin".to_owned()),
            )
            .unwrap_err();
        assert!(error.contains("reject appended model and skin"), "{error}");
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
            crate::model_assets::RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(
        owner.catalog.files.name(ModelFileId(handles[0] as u32)),
        model_name
    );
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
        lighting: Default::default(),
        compile_policy: Default::default(),
        defaults: Default::default(),
    });
    assets
        .register_world_presentation_metadata(world, &entries, None)
        .unwrap();
    let owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [world], Default::default()).unwrap();
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
        search: None,
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
    let ordinary_request = owner.catalog.shaders[shader_index]
        .first_request(&owner.catalog.shader_bindings, profile.source_rules)
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
    let ordinary_request = owner.catalog.shaders[shader_index]
        .first_request(&owner.catalog.shader_bindings, profile.source_rules)
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
                crate::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| { panic!("missing selected world reached backend") }
            )
            .is_err()
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);

    let vertex = RendererRequest {
        search: None,
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
            crate::model_assets::RegistrationVariant::Guard(false),
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
        search: None,
        name: b"textures/absent".as_slice(),
        ..vertex
    };
    let (public, _, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &missing,
            profile,
            crate::model_assets::RegistrationVariant::Guard(false),
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
            None,
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
            .shader_outcome(missing.name, profile, ShaderLighting::TwoDimensional, None),
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
                crate::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| { panic!("cached missing world shader reached backend") }
            )
            .unwrap()
            .0,
        0
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let ordinary_missing = RendererRequest {
        search: None,
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
                crate::model_assets::RegistrationVariant::Guard(false),
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
            .shader_outcome(missing.name, profile, ShaderLighting::TwoDimensional, None),
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
    let mut owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [], Default::default()).unwrap();
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
        let ModelShaderChoice::Single(shader @ ShaderChoice::Direct(_)) = choice.shader else {
            panic!("single authored skin");
        };
        let selected = view
            .choice(shader, choice.registrations, None, choice.default)
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

pub(super) fn interleaved_mdl_frames() -> Vec<u8> {
    let original = support::mdl();
    let mut bytes = original[..84].to_vec();
    bytes[48..52].copy_from_slice(&4i32.to_le_bytes());
    bytes[68..72].copy_from_slice(&4i32.to_le_bytes());
    bytes[72..76].copy_from_slice(&1i32.to_le_bytes());
    for (members, endpoints) in [
        (1..2u8, &[][..]),
        (2..4, &[0.125f32, 0.5][..]),
        (4..7, &[0.125, 0.375, 1.][..]),
        (7..13, &[0.125, 0.25, 0.5, 0.75, 1., 1.5][..]),
    ] {
        bytes.extend(i32::from(!endpoints.is_empty()).to_le_bytes());
        if !endpoints.is_empty() {
            bytes.extend((members.len() as i32).to_le_bytes());
            bytes.extend(endpoints.iter().flat_map(|value| value.to_le_bytes()));
        }
        for member in members {
            bytes.extend([member; 2]);
        }
    }
    bytes.extend_from_slice(&original[90..142]);
    let pose = |index: u8| {
        let mut body = original[146..182].to_vec();
        body[..8].copy_from_slice(&[0, 0, 0, 0, 32, 32, 32, 0]);
        body[8..24].fill(0);
        body[8] = b'a' + index;
        for vertex in 0..3 {
            body[24 + vertex * 4] = 8 + index * 2;
        }
        body
    };
    for (members, endpoints) in [
        (0..1u8, &[][..]),
        (1..4, &[0.125f32, 0.375, 1.0][..]),
        (4..5, &[][..]),
        (5..7, &[0.25, 0.75][..]),
    ] {
        bytes.extend(i32::from(!endpoints.is_empty()).to_le_bytes());
        if !endpoints.is_empty() {
            bytes.extend((members.len() as i32).to_le_bytes());
            bytes.extend([0, 0, 0, 0, 32, 32, 32, 0]);
            bytes.extend(endpoints.iter().flat_map(|value| value.to_le_bytes()));
        }
        for member in members {
            bytes.extend(pose(member));
        }
    }
    bytes
}

#[test]
fn grouped_mdl_parent_frames_and_recipe_survive_profile_and_forced_reload_without_source() {
    let mut fixture = support::Fixture::new(&[("progs/grouped.mdl", interleaved_mdl_frames())]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let original = ModelRequest::native("", RuleSetId::Quake, Some(palette)).profile();
    let mut owner =
        ModelAssets::new(&fixture.vfs, assets, [original], [], Default::default()).unwrap();
    owner.set_image_search_order(ImageSearchOrder::parse(b"").unwrap());
    owner
        .adopt_palettes(
            crate::render_settings::PaletteRegistrations::from_registered(
                &fixture.vfs,
                [Some(palette), Some(palette), None, None, None],
            ),
        )
        .unwrap();
    let models_before = owner.assets().counts().models;
    let handle = owner
        .register_model(&fixture.vfs, b"progs/grouped.mdl", original.source_rules)
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(handle, original)
        .unwrap();
    let binding = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    let model = owner.assets().model(binding.model).unwrap();
    assert_eq!(model.frame_count(), 7);
    assert_eq!(model.native_frame_count(), 4);
    assert_eq!(model.native_skin_count(), 4);
    let recipe = model.animation().unwrap();
    assert_eq!(recipe.sync(), 1);
    assert_eq!(&recipe.endpoints()[11..], &[0.125, 0.375, 1., 0.25, 0.75]);
    assert_eq!(
        recipe
            .skin_groups()
            .iter()
            .map(|group| group.members)
            .collect::<Vec<_>>(),
        [
            IndexSpan { first: 0, count: 1 },
            IndexSpan { first: 1, count: 2 },
            IndexSpan { first: 3, count: 3 },
            IndexSpan { first: 6, count: 6 },
        ]
    );
    assert_eq!(
        recipe
            .frame_groups()
            .iter()
            .map(|group| group.members)
            .collect::<Vec<_>>(),
        [
            IndexSpan { first: 0, count: 1 },
            IndexSpan { first: 1, count: 3 },
            IndexSpan { first: 4, count: 1 },
            IndexSpan { first: 5, count: 2 },
        ]
    );
    assert_eq!(
        recipe.frame_groups()[1].bounds.maxs,
        qa_core::primitives::Vec3([32.; 3])
    );
    let recipe_address = recipe as *const qa_render::assets::ModelAnimation;
    for (frame, x) in [8., 10., 12., 14., 16., 18., 20.].into_iter().enumerate() {
        assert_eq!(
            model
                .frames(frame as u32, frame as u32, 0.)
                .unwrap()
                .vertex(0)
                .position
                .0[0],
            x
        );
    }
    let models = owner.assets().counts().models;
    assert_eq!(models, models_before + 1);
    let skin_images = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Model, handle, original)
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, receipt)| {
            let material = owner
                .assets()
                .material(
                    owner
                        .assets()
                        .shader_binding(receipt.shader)
                        .unwrap()
                        .material,
                )
                .unwrap();
            let StageTexture::Image(image) = material.stages[0].texture else {
                panic!("embedded grouped skin")
            };
            let image = owner.assets().image(image).unwrap();
            assert_eq!(
                image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
                [index as u8 + 1; 2]
            );
            (receipt.shader, image.rgba.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(skin_images.len(), 12);
    let parent_bindings = |owner: &ModelAssets, registration| {
        [0, 1, 2, 3, u32::MAX].map(|parent| {
            let mut parts = owner
                .catalog()
                .bindings(registration, parent, Default::default())
                .unwrap()
                .choices();
            let part = parts.next().unwrap();
            assert!(parts.next().is_none());
            assert_eq!(part.model, binding.model);
            if (1..=3).contains(&parent) {
                let ModelShaderChoice::SkinGroup { group, members } = part.shader else {
                    panic!("grouped native skin parent")
                };
                assert_eq!(group, parent);
                assert_eq!(members.len(), [1, 2, 3, 6][parent as usize]);
                assert!(part.physical_binding().is_none());
            } else {
                assert!(matches!(part.shader, ModelShaderChoice::Single(_)));
                assert!(part.physical_binding().is_some());
            }
            part.shaders()
                .map(|shader| {
                    part.resolve_shader(
                        shader,
                        qa_render::shader::reference::ShaderView::direct(),
                        None,
                    )
                    .unwrap()
                    .source(&())
                    .unwrap()
                })
                .collect::<Vec<_>>()
        })
    };
    let original_skins = parent_bindings(&owner, registration);
    assert_eq!(
        original_skins.clone().map(|members| members.len()),
        [1, 2, 3, 6, 1]
    );
    assert_eq!(original_skins[0], original_skins[4]);
    assert_eq!(
        original_skins[..4].concat(),
        skin_images
            .iter()
            .map(|(shader, _)| *shader)
            .collect::<Vec<_>>()
    );
    let check_original_images = |owner: &ModelAssets| {
        for (shader, rgba) in &skin_images {
            let material = owner
                .assets()
                .material(owner.assets().shader_binding(*shader).unwrap().material)
                .unwrap();
            let StageTexture::Image(image) = material.stages[0].texture else {
                panic!("retained skin image")
            };
            assert_eq!(&owner.assets().image(image).unwrap().rgba, rgba);
        }
    };
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, u32::MAX, Default::default())
            .unwrap()
            .next(),
        Some(binding)
    );
    let (mut owner, mut cpu) = publish(owner);
    fixture.remove("progs/grouped.mdl");
    let (mount, path) = {
        let mount = fixture.vfs.mounts().next().unwrap();
        (mount.id, mount.path.clone())
    };
    assert!(fixture.vfs.unmount(mount));
    fixture.vfs.mount_directory(&path, 0).unwrap();
    assert!(fixture.vfs.open(b"progs/grouped.mdl").is_none());
    fixture.vfs.take_lookup_count();
    let finish = MaterialFinishPolicy {
        vertex_light: true,
        ..original.finish
    };
    let mut image_settings = original.image_settings;
    image_settings.gamma_exponent = 0.5;
    let mut settings = [None; 5];
    settings[original.rules as usize] = Some(image_settings);
    let prefix = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    let mut prepared = None;
    let failed: Result<(), String> = owner.image_transaction(
        &fixture.vfs,
        None,
        ImageRefreshMode::Reconfigure,
        |owner, _| {
            let refresh = owner.refresh_images(
                &fixture.vfs,
                &[original],
                &[],
                settings,
                finish,
                false,
                |_| Ok(()),
            )?;
            let selected = refresh.registration(registration, owner.catalog())?;
            assert_eq!(
                owner
                    .catalog()
                    .bindings(selected, 0, Default::default())
                    .unwrap()
                    .next()
                    .unwrap()
                    .model,
                binding.model
            );
            prepared =
                Some(CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?);
            Err("reject retained grouped model profile".to_owned())
        },
    );
    assert!(failed.is_err());
    drop(prepared.take());
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(owner.catalog().capacity(), capacity);
    check_original_images(&owner);
    assert_eq!(parent_bindings(&owner, registration), original_skins);
    assert_eq!(cpu.admitted_assets(), Some(prefix));
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .next(),
        Some(binding)
    );
    let refresh = owner
        .image_transaction(
            &fixture.vfs,
            None,
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                let refresh = owner.refresh_images(
                    &fixture.vfs,
                    &[original],
                    &[],
                    settings,
                    finish,
                    false,
                    |_| Ok(()),
                )?;
                prepared = Some(
                    CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?,
                );
                Ok(refresh)
            },
        )
        .unwrap();
    let current = refresh.profile(original);
    cpu = prepared.take().unwrap();
    assert_eq!(cpu.admitted_assets(), Some(owner.assets().counts()));
    assert_ne!(current, original);
    assert_eq!(current.image_settings, image_settings);
    let current_registration = owner
        .catalog()
        .renderer_registration(handle, current)
        .unwrap();
    assert_eq!(
        refresh.registration(registration, owner.catalog()).unwrap(),
        current_registration
    );
    let current_binding = owner
        .catalog()
        .bindings(current_registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(current_binding.model, binding.model);
    let current_skins = parent_bindings(&owner, current_registration);
    assert_eq!(
        current_skins.clone().map(|members| members.len()),
        [1, 2, 3, 6, 1]
    );
    assert_eq!(current_skins[0], current_skins[4]);
    assert_ne!(current_skins, original_skins);
    check_original_images(&owner);
    assert!(std::ptr::eq(
        owner
            .assets()
            .model(binding.model)
            .unwrap()
            .animation()
            .unwrap(),
        recipe_address
    ));
    let capacity = owner.catalog().capacity();
    let refresh = owner
        .image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
            let refresh = owner.refresh_images(
                &fixture.vfs,
                &[current],
                &[],
                [None; 5],
                finish,
                false,
                |_| Ok(()),
            )?;
            prepared =
                Some(CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?);
            Ok(refresh)
        })
        .unwrap();
    cpu = prepared.unwrap();
    assert_eq!(refresh.profile(current), current);
    assert_eq!(
        refresh
            .registration(current_registration, owner.catalog())
            .unwrap(),
        current_registration
    );
    let reloaded = owner
        .catalog()
        .bindings(current_registration, 0, Default::default())
        .unwrap()
        .next()
        .unwrap();
    assert_eq!(reloaded.model, binding.model);
    let reloaded_skins = parent_bindings(&owner, current_registration);
    assert_eq!(
        reloaded_skins.clone().map(|members| members.len()),
        [1, 2, 3, 6, 1]
    );
    assert_eq!(reloaded_skins[0], reloaded_skins[4]);
    for (index, shader) in reloaded_skins[..4].concat().into_iter().enumerate() {
        let material = owner
            .assets()
            .material(owner.assets().shader_binding(shader).unwrap().material)
            .unwrap();
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("reloaded grouped skin image")
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
            [index as u8 + 1; 2]
        );
    }
    check_original_images(&owner);
    assert_eq!(owner.assets().counts().models, models);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert!(std::ptr::eq(
        owner
            .assets()
            .model(reloaded.model)
            .unwrap()
            .animation()
            .unwrap(),
        recipe_address
    ));
    assert_eq!(
        owner
            .assets()
            .model(reloaded.model)
            .unwrap()
            .native_frame_count(),
        4
    );
    assert_eq!(cpu.admitted_assets(), Some(owner.assets().counts()));
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}

#[test]
fn grouped_spr_keeps_one_mesh_and_authored_frames_through_source_unlink_and_refresh_retry() {
    let mut fixture = support::Fixture::new(&[("progs/mixed.spr", support::spr())]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = ModelRequest::native("", RuleSetId::Quake, Some(palette)).profile();
    let mut owner =
        ModelAssets::new(&fixture.vfs, assets, [profile], [], Default::default()).unwrap();
    owner.set_image_search_order(ImageSearchOrder::parse(b"").unwrap());
    owner
        .adopt_palettes(
            crate::render_settings::PaletteRegistrations::from_registered(
                &fixture.vfs,
                [Some(palette), Some(palette), None, None, None],
            ),
        )
        .unwrap();
    let models_before = owner.assets().counts().models;
    let handle = owner
        .register_model(&fixture.vfs, b"progs/mixed.spr", profile.source_rules)
        .unwrap();
    let registration = owner
        .catalog()
        .renderer_registration(handle, profile)
        .unwrap();
    let choice = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .choices()
        .next()
        .unwrap();
    let ModelShaderChoice::Frames { members } = choice.shader else {
        panic!("deferred SPR physical frames")
    };
    assert_eq!(members.len(), 4);
    assert!(choice.physical_binding().is_none());
    let model_id = choice.model;
    let model = owner.assets().model(model_id).unwrap();
    assert_eq!((model.frame_count(), model.native_frame_count()), (4, 3));
    let geometry = model.sprite.as_ref().unwrap();
    assert_eq!(
        geometry.orientation,
        qa_render::assets::SpriteOrientation::ViewParallel
    );
    assert_eq!(geometry.beam_length, 1.5);
    let extents = [
        [-2., 2., 2., -2.],
        [-3., 1., 1., -1.],
        [-1., 3., 3., -1.],
        [-4., 1., 1., -2.],
    ];
    for (index, expected) in extents.into_iter().enumerate() {
        assert_eq!(model.sprite_frame(index as u32).unwrap().extents, expected);
    }
    let animation = model.animation().unwrap();
    assert_eq!(animation.sync(), 1);
    assert_eq!(animation.endpoints(), [0.125, 0.5]);
    assert_eq!(
        animation
            .frame_groups()
            .iter()
            .map(|group| group.members)
            .collect::<Vec<_>>(),
        [
            IndexSpan { first: 0, count: 1 },
            IndexSpan { first: 1, count: 2 },
            IndexSpan { first: 3, count: 1 }
        ]
    );
    let animation_address = animation as *const qa_render::assets::ModelAnimation;
    let original_shaders = choice
        .shaders()
        .map(|shader| {
            choice
                .resolve_shader(
                    shader,
                    qa_render::shader::reference::ShaderView::direct(),
                    None,
                )
                .unwrap()
                .source(&())
                .unwrap()
        })
        .collect::<Vec<_>>();
    for (shader, index) in original_shaders.iter().zip([11, 22, 33, 44]) {
        let material = owner
            .assets()
            .material(owner.assets().shader_binding(*shader).unwrap().material)
            .unwrap();
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("indexed sprite frame")
        };
        assert!(
            owner
                .assets()
                .image(image)
                .unwrap()
                .indexed
                .as_ref()
                .unwrap()
                .mip(0)
                .unwrap()
                .indices()
                .iter()
                .all(|value| *value == index)
        );
    }
    let (mut owner, mut cpu) = publish(owner);
    fixture.remove("progs/mixed.spr");
    let (mount, path) = {
        let mount = fixture.vfs.mounts().next().unwrap();
        (mount.id, mount.path.clone())
    };
    assert!(fixture.vfs.unmount(mount));
    fixture.vfs.mount_directory(&path, 0).unwrap();
    assert!(fixture.vfs.open(b"progs/mixed.spr").is_none());
    fixture.vfs.take_lookup_count();
    let prefix = owner.assets().counts();
    let finish = MaterialFinishPolicy {
        vertex_light: true,
        ..profile.finish
    };
    let mut changed = profile;
    for reject in [true, false] {
        let result = owner.image_transaction(
            &fixture.vfs,
            None,
            ImageRefreshMode::Reconfigure,
            |owner, _| {
                let refresh = owner.refresh_images(
                    &fixture.vfs,
                    &[profile],
                    &[],
                    [None; 5],
                    finish,
                    false,
                    |_| Ok(()),
                )?;
                let candidate =
                    CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?;
                if reject {
                    return Err("reject refreshed SPR backend".to_owned());
                }
                cpu = candidate;
                changed = refresh.profile(profile);
                Ok(())
            },
        );
        if reject {
            assert!(result.is_err());
            assert_eq!(owner.assets().counts(), prefix);
            assert_eq!(
                owner.catalog().renderer_registration(handle, profile),
                Some(registration)
            );
            assert_eq!(cpu.admitted_assets(), Some(prefix));
        } else {
            result.unwrap();
        }
    }
    assert_ne!(changed, profile);
    let current = owner
        .catalog()
        .renderer_registration(handle, changed)
        .unwrap();
    owner
        .image_transaction(&fixture.vfs, None, ImageRefreshMode::Reload, |owner, _| {
            let refresh = owner.refresh_images(
                &fixture.vfs,
                &[changed],
                &[],
                [None; 5],
                finish,
                false,
                |_| Ok(()),
            )?;
            assert_eq!(refresh.registration(current, owner.catalog())?, current);
            cpu = CpuBackend::load_with_assets(8, 8, owner.assets()).map_err(str::to_owned)?;
            Ok(())
        })
        .unwrap();
    let reloaded = owner
        .catalog()
        .bindings(current, 0, Default::default())
        .unwrap()
        .choices()
        .next()
        .unwrap();
    assert_eq!(reloaded.model, model_id);
    assert_eq!(reloaded.shaders().count(), 4);
    assert_eq!(owner.assets().counts().models, models_before + 1);
    let model = owner.assets().model(model_id).unwrap();
    assert!(std::ptr::eq(model.animation().unwrap(), animation_address));
    for (index, expected) in extents.into_iter().enumerate() {
        assert_eq!(model.sprite_frame(index as u32).unwrap().extents, expected);
    }
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(cpu.admitted_assets(), Some(owner.assets().counts()));
}

#[path = "indexed_skin_refresh.rs"]
mod indexed_skin_refresh;

#[path = "published_files.rs"]
mod published_files;
