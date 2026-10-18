#[path = "world_presentations.rs"]
mod world_presentations;

use super::*;
use qa_compat::services::{RendererRegistration, RendererRequest};
use qa_render::{
    cpu::{CpuBackend, CpuLimits},
    material::resources::ImageSettings,
};

fn shader_image_policy(
    owner: &ModelAssets,
    handle: i32,
    profile: ModelProfile,
) -> (qa_render::ImageId, qa_render::assets::Sampler, [u8; 4]) {
    let shader = owner.catalog().shader_binding(handle, profile).unwrap();
    let stage = &owner
        .assets()
        .material(owner.assets().shader_binding(shader).unwrap().material)
        .unwrap()
        .stages[0];
    let StageTexture::Image(image) = stage.texture else {
        panic!("registered shader image")
    };
    (
        image,
        stage.sampler,
        owner.assets().image(image).unwrap().rgba[..4]
            .try_into()
            .unwrap(),
    )
}

#[test]
fn precreated_profiles_follow_first_published_source_request_not_profile_order() {
    use qa_app::model_assets::RegistrationVariant;
    use qa_compat::services::ShaderLighting;
    let fixture = support::Fixture::new(&[
        (
            "ui/shared.tga",
            support::tga(4, 4, &[[211, 17, 29, 255]; 16]),
        ),
        (
            "ui/shared.jpeg",
            include_bytes!("../../../render/tests/resources/blue.jpg").to_vec(),
        ),
    ]);
    for reverse in [false, true] {
        let mut native = shader_profile(RuleSetId::Quake3);
        native.image_settings.picmip = 0;
        native.image_settings.override_textures = 0;
        let foreign = ModelProfile {
            source_rules: RuleSetId::Quake,
            ..native
        };
        let profiles = if reverse {
            [native, foreign]
        } else {
            [foreign, native]
        };
        let mut owner = ModelAssets::new(
            &fixture.vfs,
            Assets::load().unwrap(),
            profiles,
            [],
            Default::default(),
        )
        .unwrap();
        let mut cpu = cpu(&mut owner);
        let kind = |mipmaps| RendererRegistration::Shader {
            mipmaps,
            lighting: ShaderLighting::TwoDimensional,
        };
        let rejected = renderer_request("ui/shared.tga", kind(true), native.source_rules, None);
        let before = owner.assets().counts();
        assert!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &rejected,
                    native,
                    RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |_, _| Err("reject first request".to_owned())
                )
                .is_err()
        );
        assert_eq!(owner.assets().counts(), before);
        assert_eq!(cpu.admitted_assets(), Some(before));
        assert_eq!(
            owner
                .catalog()
                .registration_cached(rejected.kind, b"ui/shared.tga", native, None,),
            None
        );
        let first = renderer_request("ui/shared.jpeg", kind(false), native.source_rules, None);
        let handle = owner
            .prepare_registration(
                &fixture.vfs,
                &first,
                native,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap()
            .0;
        assert!(handle > 0);
        let original = shader_image_policy(&owner, handle, native);
        assert!(!original.1.mipmaps);
        assert!(original.2[2] > 230 && original.2[0] < 10);
        let other = renderer_request("ui/shared.tga", kind(true), foreign.source_rules, None);
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &other,
                    foreign,
                    RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
                )
                .unwrap()
                .0,
            handle
        );
        let independent = shader_image_policy(&owner, handle, foreign);
        assert!(independent.1.mipmaps);
        assert_eq!(independent.2, [211, 17, 29, 255]);
        let mut settings = native.image_settings;
        settings.picmip += 1;
        let (later, _) = owner
            .prepare_profile(
                &fixture.vfs,
                native.source_rules,
                native.rules,
                settings,
                native.finish,
                None,
                |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
            )
            .unwrap();
        let changed = renderer_request("ui/shared.tga", kind(true), native.source_rules, None);
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &changed,
                    later,
                    RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
                )
                .unwrap()
                .0,
            handle
        );
        let rebuilt = shader_image_policy(&owner, handle, later);
        assert!(!rebuilt.1.mipmaps);
        assert_eq!(rebuilt.2, original.2);
        assert_eq!(shader_image_policy(&owner, handle, native), original);
        assert_eq!(shader_image_policy(&owner, handle, foreign), independent);
        fixture.vfs.take_lookup_count();
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &changed,
                    later,
                    RegistrationVariant::Retained,
                    |_| Ok(()),
                    |_, _| panic!("retained first request must not prepare again")
                )
                .unwrap()
                .0,
            handle
        );
        assert_eq!(fixture.vfs.take_lookup_count(), 0);
    }
}

#[test]
fn positive_skin_source_survives_unlink_and_new_finish_backend_rejection() {
    use qa_app::model_assets::RegistrationVariant;
    let mut fixture = support::Fixture::surfaces();
    let first = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [first],
        [],
        Default::default(),
    )
    .unwrap();
    let model = owner
        .register_model(&fixture.vfs, b"models/test.md3", first.source_rules)
        .unwrap();
    let skin = owner
        .register_skin(&fixture.vfs, b"models/test.skin", first.source_rules)
        .unwrap();
    let registration = owner.catalog().renderer_registration(model, first).unwrap();
    let policy = ModelMaterialPolicy {
        custom_skin: skin,
        ..Default::default()
    };
    let original = owner
        .catalog()
        .bindings(registration, 0, policy)
        .unwrap()
        .collect::<Vec<_>>();
    let old_receipts = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Skin, skin, first)
        .unwrap()
        .to_vec();
    let mut cpu = cpu(&mut owner);
    let meshes = owner.assets().counts().models;
    fixture.remove("models/test.skin");
    fixture.remove("models/test.md3");
    let mount = fixture.vfs.mounts().next().unwrap().id;
    assert!(fixture.vfs.unmount(mount));
    assert!(fixture.vfs.open(b"models/test.skin").is_none());
    fixture.vfs.take_lookup_count();
    let finish = qa_render::material::MaterialFinishPolicy {
        vertex_light: true,
        ..first.finish
    };
    let (later, _) = owner
        .prepare_profile(
            &fixture.vfs,
            first.source_rules,
            first.rules,
            first.image_settings,
            finish,
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let model_request = renderer_request(
        "models/test.md3",
        RendererRegistration::Model,
        first.source_rules,
        None,
    );
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &model_request,
                later,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        model
    );
    let request = renderer_request(
        "models/test.skin",
        RendererRegistration::Skin,
        first.source_rules,
        None,
    );
    let before = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                later,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| Err("reject retained skin variant".to_owned())
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert!(!owner.catalog().renderer_skin(skin, later));
    assert!(owner.catalog().renderer_skin(skin, first));
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                later,
                RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        skin
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let new_registration = owner.catalog().renderer_registration(model, later).unwrap();
    let rebuilt = owner
        .catalog()
        .bindings(new_registration, 0, policy)
        .unwrap()
        .collect::<Vec<_>>();
    assert_eq!(
        rebuilt
            .iter()
            .map(|binding| binding.model)
            .collect::<Vec<_>>(),
        original
            .iter()
            .map(|binding| binding.model)
            .collect::<Vec<_>>()
    );
    assert_eq!(owner.assets().counts().models, meshes);
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, policy)
            .unwrap()
            .collect::<Vec<_>>(),
        original
    );
    let new_receipts = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Skin, skin, later)
        .unwrap();
    assert_eq!(
        new_receipts.iter().map(|row| row.key).collect::<Vec<_>>(),
        old_receipts.iter().map(|row| row.key).collect::<Vec<_>>()
    );
    let stages = |shader| {
        owner
            .assets()
            .material(owner.assets().shader_binding(shader).unwrap().material)
            .unwrap()
            .stages
            .len()
    };
    assert_eq!(stages(old_receipts[1].shader), 2);
    assert_eq!(stages(new_receipts[1].shader), 1);
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                later,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("retained skin must not prepare again")
            )
            .unwrap()
            .0,
        skin
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}

fn cpu(owner: &mut ModelAssets) -> CpuBackend {
    let cpu = CpuBackend::load_with_limits(
        64,
        48,
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

fn native_palettes() -> Vec<(&'static str, Vec<u8>)> {
    let rgb = (0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect();
    let mut shades = (0..64).flat_map(|_| 0..=255u8).collect::<Vec<_>>();
    shades.push(32);
    vec![
        ("gfx/palette.lmp", rgb),
        ("gfx/colormap.lmp", shades),
        ("pics/colormap.pcx", support::pcx_dimensions(256, 320, 11)),
    ]
}

#[test]
fn late_profile_palette_admission_rolls_back_and_installs_fallback_only_after_backend_success() {
    let fixture = support::Fixture::new(&native_palettes());
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [],
        [],
        Default::default(),
    )
    .unwrap();
    let mut cpu = cpu(&mut owner);
    let before = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    let settings = ImageSettings::native(RuleSetId::Quake);
    let result = owner.prepare_profile(
        &fixture.vfs,
        RuleSetId::Quake2,
        RuleSetId::Quake,
        settings,
        qa_render::material::MaterialFinishPolicy::default(),
        None,
        |assets| {
            assert_eq!(assets.counts().palettes, before.palettes + 2);
            let target = PaletteId((before.palettes + 1) as u32);
            assert!(assets.palette(target).unwrap().translucency_map().is_none());
            Err("backend unavailable".to_owned())
        },
    );
    assert!(result.unwrap_err().contains("backend unavailable"));
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().capacity(), capacity);
    for source in RuleSetId::ALL {
        assert_eq!(owner.palettes().get(source, None), None);
    }
    assert_eq!(cpu.admitted_assets(), Some(before));
    let (profile, counts) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake2,
            RuleSetId::Quake,
            settings,
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| {
                assert!(
                    assets
                        .palette(PaletteId((before.palettes + 1) as u32))
                        .unwrap()
                        .translucency_map()
                        .is_none()
                );
                cpu.prepare_assets(assets, None).map_err(str::to_owned)
            },
        )
        .unwrap();
    assert_eq!(cpu.admitted_assets(), Some(counts));
    assert_eq!(counts.palettes, before.palettes + 2);
    assert_eq!(
        profile.source_palette,
        owner.palettes().get(RuleSetId::Quake2Rerelease, None)
    );
    assert_eq!(
        profile.palette,
        owner.palettes().get(RuleSetId::QuakeWorld, None)
    );
    assert_ne!(profile.source_palette, profile.palette);
    let target = profile.palette.unwrap();
    assert_eq!(
        owner
            .assets()
            .palette(target)
            .unwrap()
            .translucency_map()
            .unwrap()
            .len(),
        65_536
    );
    let authored = owner
        .assets()
        .palette(profile.source_palette.unwrap())
        .unwrap()
        .translucency_map()
        .unwrap()
        .to_vec();
    assert_eq!(authored, vec![11; 65_536]);
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_profile(
                &fixture.vfs,
                RuleSetId::Quake2,
                RuleSetId::Quake,
                settings,
                qa_render::material::MaterialFinishPolicy::default(),
                None,
                |_| { panic!("unchanged exact profile must not prepare again") }
            )
            .unwrap(),
        (profile, counts)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(
        owner
            .assets()
            .palette(profile.source_palette.unwrap())
            .unwrap()
            .translucency_map()
            .unwrap(),
        authored
    );
}

#[test]
fn appended_profile_membership_keeps_old_model_shader_skin_and_color_bindings_stable() {
    let fixture = support::Fixture::new(&[
        ("models/a.md2", support::md2()),
        ("models/b.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx(11)),
        ("models/test/skin1.pcx", support::pcx(22)),
        ("sprites/test.sp2", support::sp2()),
        ("sprites/first.pcx", support::pcx(33)),
        ("sprites/second.pcx", support::pcx(44)),
        (
            "ui_a.part/first.tga",
            support::tga(4, 4, &[[211, 17, 29, 255]; 16]),
        ),
        (
            "ui_b.part/first.tga",
            support::tga(4, 4, &[[31, 223, 47, 255]; 16]),
        ),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let first = native_profile(RuleSetId::Quake2, palette);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        assets,
        [first],
        [],
        ModelLimits {
            frame_vertices: 20,
            indices: 12,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered(
                &fixture.vfs,
                [None, None, Some(palette), Some(palette), None],
            ),
        )
        .unwrap();
    let models = [b"models/a.md2".as_slice(), b"models/b.md2".as_slice()].map(|path| {
        owner
            .register_model(&fixture.vfs, path, first.source_rules)
            .unwrap()
    });
    let sprite = owner
        .register_model(&fixture.vfs, b"sprites/test.sp2", first.source_rules)
        .unwrap();
    let sprite_model = owner
        .catalog()
        .bindings(
            owner
                .catalog()
                .renderer_registration(sprite, first)
                .unwrap(),
            0,
            Default::default(),
        )
        .unwrap()
        .choices()
        .next()
        .unwrap()
        .model;
    assert_eq!(owner.assets().model(sprite_model).unwrap().frame_count(), 2);
    let registrations = models.map(|id| owner.catalog().renderer_registration(id, first).unwrap());
    let original = binding(owner.catalog(), "models/a.md2", first, 0).unwrap();
    let colors = owner.catalog().color_binding(first);
    let shaders = [
        (b"ui_a.part/first.tga".as_slice(), false),
        (b"ui_b.part/first.tga".as_slice(), true),
    ]
    .map(|(path, mips)| {
        owner
            .register_shader(
                &fixture.vfs,
                path,
                first.source_rules,
                mips,
                qa_compat::services::ShaderLighting::TwoDimensional,
            )
            .unwrap()
    });
    let materials = shaders.map(|id| owner.catalog().shader_binding(id, first).unwrap());
    let skin = owner
        .register_skin(&fixture.vfs, b"models/test/skin0.pcx", first.source_rules)
        .unwrap();
    assert!(owner.catalog().renderer_skin(skin, first));
    let mut cpu = cpu(&mut owner);
    let pixels = render_indexed_on(&mut cpu, owner.assets(), original, palette);
    let before = owner.assets().counts();
    let (second, counts) = owner
        .prepare_profile(
            &fixture.vfs,
            first.source_rules,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(before, counts);
    assert_eq!(owner.catalog().color_binding(first), colors);
    for (id, registration) in models.into_iter().zip(registrations) {
        assert_eq!(
            owner.catalog().renderer_registration(id, first),
            Some(registration)
        );
        assert_eq!(owner.catalog().renderer_registration(id, second), None);
        let name = owner
            .catalog()
            .name(qa_core::primitives::NameId(id as u32))
            .unwrap()
            .to_owned();
        let request = RendererRequest {
            name: name.as_bytes(),
            source: second.source_rules,
            kind: RendererRegistration::Model,
            world: None,
        };
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &request,
                    second,
                    qa_app::model_assets::RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
                )
                .unwrap()
                .0,
            id
        );
        assert!(owner.catalog().renderer_registration(id, second).is_some());
        assert_eq!(
            binding(owner.catalog(), &name, second, 0).unwrap().model,
            binding(owner.catalog(), &name, first, 0).unwrap().model
        );
        assert_eq!(
            owner.catalog().renderer_registration(id, first),
            Some(registration)
        );
    }
    let request = RendererRequest {
        name: b"sprites/test.sp2".as_slice(),
        source: second.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    };
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                second,
                qa_app::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        sprite
    );
    let sprite_choice = owner
        .catalog()
        .bindings(
            owner
                .catalog()
                .renderer_registration(sprite, second)
                .unwrap(),
            0,
            Default::default(),
        )
        .unwrap()
        .choices()
        .next()
        .unwrap();
    assert_eq!(sprite_choice.model, sprite_model);
    assert_eq!(sprite_choice.shaders().count(), 2);
    for (index, path) in ["ui_a.part/unavailable.tga", "ui_b.part/unavailable.tga"]
        .into_iter()
        .enumerate()
    {
        let request = RendererRequest {
            name: path.as_bytes(),
            source: second.source_rules,
            kind: RendererRegistration::Shader {
                mipmaps: index == 0,
                lighting: qa_compat::services::ShaderLighting::TwoDimensional,
            },
            world: None,
        };
        assert_eq!(
            owner
                .prepare_registration(
                    &fixture.vfs,
                    &request,
                    second,
                    qa_app::model_assets::RegistrationVariant::Guard(false),
                    |_| Ok(()),
                    |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
                )
                .unwrap()
                .0,
            shaders[index]
        );
        let material = owner
            .catalog()
            .shader_binding(shaders[index], second)
            .unwrap();
        assert_eq!(
            owner
                .assets()
                .material(owner.assets().shader_binding(material).unwrap().material)
                .unwrap()
                .stages[0]
                .sampler
                .mipmaps,
            index == 1
        );
        assert_eq!(
            owner.catalog().shader_binding(shaders[index], first),
            Some(materials[index])
        );
    }
    let request = RendererRequest {
        name: b"models/test/skin0.pcx".as_slice(),
        source: second.source_rules,
        kind: RendererRegistration::Skin,
        world: None,
    };
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                second,
                qa_app::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        skin
    );
    assert!(owner.catalog().renderer_skin(skin, second));
    assert!(owner.catalog().renderer_skin(skin, first));
    assert_eq!(
        render_indexed_on(&mut cpu, owner.assets(), original, palette),
        pixels
    );
    assert_eq!(owner.catalog().color_binding(first), colors);
    assert_eq!(owner.catalog().capacity().registrations, 6);
    assert_eq!(owner.catalog().capacity().frame_vertices, 20);
    assert_eq!(owner.catalog().capacity().indices, 12);
    assert_eq!(owner.assets().counts().models, before.models);
}

#[test]
fn late_profile_rollback_preserves_source_shader_first_request_and_new_source_independence() {
    let mut entries = native_palettes();
    entries.extend([
        (
            "ui.part/first.tga",
            support::tga(4, 4, &[[211, 17, 29, 255]; 16]),
        ),
        (
            "ui.part/second.tga",
            support::tga(4, 4, &[[31, 223, 47, 255]; 16]),
        ),
    ]);
    let fixture = support::Fixture::new(&entries);
    let first = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [first],
        [],
        Default::default(),
    )
    .unwrap();
    let shader = owner
        .register_shader(
            &fixture.vfs,
            b"ui.part/first.tga",
            first.source_rules,
            false,
            qa_compat::services::ShaderLighting::TwoDimensional,
        )
        .unwrap();
    let original = owner.catalog().shader_binding(shader, first).unwrap();
    let mut cpu = cpu(&mut owner);
    let before = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    let mut settings = first.image_settings;
    settings.picmip = first.image_settings.picmip + 1;
    assert!(
        owner
            .prepare_profile(
                &fixture.vfs,
                first.source_rules,
                first.rules,
                settings,
                qa_render::material::MaterialFinishPolicy::default(),
                None,
                |_| Err("profile reject".to_owned())
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().capacity(), capacity);
    assert_eq!(
        owner.catalog().shader_binding(shader, first),
        Some(original)
    );
    let (second, _) = owner
        .prepare_profile(
            &fixture.vfs,
            first.source_rules,
            first.rules,
            settings,
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = RendererRequest {
        name: b"ui.part/second.tga".as_slice(),
        source: second.source_rules,
        kind: RendererRegistration::Shader {
            mipmaps: true,
            lighting: qa_compat::services::ShaderLighting::TwoDimensional,
        },
        world: None,
    };
    owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            second,
            qa_app::model_assets::RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let material = owner.catalog().shader_binding(shader, second).unwrap();
    let StageTexture::Image(image) = owner
        .assets()
        .material(owner.assets().shader_binding(material).unwrap().material)
        .unwrap()
        .stages[0]
        .texture
    else {
        panic!("shader texture")
    };
    assert_eq!(
        owner.assets().image(image).unwrap().rgba.as_ref(),
        [211, 17, 29, 255].repeat(16)
    );
    assert!(
        !owner
            .assets()
            .material(owner.assets().shader_binding(material).unwrap().material)
            .unwrap()
            .stages[0]
            .sampler
            .mipmaps
    );
    // A different Source owns its first exact spelling and flags separately.
    let (third, _) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake,
            RuleSetId::Quake3,
            settings,
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = RendererRequest {
        name: b"ui.part/second.tga".as_slice(),
        source: third.source_rules,
        kind: RendererRegistration::Shader {
            mipmaps: true,
            lighting: qa_compat::services::ShaderLighting::TwoDimensional,
        },
        world: None,
    };
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                third,
                qa_app::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |_, _| Err("new Source backend reject".to_owned())
            )
            .is_err()
    );
    assert_eq!(
        owner.catalog().shader_binding(shader, first),
        Some(original)
    );
    assert_eq!(
        owner.catalog().shader_binding(shader, second),
        Some(material)
    );
    assert_eq!(owner.catalog().shader_binding(shader, third), None);
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                third,
                qa_app::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        shader
    );
    let third_material = owner.catalog().shader_binding(shader, third).unwrap();
    let StageTexture::Image(image) = owner
        .assets()
        .material(
            owner
                .assets()
                .shader_binding(third_material)
                .unwrap()
                .material,
        )
        .unwrap()
        .stages[0]
        .texture
    else {
        panic!("shader texture")
    };
    assert_eq!(
        owner.assets().image(image).unwrap().rgba.as_ref(),
        [31, 223, 47, 255].repeat(16)
    );
    assert!(
        owner
            .assets()
            .material(
                owner
                    .assets()
                    .shader_binding(third_material)
                    .unwrap()
                    .material
            )
            .unwrap()
            .stages[0]
            .sampler
            .mipmaps
    );
}

#[test]
fn metadata_only_model_gets_its_first_stable_row_when_a_profile_is_admitted() {
    let fixture = support::Fixture::new(&[("progs/test.mdl", support::mdl())]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let mut owner = ModelAssets::new(&fixture.vfs, assets, [], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered(
                &fixture.vfs,
                [Some(palette), Some(palette), None, None, None],
            ),
        )
        .unwrap();
    owner
        .precache(&fixture.vfs, b"progs/test.mdl", RuleSetId::Quake)
        .unwrap();
    assert!(owner.catalog().metadata(b"progs/test.mdl", None).is_some());
    let handle = (0..owner.catalog().capacity().names)
        .find(|&index| {
            owner
                .catalog()
                .name(qa_core::primitives::NameId(index as u32))
                == Some("progs/test.mdl")
        })
        .unwrap() as i32;
    assert_eq!(owner.catalog().file_handle(b"progs/test.mdl"), None);
    assert_eq!(owner.catalog().capacity().registrations, 0);
    let mut cpu = cpu(&mut owner);
    let (profile, _) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake,
            RuleSetId::Quake,
            ImageSettings::native(RuleSetId::Quake),
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(owner.catalog().capacity().registrations, 1);
    let request = RendererRequest {
        name: b"progs/test.mdl".as_slice(),
        source: profile.source_rules,
        kind: RendererRegistration::Model,
        world: None,
    };
    assert_eq!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                profile,
                qa_app::model_assets::RegistrationVariant::Guard(false),
                |_| Ok(()),
                |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned)
            )
            .unwrap()
            .0,
        handle
    );
    assert!(
        owner
            .catalog()
            .renderer_registration(handle, profile)
            .is_some()
    );
    assert!(binding(owner.catalog(), "progs/test.mdl", profile, 0).is_some());
}

#[test]
fn world_source_palette_is_resolved_inside_its_own_transaction_independently_of_model_profile() {
    use qa_app::{map, renderer::SceneComposer};
    use qa_world::collision::CollisionStore;
    let mut entries = native_palettes();
    entries.push(("maps/cold.bsp", support::world_map(8.0)));
    let fixture = support::Fixture::new(&entries);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [],
        [],
        Default::default(),
    )
    .unwrap();
    let (mut scene, _) =
        SceneComposer::load(Default::default(), owner.assets_mut(), vec![]).unwrap();
    let mut cpu = cpu(&mut owner);
    let (profile, profile_counts) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake2,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(profile.source_palette, Some(PaletteId(0)));
    assert_eq!(profile.palette, None);
    assert_eq!(owner.palettes().get(RuleSetId::Quake, None), None);
    let mut geometry = CollisionStore::new();
    assert!(
        owner
            .prepare_world(
                &fixture.vfs,
                &mut geometry,
                map::read(&fixture.vfs, "cold").unwrap(),
                world_registration(profile),
                |view, _, loaded| scene.stage_world(view.assets, &loaded.render),
                |assets, _| {
                    assert_eq!(assets.counts().palettes, profile_counts.palettes + 1);
                    Err("world backend reject".to_owned())
                }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), profile_counts);
    assert_eq!(owner.palettes().get(RuleSetId::Quake, None), None);
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake2, None),
        profile.source_palette
    );
    assert_eq!(geometry.geometries().count(), 0);
    let (loaded, append, counts) = owner
        .prepare_world(
            &fixture.vfs,
            &mut geometry,
            map::read(&fixture.vfs, "cold").unwrap(),
            world_registration(profile),
            |view, _, loaded| scene.stage_world(view.assets, &loaded.render),
            |assets, append| {
                cpu.prepare_assets(assets, Some(append.limits()))
                    .map_err(str::to_owned)
            },
        )
        .unwrap();
    append.commit();
    assert_eq!(counts.palettes, profile_counts.palettes + 1);
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake, None),
        Some(PaletteId(1))
    );
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake2, None),
        Some(PaletteId(0))
    );
    assert_eq!(loaded.native_source, RuleSetId::Quake);
    assert_eq!(
        loaded.render.presentations[0].profile.rules,
        RuleSetId::Quake3
    );
    assert_eq!(loaded.render.presentations[0].profile.indexed_palette, None);
    assert_eq!(cpu.admitted_assets(), Some(counts));
}

#[test]
fn eager_native_default_profile_rolls_back_backend_failure_and_keeps_published_peer() {
    let fixture = support::Fixture::new(&[]);
    let first = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [first],
        [],
        Default::default(),
    )
    .unwrap();
    let mut cpu = cpu(&mut owner);
    let original = owner.catalog().native_default(first).unwrap();
    let original_image = match owner
        .assets()
        .material(owner.assets().shader_binding(original).unwrap().material)
        .unwrap()
        .stages[0]
        .texture
    {
        StageTexture::Image(id) => id,
        _ => panic!("default image"),
    };
    let pixels = owner
        .assets()
        .image(original_image)
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .levels[0]
        .rgba
        .to_vec();
    let before = owner.assets().counts();
    let mut second = first;
    second.image_settings.gamma_exponent = 1.75;
    assert!(
        owner
            .prepare_profile(
                &fixture.vfs,
                second.source_rules,
                second.rules,
                second.image_settings,
                qa_render::material::MaterialFinishPolicy::default(),
                None,
                |assets| {
                    assert_eq!(assets.counts().images, before.images + 1);
                    assert_eq!(assets.counts().shader_bindings, before.shader_bindings + 1);
                    Err("reject native default backend".to_owned())
                }
            )
            .unwrap_err()
            .contains("reject native default backend")
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(owner.catalog().native_default(first), Some(original));
    assert_eq!(owner.catalog().native_default(second), None);
    let (admitted, counts) = owner
        .prepare_profile(
            &fixture.vfs,
            second.source_rules,
            second.rules,
            second.image_settings,
            qa_render::material::MaterialFinishPolicy::default(),
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(admitted, second);
    assert_eq!(counts.images, before.images + 1);
    assert_ne!(owner.catalog().native_default(second), Some(original));
    assert_eq!(owner.catalog().native_default(first), Some(original));
    assert_eq!(
        owner
            .assets()
            .image(original_image)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .rgba
            .as_ref(),
        pixels
    );
    assert_eq!(cpu.admitted_assets(), Some(counts));
    // Exact admitted profile/cache hit performs no new backend work.
    assert_eq!(
        owner
            .prepare_profile(
                &fixture.vfs,
                second.source_rules,
                second.rules,
                second.image_settings,
                qa_render::material::MaterialFinishPolicy::default(),
                None,
                |_| panic!("cached profile must not reprepare")
            )
            .unwrap(),
        (second, counts)
    );
}

#[test]
fn finish_profiles_share_image_storage_and_guard_variants_publish_atomically() {
    use qa_app::model_assets::RegistrationVariant;
    use qa_compat::services::ShaderLighting;
    use qa_render::{ShaderBindingId, material::MaterialFinishPolicy};
    let fixture = support::Fixture::new(&[
        (
            "textures/shared.tga",
            support::tga(4, 4, &[[71, 137, 193, 255]; 16]),
        ),
        (
            "scripts/finish.shader",
            br#"
            textures/first {
                { map textures/shared tcGen texture }
                { map textures/shared tcGen texture blendFunc filter }
            }
            textures/later {
                { map textures/shared tcGen texture }
                { map textures/shared tcGen texture blendFunc filter }
            }
        "#
            .to_vec(),
        ),
    ]);
    let first = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [first],
        [],
        Default::default(),
    )
    .unwrap();
    let mut cpu = cpu(&mut owner);
    let request = renderer_request(
        "textures/first",
        RendererRegistration::Shader {
            mipmaps: true,
            lighting: ShaderLighting::TwoDimensional,
        },
        first.source_rules,
        None,
    );
    let (handle, plain, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            first,
            RegistrationVariant::Guard(false),
            |candidate| Ok(candidate.registrations[0]),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let image = |assets: &Assets, shader: ShaderBindingId| {
        let material = assets
            .material(assets.shader_binding(shader).unwrap().material)
            .unwrap();
        let StageTexture::Image(image) = material.stages[0].texture else {
            panic!("authored image")
        };
        image
    };
    let shared_image = image(owner.assets(), plain.shader);
    assert_eq!(
        owner
            .assets()
            .material(
                owner
                    .assets()
                    .shader_binding(plain.shader)
                    .unwrap()
                    .material
            )
            .unwrap()
            .stages
            .len(),
        2
    );
    let image_prefix = owner.assets().counts().images;
    let source_pixels = owner.assets().image(shared_image).unwrap().rgba.to_vec();
    fixture.vfs.take_lookup_count();
    let plain_counts = owner.assets().counts();
    owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            first,
            RegistrationVariant::Guard(true),
            |candidate| {
                assert_eq!(candidate.registrations[0], plain);
                Ok(())
            },
            |_, _| panic!("irrelevant fullscreen guard must not prepare"),
        )
        .unwrap();
    assert_eq!(owner.assets().counts(), plain_counts);
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let finish = MaterialFinishPolicy {
        vertex_light: true,
        ..Default::default()
    };
    let (vertex, _) = owner
        .prepare_profile(
            &fixture.vfs,
            first.source_rules,
            first.rules,
            first.image_settings,
            finish,
            None,
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_ne!(vertex, first);
    assert_eq!(vertex.image_settings, first.image_settings);
    assert_eq!(owner.assets().counts().images, image_prefix);
    let (vertex_handle, collapsed, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            vertex,
            RegistrationVariant::Guard(false),
            |candidate| Ok(candidate.registrations[0]),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(
        vertex_handle, handle,
        "native name handles survive exact profile changes"
    );
    assert_ne!(collapsed.shader, plain.shader);
    assert_eq!(image(owner.assets(), collapsed.shader), shared_image);
    assert_eq!(owner.assets().counts().images, image_prefix);
    assert_eq!(
        owner
            .assets()
            .material(
                owner
                    .assets()
                    .shader_binding(collapsed.shader)
                    .unwrap()
                    .material
            )
            .unwrap()
            .stages
            .len(),
        1
    );
    let before = owner.assets().counts();
    fixture.vfs.take_lookup_count();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &request,
                vertex,
                RegistrationVariant::Guard(true),
                |candidate| Ok(candidate.registrations[0]),
                |assets, candidate| {
                    assert_ne!(candidate.shader, collapsed.shader);
                    assert_eq!(
                        assets
                            .material(assets.shader_binding(candidate.shader).unwrap().material)
                            .unwrap()
                            .stages
                            .len(),
                        2
                    );
                    Err("backend rejected fullscreen variant".to_owned())
                }
            )
            .is_err()
    );
    assert_eq!(owner.assets().counts(), before);
    assert_eq!(cpu.admitted_assets(), Some(before));
    assert_eq!(
        owner.catalog().shader_binding(handle, vertex),
        Some(collapsed.shader)
    );
    assert_eq!(
        owner.catalog().shader_binding(handle, first),
        Some(plain.shader)
    );
    let (retried, fullscreen, counts) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            vertex,
            RegistrationVariant::Guard(true),
            |candidate| Ok(candidate.registrations[0]),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(retried, handle);
    assert_eq!(image(owner.assets(), fullscreen.shader), shared_image);
    assert_ne!(fullscreen.shader, collapsed.shader);
    assert_eq!(owner.assets().counts().images, image_prefix);
    assert_eq!(cpu.admitted_assets(), Some(counts));
    assert_eq!(
        owner.assets().image(shared_image).unwrap().rgba.as_ref(),
        source_pixels
    );
    fixture.vfs.take_lookup_count();
    let (retained_handle, retained, retained_counts) = owner
        .prepare_registration(
            &fixture.vfs,
            &request,
            vertex,
            RegistrationVariant::Retained,
            |candidate| Ok(candidate.registrations[0]),
            |_, _| panic!("logical hit must not prepare a backend"),
        )
        .unwrap();
    assert_eq!(retained_handle, handle);
    assert_eq!(
        retained.shader, collapsed.shader,
        "a logical hit retains its first body after a guard flip"
    );
    assert_eq!(retained_counts, counts);
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let later = renderer_request("textures/later", request.kind, vertex.source_rules, None);
    fixture.vfs.take_lookup_count();
    assert!(
        owner
            .prepare_registration(
                &fixture.vfs,
                &later,
                vertex,
                RegistrationVariant::Retained,
                |_| Ok(()),
                |_, _| panic!("logical miss cannot compile through Retained")
            )
            .is_err()
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let (_, later_row, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &later,
            vertex,
            RegistrationVariant::Guard(true),
            |candidate| Ok(candidate.registrations[0]),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(
        owner
            .assets()
            .material(
                owner
                    .assets()
                    .shader_binding(later_row.shader)
                    .unwrap()
                    .material
            )
            .unwrap()
            .stages
            .len(),
        2
    );
    assert_eq!(image(owner.assets(), later_row.shader), shared_image);
    fixture.vfs.take_lookup_count();
    owner
        .prepare_registration(
            &fixture.vfs,
            &later,
            vertex,
            RegistrationVariant::Retained,
            |candidate| {
                assert_eq!(candidate.registrations[0], later_row);
                Ok(())
            },
            |_, _| panic!("admitted logical hit must not prepare"),
        )
        .unwrap();
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}

#[test]
fn first_fullscreen_shader_and_skin_remain_visible_to_frozen_source_lookup() {
    use qa_app::model_assets::RegistrationVariant;
    use qa_compat::services::ShaderLighting;
    let fixture = support::Fixture::surfaces();
    let mut profile = shader_profile(RuleSetId::Quake3);
    profile.finish.vertex_light = true;
    let mut owner = ModelAssets::new(
        &fixture.vfs,
        Assets::load().unwrap(),
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    let mut cpu = cpu(&mut owner);
    let shader_request = renderer_request(
        "textures/glow",
        RendererRegistration::Shader {
            mipmaps: true,
            lighting: ShaderLighting::TwoDimensional,
        },
        profile.source_rules,
        None,
    );
    let (shader, _, _) = owner
        .prepare_registration(
            &fixture.vfs,
            &shader_request,
            profile,
            RegistrationVariant::Guard(true),
            |candidate| {
                assert_eq!(candidate.registrations.len(), 1);
                Ok(())
            },
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert!(shader > 0);
    let skin_request = renderer_request(
        "models/test.skin",
        RendererRegistration::Skin,
        profile.source_rules,
        None,
    );
    let (skin, _, counts) = owner
        .prepare_registration(
            &fixture.vfs,
            &skin_request,
            profile,
            RegistrationVariant::Guard(true),
            |candidate| {
                assert_eq!(candidate.registrations.len(), 3);
                Ok(())
            },
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert!(skin > 0);
    assert!(owner.catalog().shader_binding(shader, profile).is_some());
    assert!(owner.catalog().renderer_skin(skin, profile));
    fixture.vfs.take_lookup_count();
    let frozen: &dyn ModelLookup = owner.catalog();
    assert_eq!(
        frozen.shader_handle(b"textures/glow", profile.source_rules),
        Some(shader)
    );
    assert_eq!(
        frozen.skin_handle(b"models/test.skin", profile.source_rules),
        Some(skin)
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert_eq!(owner.assets().counts(), counts);
    assert_eq!(cpu.admitted_assets(), Some(counts));
}
