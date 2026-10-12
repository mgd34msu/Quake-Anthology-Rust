#[path = "model_assets/support.rs"]
mod support;

use qa_app::model_assets::{ModelCatalog, ModelLimits, ModelMaterialPolicy, ModelRequest};
use qa_core::primitives::{Bounds, IndexSpan, NameId, RuleSetId, Vec3};
use qa_formats::archive::ArchiveReader;
use qa_render::Assets;
use qa_render::assets::{SoftwareAlias, StageTexture};
use qa_render::shader::{AlphaGen, RgbGen, StageBlend};
use qa_render::world::{
    WorldId,
    geometry::{GeometryPartition, PatchStats, WorldGeometry, WorldModel},
};
use qa_world::visibility::{PvsRows, VisLeaf, VisibilityWorld};

fn load(
    fixture: &support::Fixture,
    assets: &mut Assets,
    requests: &[ModelRequest<'_>],
    limits: ModelLimits,
) -> Result<ModelCatalog, String> {
    ModelCatalog::load(
        &fixture.vfs,
        &mut ArchiveReader::default(),
        assets,
        requests.iter().copied(),
        [],
        limits,
    )
}

fn image(assets: &Assets, catalog: &ModelCatalog, path: &str, skin: u32) -> qa_render::ImageId {
    let binding = catalog.resolve_path(path, skin).unwrap();
    let StageTexture::Image(image) = assets.material(binding.material).unwrap().stages[0].texture
    else {
        panic!("skin material")
    };
    image
}

#[test]
fn native_md2_registration_keeps_frame_major_seams_skins_and_global_palette() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let models_before = assets.models().len();
    let catalog = load(
        &fixture,
        &mut assets,
        &[ModelRequest::native(
            "models/test/tris.md2",
            RuleSetId::Quake2,
            Some(palette),
        )],
        ModelLimits::default(),
    )
    .unwrap();
    let first = catalog.resolve_path("models/test/tris.md2", 0).unwrap();
    let second = catalog.resolve_path("models/test/tris.md2", 1).unwrap();
    assert_eq!(first.model, second.model);
    assert_ne!(first.material, second.material);
    assert_eq!(first.rules, RuleSetId::Quake2);
    assert_eq!(second.rules, RuleSetId::Quake2);
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
            .image(image(&assets, &catalog, "models/test/tris.md2", skin))
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
                    catalog
                        .resolve_path("models/test/tris.md2", skin)
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
    // An MDL can use Q2 presentation independently of its file provenance.
    let catalog = load(
        &fixture,
        &mut assets,
        &[ModelRequest::native(
            "progs/test.mdl",
            RuleSetId::Quake2,
            Some(palette),
        )],
        ModelLimits::default(),
    )
    .unwrap();
    let binding = catalog.resolve_path("progs/test.mdl", 0).unwrap();
    assert_eq!(binding.rules, RuleSetId::Quake2);
    let model = assets.model(binding.model).unwrap();
    assert_eq!(model.frame_count(), 2);
    assert_eq!(model.vertices_per_frame, 3);
    assert_eq!(model.vertices[0].position, Vec3([8.0, 4.0, 0.0]));
    assert_eq!(model.vertices[3].position, Vec3([10.0, 4.0, 0.0]));
    assert_eq!(model.vertices[0].texcoord, [0.75, 0.5]);
    let image = assets
        .image(image(&assets, &catalog, "progs/test.mdl", 0))
        .unwrap();
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        [1, 2]
    );
    assert_eq!(&image.rgba[..8], &[1, 254, 0, 255, 2, 253, 1, 255]);
    assert!(image.prepared.is_some());
}

#[test]
fn canonical_name_registration_is_unique_and_numeric_skin_lookup_falls_back_to_zero() {
    let fixture = support::Fixture::models();
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let models_before = assets.models().len();
    let catalog = load(
        &fixture,
        &mut assets,
        &[
            ModelRequest::native("MODELS\\TEST\\TRIS.MD2", RuleSetId::Quake2, Some(palette)),
            ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(palette)),
        ],
        ModelLimits::default(),
    )
    .unwrap();
    let name = catalog.name_for_path("MODELS\\TEST\\TRIS.MD2").unwrap();
    assert_eq!(catalog.name(name), Some("models/test/tris.md2"));
    assert_eq!(catalog.name_for_path("models/test/tris.md2"), Some(name));
    assert_eq!(
        catalog.resolve_name(name, 99),
        catalog.resolve_name(name, 0)
    );
    assert_eq!(catalog.resolve_name(NameId(u32::MAX), 0), None);
    assert_eq!(catalog.resolve_path("missing.md2", 0), None);
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
    let before = assets.images().len();
    let catalog = load(
        &fixture,
        &mut assets,
        &["models/a.md2", "models/b.md2"]
            .map(|path| ModelRequest::native(path, RuleSetId::Quake2, Some(palette))),
        ModelLimits::default(),
    )
    .unwrap();
    // Filename identity remains distinct even when source bytes are equal.
    assert_ne!(
        catalog.resolve_path("models/a.md2", 0).unwrap().model,
        catalog.resolve_path("models/b.md2", 0).unwrap().model
    );
    assert_eq!(assets.images().len(), before + 2);
    for skin in [0, 1] {
        assert_eq!(
            image(&assets, &catalog, "models/a.md2", skin),
            image(&assets, &catalog, "models/b.md2", skin)
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
        let catalog = load(
            &fixture,
            &mut assets,
            &[ModelRequest::native(
                "models/test/tris.md2",
                rules,
                Some(palette),
            )],
            ModelLimits::default(),
        )
        .unwrap();
        let name = catalog.name_for_path("models/test/tris.md2").unwrap();
        let opaque = catalog.resolve_name(name, 0).unwrap();
        let base_image = image(&assets, &catalog, "models/test/tris.md2", 0);
        for translucent in [false, true] {
            for fullbright in [false, true] {
                let policy = ModelMaterialPolicy {
                    translucent,
                    fullbright,
                };
                let binding = catalog.resolve_name_with(name, 0, policy).unwrap();
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
    let catalog = load(&fixture, &mut assets, &requests, ModelLimits::default()).unwrap();
    assert_eq!(catalog.diagnostics.len(), 3);
    assert_eq!(assets.models().len(), count);
    for diagnostic in &catalog.diagnostics {
        assert_eq!(catalog.resolve_name(diagnostic.name, 0), None);
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
        &mut assets,
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
fn load_limits_and_conflicting_presentations_reject_at_registration() {
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
                &mut assets,
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
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let error = load(
        &fixture,
        &mut assets,
        &[
            ModelRequest::native("models/test/tris.md2", RuleSetId::Quake2, Some(palette)),
            ModelRequest::native("MODELS/TEST/TRIS.MD2", RuleSetId::Quake, Some(palette)),
        ],
        ModelLimits::default(),
    )
    .err()
    .unwrap();
    assert!(error.contains("conflicting model image registration"));
}

fn world(assets: &mut Assets, origin: f32) -> WorldId {
    let bounds = Bounds {
        mins: Vec3([origin; 3]),
        maxs: Vec3([origin + 1.0; 3]),
    };
    let geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: false,
        vertices: vec![],
        indices: vec![],
        boundaries: vec![],
        surfaces: vec![],
        light_samples: vec![],
        models: vec![
            WorldModel {
                bounds,
                origin: Vec3([origin; 3]),
                surfaces: IndexSpan::default(),
            },
            WorldModel {
                bounds,
                origin: Vec3([origin + 1.0; 3]),
                surfaces: IndexSpan::default(),
            },
        ],
        patch_stats: PatchStats::default(),
    };
    let visibility = VisibilityWorld::load(
        vec![],
        vec![],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds,
            surfaces: IndexSpan::default(),
        }],
        vec![],
        0,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    assets.register_world(geometry, visibility, &[]).unwrap()
}

#[test]
fn inline_model_ordinals_stay_in_their_world_namespace_without_copying_geometry() {
    let fixture = support::Fixture::new(&[]);
    let mut assets = Assets::load().unwrap();
    let first = world(&mut assets, 0.0);
    let second = world(&mut assets, 100.0);
    let meshes = assets.models().len();
    let catalog = ModelCatalog::load(
        &fixture.vfs,
        &mut ArchiveReader::default(),
        &mut assets,
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
            &mut ArchiveReader::default(),
            &mut assets,
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
