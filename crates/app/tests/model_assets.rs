#[path = "model_assets/support.rs"]
mod support;

use qa_app::model_assets::{
    ModelCatalog, ModelCatalogBuilder, ModelLimits, ModelMaterialPolicy, ModelProfile, ModelRequest,
};
use qa_compat::services::{ModelLoader, ModelLookup};
use qa_core::primitives::{Bounds, RuleSetId, Vec3};
use qa_render::assets::{SoftwareAlias, StageTexture};
use qa_render::shader::{AlphaGen, RgbGen, StageBlend};
use qa_render::world::{WorldId, WorldPresentationId};
use qa_render::{Assets, PaletteId};

fn load(
    fixture: &support::Fixture,
    assets: Assets,
    requests: &[ModelRequest<'_>],
    limits: ModelLimits,
) -> Result<(Assets, ModelCatalog), String> {
    ModelCatalog::load(&fixture.vfs, assets, requests.iter().copied(), [], limits)
}

fn native_profile(rules: RuleSetId, palette: PaletteId) -> ModelProfile {
    ModelRequest::native("", rules, Some(palette)).profile()
}

#[test]
fn foreign_presentation_colors_bind_and_generate_missing_blends_at_load() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let source = support::palette(&mut assets);
    let target = support::shifted_palette(&mut assets, 100);
    let mut foreign = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(source));
    foreign.rules = RuleSetId::Quake;
    foreign.palette = Some(target);
    foreign.image_settings = qa_render::material::resources::ImageSettings::native(foreign.rules);
    assert!(assets.palette(target).unwrap().translucency_map().is_none());
    let (assets, catalog) = load(&fixture, assets, &[foreign], ModelLimits::default()).unwrap();
    fixture.remove("models/test/tris.md2");
    let binding = catalog.color_binding(foreign.profile()).unwrap();
    let colors = catalog.colors(binding).unwrap();
    assert_eq!((colors.source, colors.target), (source, target));
    assert_eq!(colors.indices[33], 189);
    let palette = assets.palette(target).unwrap();
    let table = palette.translucency_map().unwrap();
    for (major, minor) in [(0u8, 0u8), (33, 44), (77, 22), (200, 255)] {
        let a = palette.color(major).to_le_bytes();
        let b = palette.color(minor).to_le_bytes();
        let rgb =
            std::array::from_fn(|axis| ((u16::from(a[axis]) * 2 + u16::from(b[axis])) / 3) as u8);
        assert_eq!(
            table[usize::from(major) * 256 + usize::from(minor)],
            palette.nearest_index(rgb, None)
        );
    }
}

#[test]
fn sp2_frames_register_once_and_feed_shared_billboards_with_authored_offsets() {
    use qa_render::{Command, FrontEnd, Limits, Refdef, SceneEntity, Viewport};
    let fixture = support::Fixture::new(&[
        ("sprites/test.sp2", support::sp2()),
        ("sprites/first.pcx", support::pcx(11)),
        ("sprites/second.pcx", support::pcx(22)),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("sprites/test.sp2", RuleSetId::Quake2, Some(palette));
    let (assets, catalog) = load(&fixture, assets, &[request], ModelLimits::default()).unwrap();
    assert!(catalog.diagnostics.is_empty());
    assert_eq!(catalog.capacity().surfaces, 1);
    assert_eq!(catalog.capacity().skins, 2);
    fixture.remove("sprites/test.sp2");
    fixture.remove("sprites/first.pcx");
    fixture.remove("sprites/second.pcx");
    let registration = catalog
        .registration(request.path, request.profile())
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut cpu = qa_render::cpu::CpuBackend::load(32, 32).unwrap();
    for frame_number in [0, 1, 2] {
        let binding = catalog
            .bindings(
                registration,
                99,
                ModelMaterialPolicy {
                    frame: frame_number,
                    ..Default::default()
                },
            )
            .unwrap()
            .next()
            .unwrap();
        let geometry = assets.model(binding.model).unwrap().sprite.unwrap();
        let expected = if frame_number % 2 == 0 {
            [-2., 6., 3., -1.]
        } else {
            [4., 16., 3., -3.]
        };
        assert_eq!(geometry.extents, expected);
        let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_entity(SceneEntity {
            model: binding.model.into(),
            material: Some(binding.material),
            origin: Vec3([16., 0., 0.]),
            ..Default::default()
        }));
        let refdef = Refdef {
            clip_plane: Some(qa_core::primitives::Plane::oriented(Vec3([1., 0., 0.]), 4.)),
            viewport: Viewport {
                width: 32,
                height: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(frame.render_scene(refdef, &[], &assets));
        let packet = frame.finish();
        let view = packet
            .commands()
            .iter()
            .find_map(|cmd| {
                if let Command::View(view) = cmd {
                    Some(view)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(view.scene.polys.count, 2);
        let poly = packet.poly(view.scene.polys.first);
        let inputs = qa_render::stage::DrawInputs::for_view(&refdef).for_poly(poly, &packet);
        assert_eq!(inputs.view_origin, refdef.origin);
        assert_eq!(inputs.clip_plane, refdef.clip_plane);
        assert_eq!(
            packet.vertices(poly.vertices)[0].position,
            Vec3([16., -expected[0], expected[2]])
        );
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert!(
            cpu.pixels()
                .iter()
                .any(|&p| p != u32::from_le_bytes([0, 0, 0, 255]))
        );
        assert!(front.recycle(packet).is_ok());
    }
}

#[test]
fn retail_rerelease_sp2_uses_the_existing_archive_image_and_model_paths() {
    let Some(root) = std::env::var_os("QA_Q2_RERELEASE_ROOT") else {
        return;
    };
    let mut vfs = qa_content::vfs::Vfs::default();
    vfs.mount_product(std::path::Path::new(&root), 0).unwrap();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let native = ModelRequest::native(
        "sprites/s_bfg1.sp2",
        RuleSetId::Quake2Rerelease,
        Some(palette),
    );
    let mut foreign = native;
    foreign.rules = RuleSetId::Quake3;
    foreign.palette = None;
    foreign.image_settings = qa_render::material::resources::ImageSettings::native(foreign.rules);
    let (assets, catalog) =
        ModelCatalog::load(&vfs, assets, [native, foreign], [], ModelLimits::default()).unwrap();
    assert!(catalog.diagnostics.is_empty());
    for request in [native, foreign] {
        let registration = catalog
            .registration(request.path, request.profile())
            .unwrap();
        let mut frame_materials = vec![];
        for frame in [0, 1, 2] {
            let binding = catalog
                .bindings(
                    registration,
                    0,
                    ModelMaterialPolicy {
                        frame,
                        ..Default::default()
                    },
                )
                .unwrap()
                .next()
                .unwrap();
            let model = assets.model(binding.model).unwrap();
            assert!(model.sprite.is_some());
            let material = assets.material(binding.material).unwrap();
            assert_eq!(
                material.stages[0].alpha_test,
                qa_render::shader::AlphaFunc::AtLeastHalf
            );
            let StageTexture::Image(image) = material.stages[0].texture else {
                panic!("sprite image")
            };
            assert!(assets.image(image).unwrap().width > 0);
            frame_materials.push(binding.material);
        }
        assert_ne!(frame_materials[0], frame_materials[1]);
        assert_eq!(frame_materials[0], frame_materials[2]);
    }
}

fn shader_profile(presentation: RuleSetId) -> ModelProfile {
    ModelProfile {
        source_rules: RuleSetId::Quake3,
        rules: presentation,
        source_palette: None,
        palette: None,
        image_settings: qa_render::material::resources::ImageSettings::native(presentation),
    }
}

#[test]
fn native_skins_resolve_once_for_each_profile_and_preserve_first_mapping() {
    for skin_first in [false, true] {
        let fixture = support::Fixture::surfaces();
        let profiles = [
            shader_profile(RuleSetId::Quake3),
            shader_profile(RuleSetId::Quake2),
        ];
        let mut builder =
            ModelCatalogBuilder::new(Assets::load().unwrap(), profiles, [], Default::default())
                .unwrap();
        let before = skin_first.then(|| {
            builder
                .register_skin(&fixture.vfs, b"models/test.skin", RuleSetId::Quake3)
                .unwrap()
        });
        let model = builder
            .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
            .unwrap();
        let skin = before.unwrap_or_else(|| {
            builder
                .register_skin(&fixture.vfs, b"models/test.skin", RuleSetId::Quake3)
                .unwrap()
        });
        assert!(skin > 0);
        fixture.remove("models/test.skin");
        assert_eq!(
            builder.register_skin(&fixture.vfs, b"MODELS\\TEST.SKIN", RuleSetId::Quake3),
            Ok(skin)
        );
        let (assets, catalog) = builder.finish();
        assert_eq!(catalog.skin_handle(b"models/test.skin"), Some(skin));
        let mut profile_materials = Vec::new();
        for profile in profiles {
            let registration = catalog.renderer_registration(model, profile).unwrap();
            let base = catalog
                .bindings(registration, 0, Default::default())
                .unwrap()
                .collect::<Vec<_>>();
            let skinned = catalog
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
            assert_eq!(skinned.len(), 2);
            assert_eq!(
                skinned
                    .iter()
                    .map(|binding| binding.model)
                    .collect::<Vec<_>>(),
                base.iter().map(|binding| binding.model).collect::<Vec<_>>()
            );
            // BODY_1 resolves to the first body mapping, not the later duplicate.
            let sample = |material| {
                let StageTexture::Image(image) =
                    assets.material(material).unwrap().stages[0].texture
                else {
                    panic!("skin image")
                };
                let pixels = &assets.image(image).unwrap().rgba;
                [pixels[0], pixels[1], pixels[2]]
            };
            let first = sample(skinned[0].material);
            assert!(first[1] > first[0] && first[1] > first[2]);
            let second = sample(skinned[1].material);
            assert!(second[0] > second[1] && second[0] > second[2]);
            profile_materials.push(skinned[0].material);
            for invalid in [i32::MIN, -1, 0, i32::MAX] {
                assert_eq!(
                    catalog
                        .bindings(
                            registration,
                            0,
                            ModelMaterialPolicy {
                                custom_skin: invalid,
                                ..Default::default()
                            }
                        )
                        .unwrap()
                        .collect::<Vec<_>>(),
                    base
                );
            }
        }
        assert_ne!(profile_materials[0], profile_materials[1]);
    }
}

#[test]
fn failed_skin_slots_stay_failed_and_unmatched_surfaces_use_the_default_shader() {
    let fixture = support::Fixture::new(&[
        ("models/test.md3", support::md3_surfaces()),
        (
            "models/unmatched.skin",
            b"different,textures/missing\n".to_vec(),
        ),
        ("models/empty.skin", b"tag_weapon,\n".to_vec()),
        (
            "textures/body.tga",
            support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
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
    let mut builder =
        ModelCatalogBuilder::new(Assets::load().unwrap(), [profile], [], Default::default())
            .unwrap();
    assert_eq!(
        builder.register_skin(&fixture.vfs, b"models/empty.skin", RuleSetId::Quake3),
        Err(qa_compat::services::CallError::File)
    );
    assert_eq!(
        builder.register_skin(&fixture.vfs, b"MODELS/EMPTY.SKIN", RuleSetId::Quake3),
        Ok(0)
    );
    let skin = builder
        .register_skin(&fixture.vfs, b"models/unmatched.skin", RuleSetId::Quake3)
        .unwrap();
    let model = builder
        .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
        .unwrap();
    let (_, catalog) = builder.finish();
    let registration = catalog.renderer_registration(model, profile).unwrap();
    for custom_skin in [skin, 1] {
        assert!(
            catalog
                .bindings(
                    registration,
                    0,
                    ModelMaterialPolicy {
                        custom_skin,
                        ..Default::default()
                    }
                )
                .unwrap()
                .all(|binding| binding.material == qa_render::MaterialId(0))
        );
    }
}

#[test]
fn native_model_handles_share_file_registration_and_bounds() {
    let bytes = support::md2_skin(b"textures/body.tga");
    let parsed = qa_formats::model::Model::parse(&bytes).unwrap();
    let native_bounds = parsed.bounds;
    let fixture = support::Fixture::new(&[
        ("models/test.md2", bytes),
        (
            "textures/body.tga",
            support::tga(2, 2, &[[80, 30, 10, 255]; 4]),
        ),
    ]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [profile],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md2", RuleSetId::Quake3)
        .unwrap();
    assert!(handle > 0);
    assert_eq!(builder.model_bounds(handle), Some(native_bounds));
    assert!(!builder.model_has_tags(handle));
    fixture.remove("models/test.md2");
    fixture.remove("textures/body.tga");
    assert_eq!(
        builder.register_model(&fixture.vfs, b"MODELS\\TEST.MD2", RuleSetId::Quake3),
        Ok(handle)
    );
    let (_, catalog) = builder.finish();
    assert_eq!(catalog.capacity().registrations, 1);
    assert_eq!(
        catalog.model_handle(b"models/test.md2", RuleSetId::Quake3),
        Some(handle)
    );
    assert_eq!(
        catalog.model_handle(b"models/test.md2", RuleSetId::Quake2),
        None
    );
    assert_eq!(catalog.model_bounds(handle), Some(native_bounds));
    assert!(!catalog.model_has_tags(handle));
    assert_eq!(catalog.model_bounds(0), None);
    assert_eq!(catalog.model_bounds(-1), None);
    assert_eq!(catalog.model_bounds(i32::MAX), None);
    assert!(catalog.registration("models/test.md2", profile).is_some());
    assert_eq!(
        catalog.renderer_registration(handle, profile),
        catalog.registration("models/test.md2", profile)
    );
    assert!(catalog.renderer_registration(0, profile).is_none());
    assert!(catalog.renderer_registration(-1, profile).is_none());
    assert!(catalog.renderer_registration(i32::MAX, profile).is_none());
    let mut other = profile;
    other.source_rules = RuleSetId::Quake2;
    assert!(catalog.renderer_registration(handle, other).is_none());
}

#[test]
fn md3_surfaces_share_geometry_and_keep_per_surface_shader_selection() {
    let fixture = support::Fixture::surfaces();
    let profiles = [
        shader_profile(RuleSetId::Quake3),
        shader_profile(RuleSetId::Quake2),
    ];
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        profiles,
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
        .unwrap();
    let bytes = support::md3_surfaces();
    let parsed = qa_formats::model::Model::parse(&bytes).unwrap();
    assert_eq!(builder.model_bounds(handle), Some(parsed.frames[0].bounds));
    fixture.remove("models/test.md3");
    for name in [
        "textures/body.tga",
        "textures/alternate.tga",
        "textures/glow.tga",
        "scripts/test.shader",
    ] {
        fixture.remove(name);
    }
    assert_eq!(
        builder.register_model(&fixture.vfs, b"MODELS\\TEST.MD3", RuleSetId::Quake3),
        Ok(handle)
    );
    let (assets, catalog) = builder.finish();
    assert_eq!(assets.models().len(), 3);
    assert_eq!(catalog.capacity().surfaces, 4);
    assert_eq!(catalog.capacity().skins, 6);
    let mut shared = None;
    for profile in profiles {
        let registration = catalog.registration("models/test.md3", profile).unwrap();
        let selected = |skin| {
            catalog
                .bindings(registration, skin, ModelMaterialPolicy::default())
                .unwrap()
                .collect::<Vec<_>>()
        };
        let first = selected(0);
        let wrapped = selected(3);
        assert_eq!(first.len(), 2);
        assert_eq!(wrapped.len(), 2);
        assert_ne!(first[0].model, first[1].model);
        assert_eq!(first[0].model, wrapped[0].model);
        assert_ne!(first[0].material, wrapped[0].material);
        assert_eq!(first[1], wrapped[1]);
        assert_eq!(selected(u32::MAX), wrapped);
        for (binding, decoded) in first.iter().zip(&parsed.meshes) {
            let mesh = assets.model(binding.model).unwrap();
            assert_eq!(mesh.frame_count(), 2);
            assert_eq!(mesh.vertices.len(), decoded.vertices.len());
            for (vertex, original) in mesh.vertices.iter().zip(&decoded.vertices) {
                assert_eq!(vertex.position, original.position);
                assert_eq!(vertex.normal, original.normal);
            }
        }
        let opaque = assets.material(first[0].material).unwrap();
        assert_eq!(opaque.stages.len(), 1);
        assert_eq!(opaque.stages[0].rgb_gen, RgbGen::LightingDiffuse);
        assert_eq!(opaque.stages[0].blend, None);
        assert!(opaque.stages[0].depth_write);
        assert_eq!(
            opaque.stages[0].depth_func,
            qa_render::assets::DepthFunc::Lequal
        );
        let scripted = assets.material(first[1].material).unwrap();
        assert_eq!(scripted.stages.len(), 2);
        assert!(matches!(scripted.stages[0].rgb_gen, RgbGen::Wave(_)));
        assert!(scripted.stages[1].blend.is_some());
        let meshes = first
            .iter()
            .map(|binding| binding.model)
            .collect::<Vec<_>>();
        if let Some(previous) = shared.replace(meshes.clone()) {
            assert_eq!(meshes, previous);
        }
    }
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn md3_tag_only_models_register_without_a_mesh_placeholder() {
    let mut bytes = support::md3();
    let at = i32::from_le_bytes(bytes[100..104].try_into().unwrap());
    bytes[84..88].copy_from_slice(&0i32.to_le_bytes());
    bytes[104..108].copy_from_slice(&at.to_le_bytes());
    bytes.truncate(at as usize);
    let bounds = qa_formats::model::Model::parse(&bytes).unwrap().frames[0].bounds;
    let fixture = support::Fixture::new(&[("models/hand.md3", bytes)]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [profile],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/hand.md3", RuleSetId::Quake3)
        .unwrap();
    let (assets, catalog) = builder.finish();
    assert_eq!(catalog.model_bounds(handle), Some(bounds));
    assert_eq!(assets.models().len(), 1);
    assert_eq!(
        catalog
            .bindings(
                catalog.registration("models/hand.md3", profile).unwrap(),
                0,
                ModelMaterialPolicy::default()
            )
            .unwrap()
            .len(),
        0
    );
    assert_eq!(catalog.capacity().frame_vertices, 0);
    assert_eq!(catalog.capacity().surfaces, 0);
}

#[test]
fn model_tags_keep_exact_names_native_frame_clamps_and_frozen_poses() {
    let fixture = support::Fixture::surfaces();
    let profile = shader_profile(RuleSetId::Quake3);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [profile],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
        .unwrap();
    let bytes = support::md3_surfaces();
    let decoded = qa_formats::model::Model::parse(&bytes).unwrap();
    let start = decoded.tags[0].orientation;
    let end = decoded.tags[1].orientation;
    let expected = start.interpolate(end, 0.25);
    assert!(builder.model_has_tags(handle));
    assert_eq!(
        builder.lerp_tag(handle, 0, 1, 0.25, b"tag_weapon"),
        Some(expected)
    );
    fixture.remove("models/test.md3");
    let (_, catalog) = builder.finish();
    assert_eq!(catalog.capacity().tags, 2);
    assert!(catalog.model_has_tags(handle));
    assert_eq!(
        catalog.lerp_tag(handle, 0, 1, 0.25, b"tag_weapon"),
        Some(expected)
    );
    assert_eq!(
        catalog.lerp_tag(handle, 99, i32::MAX, 0.5, b"tag_weapon"),
        Some(end.interpolate(end, 0.5))
    );
    for (start_frame, end_frame, name) in [
        (-1, 0, b"tag_weapon".as_slice()),
        (0, -1, b"tag_weapon"),
        (0, 1, b"TAG_WEAPON"),
        (0, 1, b"missing"),
        (0, 1, b""),
    ] {
        assert_eq!(
            catalog.lerp_tag(handle, start_frame, end_frame, 0.5, name),
            None
        );
    }
    for handle in [0, -1, i32::MAX] {
        assert_eq!(catalog.lerp_tag(handle, 0, 1, 0.5, b"tag_weapon"), None);
    }
    assert_ne!(expected.origin, start.origin);
    assert_ne!(expected.axes, start.axes);
}

#[test]
fn tag_lookup_does_not_fold_names_between_frames() {
    let mut bytes = support::md3_surfaces();
    let tags = i32::from_le_bytes(bytes[96..100].try_into().unwrap()) as usize;
    bytes[tags + 112..tags + 176].fill(0);
    bytes[tags + 112..tags + 122].copy_from_slice(b"TAG_WEAPON");
    let fixture = support::Fixture::with_surfaces(bytes);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [profile],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
        .unwrap();
    assert!(builder.lerp_tag(handle, 0, 0, 0.5, b"tag_weapon").is_some());
    assert!(builder.lerp_tag(handle, 1, 1, 0.5, b"TAG_WEAPON").is_some());
    assert_eq!(builder.lerp_tag(handle, 0, 1, 0.5, b"tag_weapon"), None);
    assert_eq!(builder.lerp_tag(handle, 0, 1, 0.5, b"TAG_WEAPON"), None);
}

#[test]
fn tag_name_index_preserves_the_first_duplicate_in_each_frame() {
    let original = support::md3_surfaces();
    let tags = i32::from_le_bytes(original[96..100].try_into().unwrap()) as usize;
    let mut bytes = original[..tags].to_vec();
    for frame in 0..2 {
        let tag = &original[tags + frame * 112..tags + (frame + 1) * 112];
        bytes.extend(tag);
        let mut duplicate = tag.to_vec();
        duplicate[64..68].copy_from_slice(&999.0f32.to_le_bytes());
        bytes.extend(duplicate);
    }
    bytes.extend(&original[tags + 224..]);
    bytes[80..84].copy_from_slice(&2i32.to_le_bytes());
    for offset in [100, 104] {
        let value = i32::from_le_bytes(original[offset..offset + 4].try_into().unwrap());
        bytes[offset..offset + 4].copy_from_slice(&(value + 224).to_le_bytes());
    }
    let parsed = qa_formats::model::Model::parse(&original).unwrap();
    let fixture = support::Fixture::with_surfaces(bytes);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [shader_profile(RuleSetId::Quake3)],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3)
        .unwrap();
    assert_eq!(
        builder.lerp_tag(handle, 0, 1, 0.5, b"tag_weapon"),
        Some(
            parsed.tags[0]
                .orientation
                .interpolate(parsed.tags[1].orientation, 0.5)
        )
    );
}

#[test]
fn native_model_admission_rejects_exhausted_tag_storage() {
    let fixture = support::Fixture::surfaces();
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [shader_profile(RuleSetId::Quake3)],
        [],
        ModelLimits {
            tags: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        builder.register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3),
        Err(qa_compat::services::CallError::File)
    );
    let (assets, catalog) = builder.finish();
    assert_eq!(catalog.capacity().tags, 0);
    assert_eq!(catalog.capacity().frame_vertices, 0);
    assert_eq!(assets.models().len(), 1);
    assert!(catalog.diagnostics[0].error.contains("model tag capacity"));
}

#[test]
fn client_models_and_view_weapons_submit_all_md3_surfaces_to_the_shared_scene() {
    use qa_app::client_scene::{ClientModelBindings, ClientSceneInput};
    use qa_core::{
        configstrings::{ConfigLayout, ConfigStrings},
        primitives::{
            ClientId, EntityAppearance, EntityRecord, ModelRef, PlayerState, ResourceNamespace,
            ViewWeapon,
        },
    };
    use qa_network::{
        commands::packet::Protocol,
        projection::{EntityProjection, PlayerProjection},
    };
    let fixture = support::Fixture::surfaces();
    let profile = shader_profile(RuleSetId::Quake3);
    let request = ModelRequest::native("models/test.md3", RuleSetId::Quake3, None);
    let (assets, catalog) = load(
        &fixture,
        Assets::load().unwrap(),
        &[request],
        ModelLimits::default(),
    )
    .unwrap();
    // File, presentation, client wire protocol and movement are independent.
    let layout = ConfigLayout::QUAKE2;
    let mut configs =
        ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
            .unwrap();
    configs
        .set(layout.resources[0].get(50).unwrap(), b"models/test.md3")
        .unwrap();
    let client = ClientId(7);
    let reference = ModelRef::native(ResourceNamespace::Client(client), 50).unwrap();
    let mut bindings = ClientModelBindings::load(client, layout);
    assert_eq!(bindings.bind_all(&configs, &catalog, profile, None), 0);
    let projection = EntityProjection::load(Protocol::Quake2_34);
    let player_projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut player = PlayerState::default();
    player.movement_rules = RuleSetId::Quake;
    player.view_weapon = ViewWeapon {
        model: Some(reference),
        frame: 1,
        skin: 3,
        ..Default::default()
    };
    let record = EntityRecord {
        position: Vec3([20.0, 30.0, 40.0]),
        appearance: EntityAppearance {
            source: RuleSetId::Quake2,
            frame: 1,
            skin: 3,
            models: [Some(reference), None, None, None],
            ..Default::default()
        },
        ..Default::default()
    };
    let input = ClientSceneInput {
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &bindings,
        models: &catalog,
        static_entities: std::slice::from_ref(&record),
        player: &player,
        weapon_view_angles: player.view_angles,
        alias_roll: qa_gameplay::rules::appearance::AliasRoll::default(),
        interpolation: Default::default(),
        gun_animation: None,
        gun_previous: None,
        view_entity: None,
    };
    let expected = catalog
        .bindings(
            catalog.registration(request.path, profile).unwrap(),
            3,
            ModelMaterialPolicy::default(),
        )
        .unwrap()
        .collect::<Vec<_>>();
    for software in [false, true] {
        for limit in [3, 4] {
            let mut front = qa_render::FrontEnd::load(qa_render::Limits {
                entities: limit,
                ..Default::default()
            })
            .unwrap();
            let mut frame = front.begin_frame([0; 4]).unwrap();
            let counts = input.append(
                &mut frame,
                &assets,
                &qa_render::Refdef::default(),
                software,
                None,
            );
            assert_eq!(counts.entities, 1);
            assert_eq!(counts.models, limit as u32);
            assert_eq!(counts.rejected, (4 - limit) as u32);
            assert_eq!(counts.missing_models, 0);
            let packet = frame.finish();
            for index in 0..limit {
                let entity = packet.entity(index as u32);
                assert_eq!(entity.model, expected[index % 2].model.into());
                assert_eq!(entity.material, Some(expected[index % 2].material));
                assert_eq!(entity.frame, 1);
                assert_eq!(entity.old_frame, 1);
                assert_eq!(entity.depth_hack, index >= 2);
                assert_eq!(
                    entity.origin,
                    if index < 2 {
                        record.position
                    } else {
                        Vec3::default()
                    }
                );
            }
        }
    }
}

#[test]
fn unresolved_model_materials_keep_native_bounds_without_a_render_handle() {
    let bytes = support::md3();
    let parsed = qa_formats::model::Model::parse(&bytes).unwrap();
    let bounds = parsed.bounds;
    let first_frame = parsed.frames[0].bounds;
    assert_ne!(bounds, first_frame);
    let fixture = support::Fixture::new(&[("models/test.md3", bytes)]);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [shader_profile(RuleSetId::Quake3)],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    assert_eq!(
        builder.precache(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3),
        Ok(bounds)
    );
    assert_eq!(
        builder.register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3),
        Err(qa_compat::services::CallError::File)
    );
    fixture.remove("models/test.md3");
    assert_eq!(
        builder.register_model(&fixture.vfs, b"models/test.md3", RuleSetId::Quake3),
        Err(qa_compat::services::CallError::File)
    );
    let (_, catalog) = builder.finish();
    assert_eq!(
        catalog
            .metadata(b"models/test.md3")
            .unwrap()
            .renderer_bounds,
        first_frame
    );
    assert_eq!(
        catalog.model_handle(b"models/test.md3", RuleSetId::Quake3),
        None
    );
    let rejected = catalog.diagnostics[0].name.0 as i32;
    assert!(rejected > 0);
    assert_eq!(
        catalog.name(qa_core::primitives::NameId(rejected as u32)),
        Some("models/test.md3")
    );
    assert_eq!(catalog.model_bounds(rejected), None);
}

#[test]
fn cached_native_model_admission_keeps_failed_source_profiles_separate() {
    let fixture = support::Fixture::new(&[
        ("models/test.md2", support::md2_skin(b"textures/body.tga")),
        (
            "textures/body.tga",
            support::tga(2, 2, &[[80, 30, 10, 255]; 4]),
        ),
    ]);
    let healthy = shader_profile(RuleSetId::Quake3);
    let mut failed = healthy;
    failed.source_rules = RuleSetId::Quake2;
    failed.image_settings.gamma_exponent = f32::NAN;
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [failed, healthy],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, b"models/test.md2", RuleSetId::Quake3)
        .unwrap();
    assert!(handle > 0);
    assert_eq!(
        builder.register_model(&fixture.vfs, b"models/test.md2", RuleSetId::Quake2),
        Err(qa_compat::services::CallError::File)
    );
    let bounds = builder.model_bounds(handle).unwrap();
    fixture.remove("models/test.md2");
    fixture.remove("textures/body.tga");
    let (_, catalog) = builder.finish();
    assert_eq!(
        catalog.model_handle(b"MODELS\\TEST.MD2", RuleSetId::Quake3),
        Some(handle)
    );
    assert_eq!(
        catalog.model_handle(b"models/test.md2", RuleSetId::Quake2),
        None
    );
    assert_eq!(catalog.model_bounds(handle), Some(bounds));
    assert_eq!(catalog.diagnostics.len(), 1);
}

#[test]
fn native_shader_first_registration_keeps_mips_display_and_exact_profiles() {
    for mipmaps in [false, true] {
        let fixture =
            support::Fixture::new(&[("ui/pic.tga", support::tga(4, 4, &[[80, 30, 10, 255]; 16]))]);
        let q3 = shader_profile(RuleSetId::Quake3);
        let q2 = shader_profile(RuleSetId::Quake2);
        let mut builder = ModelCatalogBuilder::new(
            Assets::load().unwrap(),
            [q3, q2],
            [],
            ModelLimits::default(),
        )
        .unwrap();
        let handle = builder
            .register_shader(&fixture.vfs, b"UI/Pic.tga", RuleSetId::Quake3, mipmaps)
            .unwrap();
        assert!(handle > 0);
        fixture.remove("ui/pic.tga");
        assert_eq!(
            builder.register_shader(&fixture.vfs, b"ui/pic.jpg", RuleSetId::Quake3, !mipmaps),
            Ok(handle)
        );
        let (assets, catalog) = builder.finish();
        assert_eq!(catalog.shader_handle(b"uI/pIC"), Some(handle));
        assert_eq!(catalog.shader_name(handle), Some(b"UI/Pic".as_slice()));
        assert_eq!(catalog.shader_binding(0, q3), None);
        assert_eq!(
            catalog.shader_binding(handle, shader_profile(RuleSetId::QuakeWorld)),
            None
        );
        let q3_material = catalog.shader_binding(handle, q3).unwrap();
        let q2_material = catalog.shader_binding(handle, q2).unwrap();
        for material in [q3_material, q2_material] {
            let material = assets.material(material).unwrap();
            let stage = material.stages[0];
            assert_eq!(stage.sampler.mipmaps, mipmaps);
            assert_eq!(
                stage.sampler.wrap,
                if mipmaps {
                    qa_render::assets::Wrap::Repeat
                } else {
                    qa_render::assets::Wrap::Clamp
                }
            );
            assert_eq!(stage.rgb_gen, RgbGen::Vertex);
            assert_eq!(stage.alpha_gen, AlphaGen::Vertex);
            assert_eq!(stage.blend, Some(StageBlend::ALPHA));
            assert_eq!(stage.depth_func, qa_render::assets::DepthFunc::Always);
            assert!(!stage.depth_write);
        }
        let StageTexture::Image(q3_image) = assets.material(q3_material).unwrap().stages[0].texture
        else {
            panic!("shader image");
        };
        let StageTexture::Image(q2_image) = assets.material(q2_material).unwrap().stages[0].texture
        else {
            panic!("shader image");
        };
        if mipmaps {
            assert_eq!(
                assets
                    .image(q3_image)
                    .unwrap()
                    .prepared
                    .as_ref()
                    .unwrap()
                    .levels[0]
                    .width,
                2
            );
            assert_eq!(
                assets
                    .image(q2_image)
                    .unwrap()
                    .prepared
                    .as_ref()
                    .unwrap()
                    .levels[0]
                    .width,
                4
            );
            assert_ne!(q3_material, q2_material);
        }
    }
}

#[test]
fn native_shader_explicit_stages_use_the_shared_cached_catalog_and_compiler() {
    let script = b"ui/first\n{\n cull none\n { map $whiteimage\n blendFunc add\n rgbGen vertex\n }\n { map $lightmap\n blendFunc filter\n }\n}\nui/second\n{\n nomipmaps\n { map ui/pic.tga\n blendFunc blend\n rgbGen vertex\n alphaGen vertex\n }\n}\n";
    let fixture = support::Fixture::new(&[
        ("scripts/ui.shader", script.to_vec()),
        ("ui/pic.tga", support::tga(4, 4, &[[80, 30, 10, 255]; 16])),
    ]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [profile],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    let first = builder
        .register_shader(&fixture.vfs, b"ui/first", RuleSetId::Quake3, false)
        .unwrap();
    fixture.remove("scripts/ui.shader");
    let second = builder
        .register_shader(&fixture.vfs, b"ui/second", RuleSetId::Quake3, true)
        .unwrap();
    let (assets, catalog) = builder.finish();
    let material = assets
        .material(catalog.shader_binding(first, profile).unwrap())
        .unwrap();
    assert_eq!(material.settings.cull, qa_render::shader::Cull::None);
    assert_eq!(material.stages.len(), 2);
    assert_eq!(material.stages[0].rgb_gen, RgbGen::Vertex);
    assert_eq!(
        material.stages[0].blend.unwrap().source,
        qa_render::shader::BlendFactor::One
    );
    assert_eq!(
        material.stages[1].texture,
        StageTexture::Image(qa_render::ImageId(0))
    );
    assert!(
        !assets
            .material(catalog.shader_binding(second, profile).unwrap())
            .unwrap()
            .stages[0]
            .sampler
            .mipmaps
    );
}

#[test]
fn native_shader_first_dot_folded_key_keeps_separator_identities_and_named_misses() {
    let fixture = support::Fixture::new(&[
        ("dir.part/pic.tga", support::tga(1, 1, &[[80, 30, 10, 255]])),
        ("ui/pic.tga", support::tga(1, 1, &[[80, 30, 10, 255]])),
    ]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut builder = ModelCatalogBuilder::new(
        Assets::load().unwrap(),
        [profile],
        [],
        ModelLimits::default(),
    )
    .unwrap();
    assert_eq!(
        builder.register_shader(&fixture.vfs, b"", RuleSetId::Quake3, false),
        Ok(0)
    );
    let dot = builder
        .register_shader(&fixture.vfs, b"dir.part/pic.tga", RuleSetId::Quake3, false)
        .unwrap();
    assert_eq!(
        builder.register_shader(
            &fixture.vfs,
            b"DIR.other/missing.tga",
            RuleSetId::Quake3,
            true
        ),
        Ok(dot)
    );
    let forward = builder
        .register_shader(&fixture.vfs, b"ui/pic.tga", RuleSetId::Quake3, false)
        .unwrap();
    let backward = builder
        .register_shader(&fixture.vfs, b"ui\\pic.tga", RuleSetId::Quake3, false)
        .unwrap();
    assert_ne!(forward, backward);
    assert_eq!(
        builder.register_shader(&fixture.vfs, b"ui/missing.tga", RuleSetId::Quake3, true),
        Err(qa_compat::services::CallError::File)
    );
    assert_eq!(
        builder.register_shader(&fixture.vfs, b"UI/Missing.jpg", RuleSetId::Quake3, false),
        Ok(0)
    );
    let (_, catalog) = builder.finish();
    assert_eq!(catalog.shader_handle(b"ui/missing"), Some(0));
    assert_eq!(catalog.shader_handle(b"ui/unknown"), None);
    assert_eq!(
        catalog.shader_binding(forward, profile),
        catalog.shader_binding(backward, profile)
    );
}

#[test]
fn shader_admitted_names_do_not_block_later_model_admission() {
    let fixture = support::Fixture::new(&[
        ("progs/test.tga", support::tga(1, 1, &[[80, 30, 10, 255]])),
        ("progs/test", support::mdl()),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let shader = shader_profile(RuleSetId::Quake3);
    let model = native_profile(RuleSetId::Quake, palette);
    let mut builder =
        ModelCatalogBuilder::new(assets, [shader, model], [], ModelLimits::default()).unwrap();
    let handle = builder
        .register_shader(&fixture.vfs, b"progs/test.tga", RuleSetId::Quake3, false)
        .unwrap();
    builder
        .register_request(
            &fixture.vfs,
            ModelRequest::native("progs/test", RuleSetId::Quake, Some(palette)),
        )
        .unwrap();
    let (_, catalog) = builder.finish();
    assert!(binding(&catalog, "progs/test", model, 0).is_some());
    assert!(catalog.shader_binding(handle, shader).is_some());
    assert_eq!(catalog.shader_name(handle), Some(b"progs/test".as_slice()));
}
fn binding(
    catalog: &ModelCatalog,
    path: &str,
    profile: ModelProfile,
    skin: u32,
) -> Option<qa_app::model_assets::ModelBinding> {
    catalog
        .bindings(
            catalog.registration(path, profile)?,
            skin,
            ModelMaterialPolicy::default(),
        )
        .and_then(|mut surfaces| surfaces.next())
}
fn image(
    assets: &Assets,
    catalog: &ModelCatalog,
    path: &str,
    profile: ModelProfile,
    skin: u32,
) -> qa_render::ImageId {
    let binding = binding(catalog, path, profile, skin).unwrap();
    let StageTexture::Image(image) = assets.material(binding.material).unwrap().stages[0].texture
    else {
        panic!("skin material")
    };
    image
}

fn render_indexed(
    assets: &Assets,
    binding: qa_app::model_assets::ModelBinding,
    palette: PaletteId,
) -> (Vec<u32>, qa_render::BackendStats) {
    let mut frontend = qa_render::FrontEnd::load(Default::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.add_entity(qa_render::SceneEntity {
        model: binding.model.into(),
        material: Some(binding.material),
        origin: Vec3([0.0, -24.0, -36.0]),
        lighting: Some(qa_render::stage::EntityLighting {
            ambient: [255.0; 3],
            directed: [0.0; 3],
            direction: Vec3([-1.0, 0.0, 0.0]),
        }),
        ..Default::default()
    }));
    assert!(frame.render_scene(
        qa_render::Refdef {
            viewport: qa_render::Viewport {
                width: 64,
                height: 48,
                ..Default::default()
            },
            fov: [90.0, 75.0],
            cpu_presentation: qa_render::CpuPresentation::Indexed {
                palette,
                lighting: qa_render::surface_cache::IndexedLighting::Gray,
                ambient: 0,
                fullbright: true,
            },
            ..Default::default()
        },
        &[],
        assets,
    ));
    let mut cpu = qa_render::cpu::CpuBackend::load(64, 48).unwrap();
    let stats = cpu.render(&frame.finish(), assets);
    (cpu.pixels().to_vec(), stats)
}

#[test]
fn native_md2_registration_keeps_frame_major_seams_skins_and_global_palette() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake2, palette);
    let models_before = assets.models().len();
    let (assets, catalog) = load(
        &fixture,
        assets,
        &[ModelRequest::native(
            "models/test/tris.md2",
            RuleSetId::Quake2,
            Some(palette),
        )],
        ModelLimits::default(),
    )
    .unwrap();
    let first = binding(&catalog, "models/test/tris.md2", profile, 0).unwrap();
    let second = binding(&catalog, "models/test/tris.md2", profile, 1).unwrap();
    assert_eq!(first.model, second.model);
    assert_ne!(first.material, second.material);
    assert_eq!(
        first.pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake2).pose
    );
    assert_eq!(first.pose, second.pose);
    assert_eq!(assets.models().len(), models_before + 1);
    let model = assets.model(first.model).unwrap();
    assert_eq!(model.frame_count(), 2);
    assert_eq!(model.vertices_per_frame, 5);
    assert_eq!(model.vertices.len(), 10);
    assert_eq!(model.indices.as_ref(), [0, 2, 3, 1, 3, 4]);
    assert_eq!(model.vertices[0].position, Vec3([11.0, 24.0, 39.0]));
    assert_eq!(model.vertices[5].position, Vec3([-4.0, 8.0, 21.0]));
    assert_eq!(model.vertices[0].position, model.vertices[1].position);
    assert_ne!(model.vertices[0].texcoord, model.vertices[1].texcoord);
    assert_eq!(model.vertices[0].texcoord, [0.0625, 0.125]);
    assert_eq!(model.vertices[1].texcoord, [0.0625, 0.875]);
    assert_eq!(
        model.frames(1, 0, 0.5).unwrap().vertex(0).position,
        Vec3([3.5, 16.0, 30.0])
    );
    for (skin, index) in [(0, 11), (1, 22)] {
        let image = assets
            .image(image(
                &assets,
                &catalog,
                "models/test/tris.md2",
                profile,
                skin,
            ))
            .unwrap();
        assert_eq!(
            image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
            [index; 32]
        );
        assert_eq!(&image.rgba[..4], &[index, 255 - index, index / 2, 255]);
        assert!(image.prepared.is_some());
        assert_eq!(
            assets
                .material(
                    binding(&catalog, "models/test/tris.md2", profile, skin)
                        .unwrap()
                        .material
                )
                .unwrap()
                .stages[0]
                .sampler
                .wrap,
            qa_render::assets::Wrap::Repeat
        );
    }
    assert_eq!(catalog.capacity().skins, 2);
    assert_eq!(catalog.capacity().bindings, 8);
    assert_eq!(catalog.capacity().frame_vertices, 10);
    assert_eq!(catalog.capacity().indices, 6);
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn mdl_indices_and_uv_seams_do_not_inherit_presentation_from_file_format() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake2, palette);
    // An MDL can use Q2 presentation independently of its file provenance.
    let (assets, catalog) = load(
        &fixture,
        assets,
        &[ModelRequest::native(
            "progs/test.mdl",
            RuleSetId::Quake2,
            Some(palette),
        )],
        ModelLimits::default(),
    )
    .unwrap();
    let binding = binding(&catalog, "progs/test.mdl", profile, 0).unwrap();
    assert_eq!(
        binding.pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake2).pose
    );
    let model = assets.model(binding.model).unwrap();
    assert_eq!(model.frame_count(), 2);
    assert_eq!(model.vertices_per_frame, 3);
    assert_eq!(model.vertices[0].position, Vec3([8.0, 4.0, 0.0]));
    assert_eq!(model.vertices[3].position, Vec3([10.0, 4.0, 0.0]));
    assert_eq!(model.vertices[0].texcoord, [0.75, 0.5]);
    let image = assets
        .image(image(&assets, &catalog, "progs/test.mdl", profile, 0))
        .unwrap();
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        [1, 2]
    );
    assert_eq!(&image.rgba[..8], &[1, 254, 0, 255, 2, 253, 1, 255]);
    assert!(image.prepared.is_some());
}

#[test]
fn external_skin_decoding_does_not_inherit_upload_presentation() {
    let model = support::md2_skin(b"models/test/skin.tga");
    let mut tga = vec![0; 18];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&2u16.to_le_bytes());
    tga[14..16].copy_from_slice(&2u16.to_le_bytes());
    tga[16] = 24;
    tga[17] = 32; // Top-origin source, which native Q3 deliberately ignores.
    tga.extend([0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 0, 0]);
    let fixture = support::Fixture::new(&[
        ("models/test/tris.md2", model),
        ("models/test/skin.tga", tga),
    ]);
    let assets = Assets::load().unwrap();
    let mut request = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake3, None);
    request.source_rules = RuleSetId::Quake2;
    let (assets, catalog) = load(&fixture, assets, &[request], ModelLimits::default()).unwrap();
    let binding = binding(&catalog, request.path, request.profile(), 0).unwrap();
    assert_eq!(
        binding.pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake3).pose
    );
    let skin = assets
        .image(image(&assets, &catalog, request.path, request.profile(), 0))
        .unwrap();
    assert_eq!(&skin.rgba[..8], &[255, 0, 0, 255, 255, 0, 0, 255]);
    assert_eq!(&skin.rgba[8..], &[0, 0, 255, 255, 0, 0, 255, 255]);
    assert!(skin.prepared.is_some());
}

#[test]
fn q3_pcx_source_uses_embedded_palette_and_source_extent_without_global_palette() {
    let fixture = support::Fixture::new(&[
        ("models/test/tris.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx_dimensions(641, 4, 11)),
        ("models/test/skin1.pcx", support::pcx_dimensions(641, 4, 22)),
    ]);
    let assets = Assets::load().unwrap();
    let request = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake3, None);
    let (assets, catalog) = load(&fixture, assets, &[request], ModelLimits::default()).unwrap();
    let first = binding(&catalog, request.path, request.profile(), 0).unwrap();
    let second = binding(&catalog, request.path, request.profile(), 1).unwrap();
    assert_eq!(first.model, second.model);
    assert_eq!(
        first.pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake3).pose
    );
    for (ordinal, index) in [(0, 11), (1, 22)] {
        let skin = assets
            .image(image(
                &assets,
                &catalog,
                request.path,
                request.profile(),
                ordinal,
            ))
            .unwrap();
        assert_eq!((skin.width, skin.height), (641, 4));
        assert_eq!(&skin.rgba[..4], &[255, index, index, 255]);
        assert!(
            skin.rgba
                .chunks_exact(4)
                .all(|rgba| rgba == [255, index, index, 255])
        );
        assert!(skin.indexed.is_none());
        assert!(skin.prepared.is_some());
    }
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn rgb_pcx_and_tga_skins_keep_gl_pixels_and_draw_on_explicit_indexed_cpu_targets() {
    for path in ["models/test/skin.pcx", "models/test/skin.tga"] {
        let pixels = if path.ends_with("pcx") {
            support::pcx(11)
        } else {
            support::tga(2, 2, &[[255, 11, 11, 255]; 4])
        };
        let fixture = support::Fixture::new(&[
            (
                "models/test/tris.md2",
                support::md2_quad_skin(path.as_bytes()),
            ),
            (path, pixels),
        ]);
        let mut assets = Assets::load().unwrap();
        let first = support::palette(&mut assets);
        let second = support::shifted_palette(&mut assets, 128);
        let mut original = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, None);
        original.source_rules = RuleSetId::Quake3;
        let requests = [
            original,
            ModelRequest {
                palette: Some(first),
                ..original
            },
            ModelRequest {
                palette: Some(second),
                ..original
            },
        ];
        let (assets, catalog) = load(&fixture, assets, &requests, ModelLimits::default()).unwrap();
        let baseline = binding(&catalog, original.path, original.profile(), 0).unwrap();
        let source = assets
            .image(image(
                &assets,
                &catalog,
                original.path,
                original.profile(),
                0,
            ))
            .unwrap();
        assert!(source.indexed.is_none());
        assert!(
            source
                .rgba
                .chunks_exact(4)
                .all(|rgba| rgba == [255, 11, 11, 255])
        );
        let mut selected_images = Vec::new();
        for (request, palette) in requests[1..].iter().zip([first, second]) {
            let selected = binding(&catalog, request.path, request.profile(), 0).unwrap();
            assert_eq!(selected.model, baseline.model);
            let id = image(&assets, &catalog, request.path, request.profile(), 0);
            assert!(!selected_images.contains(&id));
            selected_images.push(id);
            let skin = assets.image(id).unwrap();
            assert_eq!(source.rgba, skin.rgba);
            assert_eq!(source.prepared, skin.prepared);
            let expected = assets
                .palette(palette)
                .unwrap()
                .nearest_index([255, 11, 11], Some(255));
            let texture = skin.indexed.as_ref().unwrap();
            assert_eq!(texture.transparent_index(), Some(255));
            assert!(
                texture
                    .mip(0)
                    .unwrap()
                    .indices()
                    .iter()
                    .all(|&index| index == expected)
            );
            let (pixels, stats) = render_indexed(&assets, selected, palette);
            assert_eq!(stats.rejected, 0);
            assert!(stats.triangles > 0);
            let color = assets.palette(palette).unwrap().color(expected);
            assert!(pixels.contains(&color));
            assert!(
                pixels
                    .iter()
                    .all(|&pixel| pixel == color || pixel == u32::from_le_bytes([3, 5, 7, 255]))
            );
        }
        assert!(catalog.diagnostics.is_empty());
    }
}

#[test]
fn source_rgba_cutout_skins_keep_alpha_and_skip_the_reserved_index_in_cpu_scene() {
    let path = "models/test/skin.tga";
    let fixture = support::Fixture::new(&[
        (
            "models/opaque.md2",
            support::md2_quad_skin(b"models/test/opaque.tga"),
        ),
        ("models/cutout.md2", support::md2_quad_skin(path.as_bytes())),
        (
            "models/test/opaque.tga",
            support::tga(2, 2, &[[255, 11, 11, 255]; 4]),
        ),
        (
            path,
            support::tga(
                2,
                2,
                &[
                    [255, 11, 11, 255],
                    [255, 11, 11, 0],
                    [255, 11, 11, 255],
                    [255, 11, 11, 0],
                ],
            ),
        ),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let requests = ["models/opaque.md2", "models/cutout.md2"].map(|path| ModelRequest {
        source_rules: RuleSetId::Quake3,
        source_palette: None,
        ..ModelRequest::native(path, RuleSetId::Quake2, Some(palette))
    });
    let (assets, catalog) = load(&fixture, assets, &requests, ModelLimits::default()).unwrap();
    let selected =
        requests.map(|request| binding(&catalog, request.path, request.profile(), 0).unwrap());
    let image = assets
        .image(image(
            &assets,
            &catalog,
            requests[1].path,
            requests[1].profile(),
            0,
        ))
        .unwrap();
    assert_eq!(
        image.rgba.as_ref(),
        &[
            255, 11, 11, 255, 255, 11, 11, 0, 255, 11, 11, 255, 255, 11, 11, 0,
        ]
    );
    let indices = image.indexed.as_ref().unwrap().mip(0).unwrap().indices();
    assert_ne!(indices[0], 255);
    assert_eq!(indices, [indices[0], 255, indices[0], 255]);
    let opaque = render_indexed(&assets, selected[0], palette);
    let cutout = render_indexed(&assets, selected[1], palette);
    assert_eq!(opaque.1.rejected, 0);
    assert_eq!(cutout.1.rejected, 0);
    assert_ne!(opaque.0, cutout.0);
    let color = assets.palette(palette).unwrap().color(indices[0]);
    let opaque_pixels = opaque.0.iter().filter(|&&pixel| pixel == color).count();
    let cutout_pixels = cutout.0.iter().filter(|&&pixel| pixel == color).count();
    assert!(cutout_pixels > 0 && cutout_pixels < opaque_pixels);
}

#[test]
fn canonical_name_registration_is_unique_and_numeric_skin_lookup_falls_back_to_zero() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake2, palette);
    let models_before = assets.models().len();
    let (assets, catalog) = load(
        &fixture,
        assets,
        &[
            ModelRequest::native("MODELS\\TEST\\TRIS.MD2", RuleSetId::Quake2, Some(palette)),
            ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(palette)),
        ],
        ModelLimits::default(),
    )
    .unwrap();
    let registration = catalog
        .registration("MODELS\\TEST\\TRIS.MD2", profile)
        .unwrap();
    assert_eq!(
        catalog.registration("models/test/tris.md2", profile),
        Some(registration)
    );
    assert_eq!(
        catalog
            .bindings(registration, 99, ModelMaterialPolicy::default())
            .and_then(|mut surfaces| surfaces.next()),
        catalog
            .bindings(registration, 0, ModelMaterialPolicy::default())
            .and_then(|mut surfaces| surfaces.next())
    );
    assert_eq!(catalog.registration("missing.md2", profile), None);
    assert_eq!(catalog.capacity().names, 2); // explicit empty NameId plus file.
    assert_eq!(catalog.capacity().name_bytes, "models/test/tris.md2".len());
    assert_eq!(assets.models().len(), models_before + 1);
}

#[test]
fn distinct_model_filenames_share_the_existing_skin_image_recipe_cache() {
    let fixture = support::Fixture::new(&[
        ("models/a.md2", support::md2()),
        ("models/b.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx(11)),
        ("models/test/skin1.pcx", support::pcx(22)),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake2, palette);
    let before = assets.images().len();
    let (assets, catalog) = load(
        &fixture,
        assets,
        &["models/a.md2", "models/b.md2"]
            .map(|path| ModelRequest::native(path, RuleSetId::Quake2, Some(palette))),
        ModelLimits::default(),
    )
    .unwrap();
    // Filename identity remains distinct even when source bytes are equal.
    assert_ne!(
        binding(&catalog, "models/a.md2", profile, 0).unwrap().model,
        binding(&catalog, "models/b.md2", profile, 0).unwrap().model
    );
    assert_eq!(assets.images().len(), before + 2);
    for skin in [0, 1] {
        assert_eq!(
            image(&assets, &catalog, "models/a.md2", profile, skin),
            image(&assets, &catalog, "models/b.md2", profile, skin)
        );
    }
}

#[test]
fn one_registered_mesh_and_image_supply_all_four_material_variants() {
    let fixture = support::Fixture::models();
    for rules in [
        RuleSetId::Quake,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ] {
        let mut assets = Assets::load().unwrap();
        let palette = support::palette(&mut assets);
        let profile = native_profile(rules, palette);
        let (assets, catalog) = load(
            &fixture,
            assets,
            &[ModelRequest::native(
                "models/test/tris.md2",
                rules,
                Some(palette),
            )],
            ModelLimits::default(),
        )
        .unwrap();
        let registration = catalog
            .registration("models/test/tris.md2", profile)
            .unwrap();
        let opaque = catalog
            .bindings(registration, 0, ModelMaterialPolicy::default())
            .and_then(|mut surfaces| surfaces.next())
            .unwrap();
        let base_image = image(&assets, &catalog, "models/test/tris.md2", profile, 0);
        for translucent in [false, true] {
            for fullbright in [false, true] {
                let policy = ModelMaterialPolicy {
                    translucent,
                    fullbright,
                    custom_skin: 0,
                    frame: 0,
                };
                let binding = catalog
                    .bindings(registration, 0, policy)
                    .and_then(|mut surfaces| surfaces.next())
                    .unwrap();
                assert_eq!(binding.model, opaque.model);
                let material = assets.material(binding.material).unwrap();
                assert_eq!(material.stages.len(), 1);
                let stage = material.stages[0];
                assert_eq!(stage.texture, StageTexture::Image(base_image));
                assert_eq!(
                    stage.rgb_gen,
                    if fullbright {
                        RgbGen::Identity
                    } else {
                        RgbGen::LightingDiffuse
                    }
                );
                assert_eq!(stage.alpha_gen, AlphaGen::Entity);
                assert_eq!(stage.blend, translucent.then_some(StageBlend::ALPHA));
                assert_eq!(stage.depth_write, !translucent);
                let expected = matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
                    .then_some(SoftwareAlias {
                        fullbright_light: fullbright.then_some([128.0, 64.0]),
                        alpha_cutoffs: translucent.then_some([0.33, 0.66]),
                    });
                assert_eq!(material.settings.software_alias, expected);
            }
        }
    }
}

#[test]
fn optional_unsupported_assets_report_diagnostics_without_mesh_placeholders() {
    let fixture = support::Fixture::new(&[
        (
            "progs/group.mdl",
            include_bytes!("../../formats/tests/fixtures/models/group.mdl").to_vec(),
        ),
        (
            "progs/group.spr",
            include_bytes!("../../formats/tests/fixtures/models/group.spr").to_vec(),
        ),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let count = assets.models().len();
    let requests =
        ["progs/group.mdl", "progs/group.spr", "progs/missing.mdl"].map(|path| ModelRequest {
            required: false,
            ..ModelRequest::native(path, RuleSetId::Quake, Some(palette))
        });
    let (assets, catalog) = load(&fixture, assets, &requests, ModelLimits::default()).unwrap();
    assert_eq!(catalog.diagnostics.len(), 3);
    assert_eq!(assets.models().len(), count);
    for diagnostic in &catalog.diagnostics {
        assert_eq!(
            catalog.registration(catalog.name(diagnostic.name).unwrap(), diagnostic.profile),
            None
        );
        assert!(!diagnostic.error.is_empty());
    }
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|d| d.error.contains("grouped MDL"))
    );
    assert!(catalog.diagnostics.iter().any(|d| d.error.contains("Spr")));
    let error = load(
        &fixture,
        assets,
        &[ModelRequest::native(
            "progs/missing.mdl",
            RuleSetId::Quake,
            Some(palette),
        )],
        ModelLimits::default(),
    )
    .err()
    .unwrap();
    assert!(error.contains("progs/missing.mdl"));
}

#[test]
fn load_limits_reject_at_registration() {
    let fixture = support::Fixture::models();
    for limits in [
        ModelLimits {
            models: 0,
            ..Default::default()
        },
        ModelLimits {
            name_bytes: 1,
            ..Default::default()
        },
        ModelLimits {
            skins: 1,
            ..Default::default()
        },
        ModelLimits {
            frame_vertices: 9,
            ..Default::default()
        },
        ModelLimits {
            indices: 5,
            ..Default::default()
        },
        ModelLimits {
            file_bytes: 67,
            ..Default::default()
        },
    ] {
        let mut assets = Assets::load().unwrap();
        let palette = support::palette(&mut assets);
        assert!(
            load(
                &fixture,
                assets,
                &[ModelRequest::native(
                    "models/test/tris.md2",
                    RuleSetId::Quake2,
                    Some(palette)
                )],
                limits
            )
            .is_err()
        );
    }
}

#[test]
fn surface_name_capacity_failure_keeps_registration_transactional() {
    use qa_compat::services::CallError;

    let failed_path = "broken.md3";
    let healthy_path = "good.md3";
    let mut failed = support::md3_surfaces();
    failed[80..84].copy_from_slice(&0i32.to_le_bytes());
    let first = i32::from_le_bytes(failed[100..104].try_into().unwrap()) as usize;
    let second =
        first + i32::from_le_bytes(failed[first + 104..first + 108].try_into().unwrap()) as usize;
    failed[second + 4..second + 68].fill(0);
    let long_name = b"surface_name_exceeding_the_entire_remaining_budget";
    failed[second + 4..second + 4 + long_name.len()].copy_from_slice(long_name);
    let parsed = qa_formats::model::Model::parse(&failed).unwrap();
    assert_eq!(parsed.meshes.len(), 2);
    assert_eq!(parsed.tags_per_frame, 0);
    assert_eq!(parsed.meshes[1].name, long_name);
    let mut healthy = support::md3();
    healthy[80..84].copy_from_slice(&0i32.to_le_bytes());
    let fixture = support::Fixture::new(&[
        (failed_path, failed),
        (healthy_path, healthy),
        (
            "textures/body.tga",
            support::tga(2, 2, &[[90, 20, 10, 255]; 4]),
        ),
    ]);
    let profiles = [
        shader_profile(RuleSetId::Quake3),
        shader_profile(RuleSetId::Quake2),
    ];
    let assets = Assets::load().unwrap();
    let before = assets.models().len();
    // The first surface fits, but the second does not. A failed batch must
    // leave those first-surface bytes available to the later healthy model.
    let mut builder = ModelCatalogBuilder::new(
        assets,
        profiles,
        [],
        ModelLimits {
            name_bytes: failed_path.len() + healthy_path.len() + b"body".len(),
            ..Default::default()
        },
    )
    .unwrap();
    let request = ModelRequest::native(failed_path, RuleSetId::Quake3, None);
    assert!(
        builder
            .register_request(&fixture.vfs, request)
            .unwrap_err()
            .contains("model surface names")
    );
    fixture.remove(failed_path);
    assert_eq!(
        builder.precache(&fixture.vfs, failed_path.as_bytes(), RuleSetId::Quake3),
        Err(CallError::Capacity)
    );
    assert_eq!(
        builder.register_model(&fixture.vfs, failed_path.as_bytes(), RuleSetId::Quake3),
        Err(CallError::Capacity)
    );
    for profile in profiles {
        let optional = ModelRequest {
            source_rules: profile.source_rules,
            rules: profile.rules,
            image_settings: profile.image_settings,
            required: false,
            ..request
        };
        assert!(
            builder
                .register_request(&fixture.vfs, optional)
                .unwrap_err()
                .contains("model surface names")
        );
    }
    builder
        .register_request(
            &fixture.vfs,
            ModelRequest::native(healthy_path, RuleSetId::Quake3, None),
        )
        .unwrap();
    let handle = builder
        .register_model(&fixture.vfs, healthy_path.as_bytes(), RuleSetId::Quake3)
        .unwrap();
    fixture.remove(healthy_path);
    let (assets, catalog) = builder.finish();
    assert_eq!(assets.models().len(), before + 1);
    assert_eq!(catalog.capacity().registrations, profiles.len());
    assert_eq!(catalog.capacity().names, 4);
    assert!(catalog.metadata(failed_path.as_bytes()).is_none());
    for profile in profiles {
        assert!(catalog.registration(failed_path, profile).is_none());
        let registration = catalog.registration(healthy_path, profile).unwrap();
        assert_eq!(
            catalog.renderer_registration(handle, profile),
            Some(registration)
        );
        assert_eq!(
            catalog
                .bindings(registration, 0, Default::default())
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn synchronous_precaches_share_one_file_mesh_and_profile_image_cache_across_calls() {
    let path = "models/test/shared.pcx";
    let fixture = support::Fixture::new(&[
        ("models/first.md2", support::md2_skin(path.as_bytes())),
        ("models/second.md2", support::md2_skin(path.as_bytes())),
        (path, support::pcx(11)),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let native = native_profile(RuleSetId::Quake2, palette);
    let foreign = ModelProfile {
        rules: RuleSetId::Quake,
        image_settings: qa_render::material::resources::ImageSettings::native(RuleSetId::Quake),
        ..native
    };
    let initial_meshes = assets.models().len();
    let mut builder =
        ModelCatalogBuilder::new(assets, [native, foreign], [], ModelLimits::default()).unwrap();
    let bounds = builder
        .precache(&fixture.vfs, b"models/first.md2", RuleSetId::Quake2)
        .unwrap();
    assert_eq!(
        builder.bounds(b"MODELS\\FIRST.MD2", RuleSetId::Quake),
        Some(bounds)
    );
    fixture.remove("models/first.md2");
    fixture.remove(path);
    assert_eq!(
        builder
            .precache(&fixture.vfs, b"MODELS\\FIRST.MD2", RuleSetId::Quake)
            .unwrap(),
        bounds
    );
    // The second file needs the existing detached recipe cache: its skin is
    // already gone from the VFS and must not be loaded through a second cache.
    assert_eq!(
        builder
            .precache(&fixture.vfs, b"models/second.md2", RuleSetId::Quake2)
            .unwrap(),
        bounds
    );
    let (assets, catalog) = builder.finish();
    assert_eq!(assets.models().len(), initial_meshes + 2);
    assert_eq!(catalog.capacity().registrations, 4);
    assert_eq!(catalog.capacity().frame_vertices, 20);
    assert!(catalog.diagnostics.is_empty());
    for profile in [native, foreign] {
        let first = binding(&catalog, "models/first.md2", profile, 0).unwrap();
        let second = binding(&catalog, "models/second.md2", profile, 0).unwrap();
        assert_ne!(first.model, second.model);
        assert_eq!(
            image(&assets, &catalog, "models/first.md2", profile, 0),
            image(&assets, &catalog, "models/second.md2", profile, 0)
        );
    }
    assert_eq!(
        binding(&catalog, "models/first.md2", native, 0)
            .unwrap()
            .model,
        binding(&catalog, "models/first.md2", foreign, 0)
            .unwrap()
            .model
    );
    assert_eq!(
        catalog.bounds(b"MODELS\\FIRST.MD2", RuleSetId::Quake),
        Some(bounds)
    );
}

#[test]
fn native_metadata_survives_unsupported_render_formats_and_builder_freeze() {
    let mut sprite = include_bytes!("../../formats/tests/fixtures/models/group.spr").to_vec();
    sprite[16..20].copy_from_slice(&5i32.to_le_bytes());
    sprite[20..24].copy_from_slice(&3i32.to_le_bytes());
    let fixture = support::Fixture::new(&[
        (
            "progs/group.mdl",
            include_bytes!("../../formats/tests/fixtures/models/group.mdl").to_vec(),
        ),
        ("progs/group.spr", sprite),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake, palette);
    let initial_meshes = assets.models().len();
    let mut builder =
        ModelCatalogBuilder::new(assets, [profile], [], ModelLimits::default()).unwrap();
    let alias = builder
        .precache(&fixture.vfs, b"progs/group.mdl", RuleSetId::Quake)
        .unwrap();
    assert_eq!(alias.mins, Vec3([-16.0; 3]));
    assert_eq!(alias.maxs, Vec3([16.0; 3]));
    let sprite = builder
        .precache(&fixture.vfs, b"progs/group.spr", RuleSetId::Quake)
        .unwrap();
    assert_eq!(sprite.mins, Vec3([-2.0, -2.0, -1.0]));
    assert_eq!(sprite.maxs, Vec3([2.0, 2.0, 1.0]));
    fixture.remove("progs/group.mdl");
    fixture.remove("progs/group.spr");
    let (assets, catalog) = builder.finish();
    assert_eq!(assets.models().len(), initial_meshes);
    assert_eq!(catalog.diagnostics.len(), 2);
    for (path, bounds, format) in [
        (
            "progs/group.mdl",
            alias,
            qa_formats::model::ModelFormat::Mdl,
        ),
        (
            "progs/group.spr",
            sprite,
            qa_formats::model::ModelFormat::Spr,
        ),
    ] {
        assert_eq!(
            catalog.bounds(path.as_bytes(), RuleSetId::Quake),
            Some(bounds)
        );
        let metadata = catalog.metadata(path.as_bytes()).unwrap();
        assert_eq!(metadata.bounds, bounds);
        assert_eq!(metadata.format, format);
        assert_eq!(catalog.registration(path, profile), None);
    }
    assert_eq!(catalog.bounds(b"progs/absent.mdl", RuleSetId::Quake), None);
}

#[test]
fn one_filename_has_explicit_profile_variants_on_one_mesh_without_first_profile_fallback() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let source = support::palette(&mut assets);
    let target = support::shifted_palette(&mut assets, 128);
    let native = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(source));
    let mut translated = native;
    translated.rules = RuleSetId::Quake;
    translated.palette = Some(target);
    translated.image_settings =
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake);
    let mut different_settings = native;
    different_settings.image_settings.intensity = 1.0;
    let mut different_source = native;
    different_source.source_rules = RuleSetId::Quake3;
    let before = assets.models().len();
    let (assets, catalog) = load(
        &fixture,
        assets,
        &[
            native,
            translated,
            different_settings,
            different_source,
            native,
        ],
        ModelLimits::default(),
    )
    .unwrap();
    let selected = [native, translated, different_settings, different_source].map(|request| {
        let id = catalog
            .registration("MODELS\\TEST\\TRIS.MD2", request.profile())
            .unwrap();
        catalog
            .bindings(id, 0, ModelMaterialPolicy::default())
            .and_then(|mut surfaces| surfaces.next())
            .unwrap()
    });
    assert_eq!(assets.models().len(), before + 1);
    assert!(
        selected
            .iter()
            .all(|binding| binding.model == selected[0].model)
    );
    assert_eq!(
        selected[0].pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake2).pose
    );
    assert_eq!(
        selected[1].pose,
        qa_gameplay::rules::appearance::model_presentation(RuleSetId::Quake).pose
    );
    assert_ne!(selected[0].material, selected[1].material);
    assert_ne!(selected[0].material, selected[2].material);
    assert_eq!(catalog.capacity().registrations, 4);
    assert_eq!(catalog.capacity().frame_vertices, 10);
    assert_eq!(catalog.capacity().indices, 6);
    let mut absent = translated.profile();
    absent.rules = RuleSetId::QuakeWorld;
    assert_eq!(catalog.registration(native.path, absent), None);
    let native_image = assets
        .image(image(&assets, &catalog, native.path, native.profile(), 0))
        .unwrap();
    let translated_image = assets
        .image(image(
            &assets,
            &catalog,
            translated.path,
            translated.profile(),
            0,
        ))
        .unwrap();
    assert_eq!(native_image.rgba, translated_image.rgba);
    assert_eq!(
        native_image
            .indexed
            .as_ref()
            .unwrap()
            .mip(0)
            .unwrap()
            .indices(),
        [11; 32]
    );
    assert_eq!(
        translated_image
            .indexed
            .as_ref()
            .unwrap()
            .mip(0)
            .unwrap()
            .indices(),
        [139; 32]
    );
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn embedded_mdl_registration_uses_the_existing_source_to_target_index_translation() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let source = support::palette(&mut assets);
    let target = support::shifted_palette(&mut assets, 128);
    let mut request = ModelRequest::native("progs/test.mdl", RuleSetId::Quake2, Some(target));
    request.source_rules = RuleSetId::Quake;
    request.source_palette = Some(source);
    let (assets, catalog) = load(&fixture, assets, &[request], ModelLimits::default()).unwrap();
    let skin = assets
        .image(image(&assets, &catalog, request.path, request.profile(), 0))
        .unwrap();
    assert_eq!(&skin.rgba[..8], &[1, 254, 0, 255, 2, 253, 1, 255]);
    assert_eq!(
        skin.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        [129, 130]
    );
}

#[test]
fn failed_optional_profile_does_not_hide_another_registration_of_the_same_filename() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let native = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(palette));
    let absent = ModelRequest {
        source_palette: None,
        required: false,
        ..native
    };
    let (_assets, catalog) =
        load(&fixture, assets, &[absent, native], ModelLimits::default()).unwrap();
    assert!(
        catalog
            .registration(native.path, native.profile())
            .is_some()
    );
    assert_eq!(catalog.registration(absent.path, absent.profile()), None);
    assert_eq!(catalog.diagnostics.len(), 1);
    assert_eq!(catalog.diagnostics[0].profile, absent.profile());
    assert!(catalog.diagnostics[0].error.contains("source palette"));
}

#[test]
fn optional_image_constructor_failure_isolated_from_healthy_same_filename_profile() {
    let fixture = support::Fixture::models();
    for invalid in 0..3 {
        let mut assets = Assets::load().unwrap();
        let palette = support::palette(&mut assets);
        let native = ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(palette));
        let mut optional = native;
        optional.required = false;
        match invalid {
            0 => optional.image_settings.max_dimension = 0,
            1 => optional.image_settings.picmip = 32,
            _ => optional.image_settings.intensity = f32::NAN,
        }
        let before = assets.models().len();
        let (assets, catalog) = load(
            &fixture,
            assets,
            &[optional, native],
            ModelLimits::default(),
        )
        .unwrap();
        let healthy = binding(&catalog, native.path, native.profile(), 0).unwrap();
        assert_eq!(assets.models().len(), before + 1);
        assert_eq!(assets.model(healthy.model).unwrap().frame_count(), 2);
        assert_eq!(
            catalog.registration(optional.path, optional.profile()),
            None
        );
        assert_eq!(catalog.capacity().registrations, 2);
        assert_eq!(catalog.diagnostics.len(), 1);
        let diagnostic = &catalog.diagnostics[0];
        assert_eq!(catalog.name(diagnostic.name), Some(native.path));
        assert!(diagnostic.error.starts_with("model images: "));
        if invalid == 2 {
            assert!(diagnostic.profile.image_settings.intensity.is_nan());
        } else {
            assert_eq!(diagnostic.profile, optional.profile());
        }
        // Required construction keeps its previous image-error category.
        optional.required = true;
        let error = load(&fixture, assets, &[optional], ModelLimits::default())
            .err()
            .unwrap();
        assert!(error.starts_with("model images: "));
    }
}

use support::{world, world_variants};

#[test]
fn inline_model_ordinals_stay_in_their_world_namespace_without_copying_geometry() {
    let fixture = support::Fixture::new(&[]);
    let mut assets = Assets::load().unwrap();
    let first = world(&mut assets, 0.0);
    let second = world(&mut assets, 100.0);
    let meshes = assets.models().len();
    let (assets, catalog) = ModelCatalog::load(
        &fixture.vfs,
        assets,
        [],
        [first, second, first],
        ModelLimits::default(),
    )
    .unwrap();
    assert_eq!(assets.models().len(), meshes);
    assert_eq!(catalog.capacity().inline_bindings, 4);
    let first_inline = catalog.resolve_inline(first, 1).unwrap();
    let second_inline = catalog.resolve_inline(second, 1).unwrap();
    assert_eq!(first_inline.world, first);
    assert_eq!(second_inline.world, second);
    assert_eq!(first_inline.index, 1);
    assert_eq!(
        first_inline.surfaces,
        assets.world(first).unwrap().geometry().models[1].surfaces
    );
    assert_eq!(catalog.resolve_inline(first, 2), None);
    assert_eq!(catalog.resolve_inline(WorldId(u32::MAX), 0), None);
    assert!(
        ModelCatalog::load(
            &fixture.vfs,
            assets,
            [],
            [first, second],
            ModelLimits {
                inline_models: 3,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn native_inline_handles_share_the_metadata_table_and_keep_world_namespaces() {
    let mut assets = Assets::load().unwrap();
    let first = world(&mut assets, 0.0);
    let second = world(&mut assets, 100.0);
    let meshes = assets.models().len();
    let first_bounds = assets.world(first).unwrap().geometry().models[1].bounds;
    let second_bounds = assets.world(second).unwrap().geometry().models[1].bounds;
    let mut builder =
        ModelCatalogBuilder::new(assets, [], [first, second], Default::default()).unwrap();
    let a = builder.register_inline(first, 1).unwrap();
    let b = builder.register_inline(second, 1).unwrap();
    assert_ne!(a, b);
    assert_eq!(builder.register_inline(first, 1), Ok(a));
    assert_eq!(builder.model_bounds(a), Some(first_bounds));
    assert_eq!(builder.model_bounds(b), Some(second_bounds));
    assert_eq!(
        builder.register_inline(first, 2),
        Err(qa_compat::services::CallError::File)
    );
    let (assets, catalog) = builder.finish();
    assert_eq!(assets.models().len(), meshes);
    assert_eq!(catalog.inline_handle(first, 1), Some(a));
    assert_eq!(catalog.inline_handle(second, 1), Some(b));
    assert_eq!(catalog.model_bounds(a), Some(first_bounds));
    assert_eq!(catalog.renderer_inline(a), catalog.resolve_inline(first, 1));
    assert_eq!(
        catalog.renderer_inline(b),
        catalog.resolve_inline(second, 1)
    );
    assert!(!catalog.model_has_tags(a));
    assert_eq!(catalog.renderer_inline(i32::MAX), None);
}

#[test]
fn loaded_world_alias_uses_native_bounds_without_reading_a_model_file() {
    let fixture = support::Fixture::new(&[
        ("maps/start.bsp", b"not a mesh or BSP".to_vec()),
        ("maps/unbound.bsp", b"not a mesh or BSP".to_vec()),
    ]);
    let mut assets = Assets::load().unwrap();
    let first = world(&mut assets, 0.0);
    let second = world(&mut assets, 100.0);
    let meshes = assets.models().len();
    let vertices = assets
        .models()
        .iter()
        .map(|model| model.vertices.len())
        .sum::<usize>();
    let indices = assets
        .models()
        .iter()
        .map(|model| model.indices.len())
        .sum::<usize>();
    let materials = assets.materials().len();
    let images = assets.images().len();
    let expected = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([2.0; 3]),
    };
    let mut collision = qa_world::collision::CollisionStore::new();
    let geometry = collision
        .load_hulls(
            vec![],
            vec![],
            vec![],
            vec![qa_world::collision::hulls::HullModel { roots: [-1; 3] }],
            vec![expected],
        )
        .unwrap();
    let native_bounds = collision.model_bounds(geometry, 0).unwrap();
    assert_ne!(
        native_bounds,
        assets.world(first).unwrap().geometry().models[0].bounds
    );
    let mut builder =
        ModelCatalogBuilder::new(assets, [], [first, second], ModelLimits::default()).unwrap();
    builder
        .bind_loaded_world("MAPS\\START.BSP", first, native_bounds)
        .unwrap();
    builder
        .bind_loaded_world("maps/absent.bsp", second, native_bounds)
        .unwrap();
    assert_eq!(builder.loaded_world(b"maps/start.bsp"), Some(first.0));
    assert_eq!(builder.loaded_world(b"MAPS\\ABSENT.BSP"), Some(second.0));
    assert_eq!(builder.loaded_world(b"maps/unbound.bsp"), None);
    for path in [b"maps/start.bsp".as_slice(), b"MAPS\\ABSENT.BSP"] {
        assert_eq!(
            builder
                .precache(&fixture.vfs, path, RuleSetId::Quake)
                .unwrap(),
            expected
        );
        assert_eq!(builder.bounds(path, RuleSetId::Quake3), Some(expected));
    }
    fixture.remove("maps/start.bsp");
    // This is the same post-initialization request that the native precache
    // collection makes; a world alias needs no configured mesh profile.
    for required in [false, true] {
        builder
            .register_request(
                &fixture.vfs,
                ModelRequest {
                    required,
                    ..ModelRequest::native("maps/start.bsp", RuleSetId::Quake, None)
                },
            )
            .unwrap();
    }
    assert!(matches!(
        builder.precache(&fixture.vfs, b"maps/unbound.bsp", RuleSetId::Quake),
        Err(qa_compat::services::CallError::File)
    ));
    let (assets, catalog) = builder.finish();
    assert_eq!(
        catalog.bounds(b"MAPS\\START.BSP", RuleSetId::QuakeWorld),
        Some(expected)
    );
    assert_eq!(catalog.loaded_world(b"MAPS\\START.BSP"), Some(first.0));
    assert_eq!(catalog.loaded_world(b"maps/absent.bsp"), Some(second.0));
    assert_eq!(catalog.loaded_world(b"maps/unbound.bsp"), None);
    assert_eq!(
        catalog.bounds(b"maps/absent.bsp", RuleSetId::Quake2),
        Some(expected)
    );
    assert_eq!(catalog.metadata(b"maps/start.bsp"), None);
    assert_eq!(
        catalog.registration(
            "maps/start.bsp",
            ModelRequest::native("", RuleSetId::Quake, None).profile()
        ),
        None
    );
    assert_eq!(catalog.resolve_inline(first, 0).unwrap().world, first);
    assert_eq!(catalog.resolve_inline(second, 0).unwrap().world, second);
    assert_eq!(catalog.capacity().registrations, 0);
    assert_eq!(catalog.capacity().bindings, 0);
    assert_eq!(catalog.capacity().inline_bindings, 4);
    assert_eq!(assets.models().len(), meshes);
    assert_eq!(
        assets
            .models()
            .iter()
            .map(|model| model.vertices.len())
            .sum::<usize>(),
        vertices
    );
    assert_eq!(
        assets
            .models()
            .iter()
            .map(|model| model.indices.len())
            .sum::<usize>(),
        indices
    );
    assert_eq!(assets.materials().len(), materials);
    assert_eq!(assets.images().len(), images);
}

#[test]
fn loaded_world_alias_repeats_are_idempotent_and_conflicts_do_not_replace_rows() {
    let mut assets = Assets::load().unwrap();
    let first = world(&mut assets, 0.0);
    let second = world(&mut assets, 100.0);
    let bounds = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([2.0; 3]),
    };
    let mut builder = ModelCatalogBuilder::new(
        assets,
        [],
        [first, second],
        ModelLimits {
            models: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(builder.bind_loaded_world("", first, bounds).is_err());
    assert!(builder.bind_loaded_world("*1", first, bounds).is_err());
    assert!(
        builder
            .bind_loaded_world("maps/start.bsp", WorldId(u32::MAX), bounds)
            .is_err()
    );
    assert!(
        builder
            .bind_loaded_world(
                "maps/start.bsp",
                first,
                Bounds {
                    mins: Vec3([f32::NAN; 3]),
                    ..bounds
                }
            )
            .is_err()
    );
    builder
        .bind_loaded_world("maps/start.bsp", first, bounds)
        .unwrap();
    builder
        .bind_loaded_world("MAPS\\START.BSP", first, bounds)
        .unwrap();
    assert!(
        builder
            .bind_loaded_world("maps/start.bsp", second, bounds)
            .is_err()
    );
    assert!(
        builder
            .bind_loaded_world(
                "maps/start.bsp",
                first,
                Bounds {
                    maxs: Vec3([3.0; 3]),
                    ..bounds
                }
            )
            .is_err()
    );
    assert!(
        builder
            .bind_loaded_world("maps/overflow.bsp", first, bounds)
            .is_err()
    );
    assert_eq!(
        builder.bounds(b"maps/start.bsp", RuleSetId::Quake),
        Some(bounds)
    );
    assert_eq!(builder.bounds(b"maps/overflow.bsp", RuleSetId::Quake), None);
    let (_, catalog) = builder.finish();
    assert_eq!(catalog.capacity().names, 2);
    assert_eq!(catalog.capacity().name_bytes, "maps/start.bsp".len());
    assert_eq!(
        catalog.bounds(b"maps/start.bsp", RuleSetId::Quake),
        Some(bounds)
    );
    assert_eq!(catalog.resolve_inline(first, 0).unwrap().world, first);
}

#[test]
fn loaded_world_alias_capacity_and_missing_model0_fail_without_admission() {
    let bounds = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([2.0; 3]),
    };
    for limits in [
        ModelLimits {
            models: 0,
            ..Default::default()
        },
        ModelLimits {
            name_bytes: "maps/start.bsp".len() - 1,
            ..Default::default()
        },
    ] {
        let mut assets = Assets::load().unwrap();
        let id = world(&mut assets, 0.0);
        let mut builder = ModelCatalogBuilder::new(assets, [], [id], limits).unwrap();
        assert!(
            builder
                .bind_loaded_world("maps/start.bsp", id, bounds)
                .is_err()
        );
        assert_eq!(builder.bounds(b"maps/start.bsp", RuleSetId::Quake), None);
        let (_, catalog) = builder.finish();
        assert_eq!(catalog.capacity().names, 1);
        assert_eq!(catalog.capacity().name_bytes, 0);
    }
    let mut assets = Assets::load().unwrap();
    let id = world(&mut assets, 0.0);
    let mut builder = ModelCatalogBuilder::new(assets, [], [], ModelLimits::default()).unwrap();
    assert!(
        builder
            .bind_loaded_world("maps/start.bsp", id, bounds)
            .is_err()
    );
    assert_eq!(builder.bounds(b"maps/start.bsp", RuleSetId::Quake), None);
    let (_, catalog) = builder.finish();
    assert_eq!(catalog.capacity().names, 1);
}

#[test]
fn loaded_world_alias_does_not_replace_or_shift_mesh_profiles() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile = native_profile(RuleSetId::Quake2, palette);
    let id = world(&mut assets, 0.0);
    let meshes = assets.models().len();
    let mut builder =
        ModelCatalogBuilder::new(assets, [profile], [id], ModelLimits::default()).unwrap();
    let mesh_bounds = builder
        .precache(&fixture.vfs, b"models/test/tris.md2", RuleSetId::Quake2)
        .unwrap();
    let native_bounds = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([2.0; 3]),
    };
    assert!(
        builder
            .bind_loaded_world("MODELS\\TEST\\TRIS.MD2", id, native_bounds)
            .is_err()
    );
    builder
        .bind_loaded_world("maps/start.bsp", id, native_bounds)
        .unwrap();
    fixture.remove("models/test/tris.md2");
    builder
        .register_request(
            &fixture.vfs,
            ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(palette)),
        )
        .unwrap();
    assert_eq!(
        builder.bounds(b"models/test/tris.md2", RuleSetId::Quake2),
        Some(mesh_bounds)
    );
    let (assets, catalog) = builder.finish();
    assert_eq!(
        catalog.metadata(b"models/test/tris.md2").unwrap().format,
        qa_formats::model::ModelFormat::Md2
    );
    assert_eq!(catalog.capacity().registrations, 1);
    assert_eq!(catalog.capacity().names, 3);
    assert_eq!(assets.models().len(), meshes + 1);
    assert!(binding(&catalog, "models/test/tris.md2", profile, 0).is_some());
    assert_eq!(catalog.registration("maps/start.bsp", profile), None);
    assert_eq!(
        catalog.bounds(b"maps/start.bsp", RuleSetId::Quake),
        Some(native_bounds)
    );
}

#[test]
fn client_inline_binding_uses_an_explicit_world_and_the_shared_entity_arena() {
    use qa_app::client_scene::{ClientModelBindings, ClientSceneInput};
    use qa_core::{
        configstrings::{ConfigLayout, ConfigStrings},
        primitives::{
            ClientId, EntityAppearance, EntityRecord, ModelRef, PlayerState, ResourceNamespace,
        },
    };
    use qa_gameplay::rules::appearance::AliasRoll;
    use qa_network::{
        commands::packet::Protocol,
        projection::{EntityProjection, PlayerProjection},
    };
    let _fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let first = world_variants(&mut assets, 0.0, &[&[], &[]]);
    let second = world(&mut assets, 100.0);
    assert_eq!(assets.worlds().len(), 2);
    assert_eq!(assets.world(first).unwrap().presentations().count, 2);
    let world_meshes = assets.models().len();
    let profile = native_profile(RuleSetId::Quake2, palette);
    let builder =
        ModelCatalogBuilder::new(assets, [profile], [first, second], ModelLimits::default())
            .unwrap();
    let (assets, catalog) = builder.finish();
    assert_eq!(assets.models().len(), world_meshes);
    assert_eq!(catalog.capacity().inline_bindings, 4);
    let layout = ConfigLayout::QUAKE2;
    let mut configs =
        ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
            .unwrap();
    configs
        .set(layout.resources[0].get(50).unwrap(), b"*1")
        .unwrap();
    let client = ClientId(7);
    let mut bindings = ClientModelBindings::load(client, layout);
    assert_eq!(bindings.bind_all(&configs, &catalog, profile, None), 1);
    let projection = EntityProjection::load(Protocol::Quake2_34);
    let player_projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let player = PlayerState::default();
    let mut record = EntityRecord {
        position: Vec3([12.0, 24.0, 48.0]),
        angles: Vec3([0.0, 45.0, 0.0]),
        appearance: EntityAppearance {
            source: RuleSetId::Quake2,
            models: [
                ModelRef::native(ResourceNamespace::Client(client), 50),
                None,
                None,
                None,
            ],
            ..Default::default()
        },
        ..Default::default()
    };
    let first_presentation = assets.world(first).unwrap().presentations().first;
    for (selected, presentation) in [
        (first, WorldPresentationId(first_presentation)),
        (first, WorldPresentationId(first_presentation + 1)),
        (
            second,
            WorldPresentationId(assets.world(second).unwrap().presentations().first),
        ),
    ] {
        assert_eq!(
            assets.world_presentation(presentation).unwrap().world(),
            selected
        );
        assert_eq!(
            bindings.bind_all(&configs, &catalog, profile, Some((selected, presentation))),
            0
        );
        let mut front = qa_render::FrontEnd::load(Default::default()).unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        let input = ClientSceneInput {
            client,
            current: None,
            previous: None,
            projection: &projection,
            appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
            player_projection: &player_projection,
            bindings: &bindings,
            models: &catalog,
            static_entities: std::slice::from_ref(&record),
            player: &player,
            weapon_view_angles: player.view_angles,
            alias_roll: AliasRoll::default(),
            interpolation: Default::default(),
            gun_animation: None,
            gun_previous: None,
            view_entity: None,
        };
        let counts = input.append(
            &mut frame,
            &assets,
            &qa_render::Refdef::default(),
            false,
            None,
        );
        assert_eq!(
            (counts.models, counts.missing_models, counts.rejected),
            (1, 0, 0)
        );
        let packet = frame.finish();
        let entity = packet.entity(0);
        let qa_render::SceneModel::World {
            world,
            presentation: selected_presentation,
            surfaces,
        } = entity.model
        else {
            panic!("inline became a mesh")
        };
        assert_eq!(world, selected);
        assert_eq!(selected_presentation, presentation);
        assert_eq!(
            surfaces,
            catalog.resolve_inline(selected, 1).unwrap().surfaces
        );
        assert_eq!(entity.origin, record.position);
        assert_eq!(entity.material, None);
    }
    record.appearance.models[0] = ModelRef::native(ResourceNamespace::Client(ClientId(8)), 50);
    let mut front = qa_render::FrontEnd::load(Default::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let input = ClientSceneInput {
        client,
        current: None,
        previous: None,
        projection: &projection,
        appearance: qa_gameplay::rules::appearance::policy(projection.rule_set()),
        player_projection: &player_projection,
        bindings: &bindings,
        models: &catalog,
        static_entities: std::slice::from_ref(&record),
        player: &player,
        weapon_view_angles: player.view_angles,
        alias_roll: AliasRoll::default(),
        interpolation: Default::default(),
        gun_animation: None,
        gun_previous: None,
        view_entity: None,
    };
    assert_eq!(
        input
            .append(
                &mut frame,
                &assets,
                &qa_render::Refdef::default(),
                false,
                None
            )
            .missing_models,
        1
    );
    bindings.reset();
    configs
        .set(layout.resources[0].get(50).unwrap(), b"*1")
        .unwrap();
    assert!(!bindings.config_changed(layout.resources[0].get(50).unwrap(), &configs, &catalog));
}
