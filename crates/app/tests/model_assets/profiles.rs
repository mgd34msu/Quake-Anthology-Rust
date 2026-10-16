#[path = "world_presentations.rs"]
mod world_presentations;

use super::*;
use qa_compat::services::{RendererRegistration, RendererRequest};
use qa_render::{
    cpu::{CpuBackend, CpuLimits},
    material::resources::ImageSettings,
};

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
    let mut owner = ModelAssets::new(Assets::load().unwrap(), [], [], Default::default()).unwrap();
    let mut cpu = cpu(&mut owner);
    let before = owner.assets().counts();
    let capacity = owner.catalog().capacity();
    let settings = ImageSettings::native(RuleSetId::Quake);
    let result = owner.prepare_profile(
        &fixture.vfs,
        RuleSetId::Quake2,
        RuleSetId::Quake,
        settings,
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
        assert_eq!(owner.palettes().get(source), None);
    }
    assert_eq!(cpu.admitted_assets(), Some(before));
    let (profile, counts) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake2,
            RuleSetId::Quake,
            settings,
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
        owner.palettes().get(RuleSetId::Quake2Rerelease)
    );
    assert_eq!(profile.palette, owner.palettes().get(RuleSetId::QuakeWorld));
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
            qa_app::render_settings::PaletteRegistrations::from_registered([
                None,
                None,
                Some(palette),
                Some(palette),
                None,
            ]),
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
    let sprite_models = [0, 1].map(|frame| {
        owner
            .catalog()
            .bindings(
                owner
                    .catalog()
                    .renderer_registration(sprite, first)
                    .unwrap(),
                0,
                ModelMaterialPolicy {
                    frame,
                    ..Default::default()
                },
            )
            .unwrap()
            .next()
            .unwrap()
            .model
    });
    let registrations = models.map(|id| owner.catalog().renderer_registration(id, first).unwrap());
    let original = binding(owner.catalog(), "models/a.md2", first, 0).unwrap();
    let colors = owner.catalog().color_binding(first);
    let shaders = [
        (b"ui_a.part/first.tga".as_slice(), false),
        (b"ui_b.part/first.tga".as_slice(), true),
    ]
    .map(|(path, mips)| {
        owner
            .register_shader(&fixture.vfs, path, first.source_rules, mips)
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
                .prepare_registration(&fixture.vfs, &request, second, |assets| cpu
                    .prepare_assets(assets, None)
                    .map_err(str::to_owned))
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
            .prepare_registration(&fixture.vfs, &request, second, |assets| cpu
                .prepare_assets(assets, None)
                .map_err(str::to_owned))
            .unwrap()
            .0,
        sprite
    );
    assert_eq!(
        [0, 1].map(|frame| owner
            .catalog()
            .bindings(
                owner
                    .catalog()
                    .renderer_registration(sprite, second)
                    .unwrap(),
                0,
                ModelMaterialPolicy {
                    frame,
                    ..Default::default()
                }
            )
            .unwrap()
            .next()
            .unwrap()
            .model),
        sprite_models
    );
    for (index, path) in ["ui_a.part/unavailable.tga", "ui_b.part/unavailable.tga"]
        .into_iter()
        .enumerate()
    {
        let request = RendererRequest {
            name: path.as_bytes(),
            source: second.source_rules,
            kind: RendererRegistration::Shader {
                mipmaps: index == 0,
            },
            world: None,
        };
        assert_eq!(
            owner
                .prepare_registration(&fixture.vfs, &request, second, |assets| cpu
                    .prepare_assets(assets, None)
                    .map_err(str::to_owned))
                .unwrap()
                .0,
            shaders[index]
        );
        let material = owner
            .catalog()
            .shader_binding(shaders[index], second)
            .unwrap();
        assert_eq!(
            owner.assets().material(material).unwrap().stages[0]
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
            .prepare_registration(&fixture.vfs, &request, second, |assets| cpu
                .prepare_assets(assets, None)
                .map_err(str::to_owned))
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
    let mut owner =
        ModelAssets::new(Assets::load().unwrap(), [first], [], Default::default()).unwrap();
    let shader = owner
        .register_shader(
            &fixture.vfs,
            b"ui.part/first.tga",
            first.source_rules,
            false,
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
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = RendererRequest {
        name: b"ui.part/second.tga".as_slice(),
        source: second.source_rules,
        kind: RendererRegistration::Shader { mipmaps: true },
        world: None,
    };
    owner
        .prepare_registration(&fixture.vfs, &request, second, |assets| {
            cpu.prepare_assets(assets, None).map_err(str::to_owned)
        })
        .unwrap();
    let material = owner.catalog().shader_binding(shader, second).unwrap();
    let StageTexture::Image(image) = owner.assets().material(material).unwrap().stages[0].texture
    else {
        panic!("shader texture")
    };
    assert_eq!(
        owner.assets().image(image).unwrap().rgba.as_ref(),
        [211, 17, 29, 255].repeat(16)
    );
    assert!(
        !owner.assets().material(material).unwrap().stages[0]
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
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let request = RendererRequest {
        name: b"ui.part/second.tga".as_slice(),
        source: third.source_rules,
        kind: RendererRegistration::Shader { mipmaps: true },
        world: None,
    };
    assert!(
        owner
            .prepare_registration(&fixture.vfs, &request, third, |_| Err(
                "new Source backend reject".to_owned()
            ))
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
            .prepare_registration(&fixture.vfs, &request, third, |assets| cpu
                .prepare_assets(assets, None)
                .map_err(str::to_owned))
            .unwrap()
            .0,
        shader
    );
    let third_material = owner.catalog().shader_binding(shader, third).unwrap();
    let StageTexture::Image(image) =
        owner.assets().material(third_material).unwrap().stages[0].texture
    else {
        panic!("shader texture")
    };
    assert_eq!(
        owner.assets().image(image).unwrap().rgba.as_ref(),
        [31, 223, 47, 255].repeat(16)
    );
    assert!(
        owner.assets().material(third_material).unwrap().stages[0]
            .sampler
            .mipmaps
    );
}

#[test]
fn metadata_only_model_gets_its_first_stable_row_when_a_profile_is_admitted() {
    let fixture = support::Fixture::new(&[("progs/test.mdl", support::mdl())]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let mut owner = ModelAssets::new(assets, [], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                Some(palette),
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    owner
        .precache(&fixture.vfs, b"progs/test.mdl", RuleSetId::Quake)
        .unwrap();
    assert!(owner.catalog().metadata(b"progs/test.mdl").is_some());
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
            .prepare_registration(&fixture.vfs, &request, profile, |assets| cpu
                .prepare_assets(assets, None)
                .map_err(str::to_owned))
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
    let mut owner = ModelAssets::new(Assets::load().unwrap(), [], [], Default::default()).unwrap();
    let (mut scene, _) =
        SceneComposer::load(Default::default(), owner.assets_mut(), vec![]).unwrap();
    let mut cpu = cpu(&mut owner);
    let (profile, profile_counts) = owner
        .prepare_profile(
            &fixture.vfs,
            RuleSetId::Quake2,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            |assets| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_eq!(profile.source_palette, Some(PaletteId(0)));
    assert_eq!(profile.palette, None);
    assert_eq!(owner.palettes().get(RuleSetId::Quake), None);
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
    assert_eq!(owner.palettes().get(RuleSetId::Quake), None);
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake2),
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
    assert_eq!(owner.palettes().get(RuleSetId::Quake), Some(PaletteId(1)));
    assert_eq!(owner.palettes().get(RuleSetId::Quake2), Some(PaletteId(0)));
    assert_eq!(loaded.native_source, RuleSetId::Quake);
    assert_eq!(
        loaded.render.presentations[0].profile.rules,
        RuleSetId::Quake3
    );
    assert_eq!(loaded.render.presentations[0].profile.indexed_palette, None);
    assert_eq!(cpu.admitted_assets(), Some(counts));
}
