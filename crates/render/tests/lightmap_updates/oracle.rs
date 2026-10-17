use super::*;
use qa_core::primitives::{Bounds, Vec3};
use qa_render::assets::{ImageId, Vertex};
use qa_render::scene::{FrontEnd, Limits, Refdef, SceneEntity, SceneModel, Span};
use qa_render::world::geometry::{
    GeometryKind, GeometryPartition, LightEncoding, LightSource, PatchStats, TextureCoordinates,
    WorldGeometry, WorldModel, WorldSurface, WorldVertex,
};
use qa_render::world::{SurfaceMaterial, VisibleSurface, WorldId, WorldPresentationId};
use qa_world::visibility::{PvsRows, VisLeaf, VisibilityWorld};

fn fixture(
    assets: &mut qa_render::Assets,
    image: ImageId,
    region: AtlasRegion,
    conversion: Option<LightmapConversion>,
    samples: [[u8; 3]; 2],
) -> (WorldId, WorldPresentationId) {
    let world = fixture_variants(assets, &[(image, region, conversion)], samples);
    let presentation = assets.world(world).unwrap().presentations()[0];
    (world, presentation)
}

fn fixture_variants(
    assets: &mut qa_render::Assets,
    variants: &[(ImageId, AtlasRegion, Option<LightmapConversion>)],
    samples: [[u8; 3]; 2],
) -> WorldId {
    fixture_variants_with_surface_count(assets, variants, samples, 1)
}

fn fixture_variants_with_surface_count(
    assets: &mut qa_render::Assets,
    variants: &[(ImageId, AtlasRegion, Option<LightmapConversion>)],
    samples: [[u8; 3]; 2],
    surface_count: u32,
) -> WorldId {
    let (_, region, conversion) = variants[0];
    let bounds = Bounds {
        mins: Vec3([4.0, -1.0, -1.0]),
        maxs: Vec3([4.0, 1.0, 1.0]),
    };
    let model = WorldModel {
        bounds,
        origin: Vec3::default(),
        surfaces: Span { first: 0, count: 1 },
    };
    let mut geometry = WorldGeometry {
        partition: GeometryPartition::Unpartitioned,
        world_has_lightdata: true,
        vertices: [[4.0, -1.0, -1.0], [4.0, 1.0, -1.0], [4.0, 0.0, 1.0]]
            .map(|position| WorldVertex {
                vertex: Vertex {
                    position: Vec3(position),
                    normal: Vec3([-1.0, 0.0, 0.0]),
                    ..Vertex::default()
                },
            })
            .into(),
        indices: vec![0, 1, 2],
        boundaries: vec![Span { first: 0, count: 3 }],
        surfaces: vec![WorldSurface {
            source_id: 0,
            kind: GeometryKind::Triangles,
            vertices: Span { first: 0, count: 3 },
            indices: Span { first: 0, count: 3 },
            boundaries: Span { first: 0, count: 1 },
            plane: None,
            bounds,
            texture_coordinates: TextureCoordinates::Normalized,
            texture_projection: [[0.0; 4]; 2],
            texture_minima: [0; 2],
            texture_extents: [16; 2],
            lightmap_projection: None,
            lightmap_grid: [region.width, region.height],
            styles: [0, 3, 255, 7],
            light_source: if conversion.is_some() {
                LightSource::Samples
            } else {
                LightSource::Page(0)
            },
            light_encoding: LightEncoding::Rgb,
            light_samples: Span {
                first: 0,
                count: region.width * region.height * 2,
            },
            source_texture: None,
            source_texture_info: None,
            source_shader: None,
            source_flags: 0,
            source_contents: 0,
            no_draw: false,
            source_fog: -1,
            source_brush_side: -1,
            source_lightmap: 0,
            lightmap_rect: [0; 4],
            lightmap_origin: Vec3::default(),
            lightmap_vectors: [Vec3::default(); 3],
            patch: None,
        }],
        light_samples: samples
            .into_iter()
            .flat_map(|sample| std::iter::repeat_n(sample, (region.width * region.height) as usize))
            .collect(),
        models: vec![model, model],
        patch_stats: PatchStats::default(),
    };
    let surface = geometry.surfaces[0].clone();
    geometry
        .surfaces
        .extend((1..surface_count).map(|source_id| WorldSurface {
            source_id,
            ..surface.clone()
        }));
    for model in &mut geometry.models {
        model.surfaces.count = surface_count;
    }
    let visibility = VisibilityWorld::load(
        vec![],
        vec![],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds,
            surfaces: Span {
                first: 0,
                count: surface_count,
            },
        }],
        (0..surface_count).collect(),
        surface_count as usize,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    let materials = variants
        .iter()
        .map(|&(image, region, conversion)| {
            vec![
                SurfaceMaterial {
                    shader: qa_render::ShaderBinding {
                        lightmap: image,
                        lightmap_region: Some(region),
                        lightmap_conversion: conversion,
                        light_source: qa_render::world::geometry::LightSource::Samples,
                        ..(SurfaceMaterial::default()).shader
                    },
                    ..SurfaceMaterial::default()
                };
                surface_count as usize
            ]
        })
        .collect::<Vec<_>>();
    let bindings = materials
        .iter()
        .map(|variant| &variant[..])
        .collect::<Vec<_>>();
    assets
        .register_world(
            geometry,
            visibility,
            &(&bindings)
                .iter()
                .map(|surfaces| qa_render::world::WorldPresentationInput::new(surfaces))
                .collect::<Vec<_>>(),
        )
        .unwrap()
}

fn region(x: u32) -> AtlasRegion {
    AtlasRegion {
        page: 71,
        page_size: 4,
        x,
        y: 1,
        width: 1,
        height: 1,
    }
}

#[test]
fn dynamic_atlas_ownership_rejects_cross_world_aliases_and_overlap_but_admits_adjacent_regions() {
    for (a, b, admitted) in [
        (region(1), region(1), false),
        (
            AtlasRegion {
                width: 2,
                ..region(0)
            },
            AtlasRegion {
                width: 2,
                ..region(1)
            },
            false,
        ),
        (
            AtlasRegion {
                width: 2,
                ..region(0)
            },
            AtlasRegion {
                width: 2,
                ..region(2)
            },
            true,
        ),
    ] {
        let mut assets = qa_render::Assets::load().unwrap();
        let image = assets.register_image(4, 4, &[0; 64]).unwrap();
        for rect in [a, b] {
            fixture(
                &mut assets,
                image,
                rect,
                Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
                [[16; 3], [0; 3]],
            );
        }
        assert_eq!(AtlasUpdates::load(&assets).is_ok(), admitted);
    }
}

#[test]
fn per_view_refresh_reuses_one_region_and_only_its_actual_scalar_styles() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    let target = region(1);
    let (_world, presentation) = fixture(
        &mut assets,
        image,
        target,
        Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        [[20, 40, 80], [10, 20, 40]],
    );
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut a = [LightStyle::default(); 256];
    a[0].indexed_scale = 128;
    let mut b = a;
    b[0].indexed_scale = 256;
    b[3].indexed_scale = 128;
    let mut uploads = Vec::new();
    for (styles, expected) in [
        (a, [20, 40, 80, 255]),
        (b, [25, 50, 100, 255]),
        (a, [20, 40, 80, 255]),
    ] {
        assert!(
            updates
                .refresh(presentation, 0, &styles, &assets, |id, rect, pixels| {
                    assert_eq!(id, image);
                    assert_eq!(rect, target);
                    assert_eq!(pixels, expected);
                    uploads.push(pixels.to_vec());
                    true
                })
                .unwrap()
        );
    }
    a[0].rgb = [0.25, 0.5, 0.75];
    a[7].indexed_scale = 77;
    assert!(
        !updates
            .refresh(presentation, 0, &a, &assets, |_, _, _| {
                uploads.push(vec![]);
                true
            })
            .unwrap()
    );
    assert_eq!(uploads.len(), 3);
    assert!(assets.image(image).unwrap().rgba.iter().all(|&b| b == 0));
}

#[test]
fn shared_page_refresh_keeps_each_world_conversion_and_static_q3_texels() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[91; 64]).unwrap();
    let (_, q2) = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Rgb { modulate: 2.0 }),
        [[20, 40, 80], [10, 20, 40]],
    );
    let (_, q1) = fixture(
        &mut assets,
        image,
        region(1),
        Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        [[9, 8, 7], [1, 2, 3]],
    );
    let (_, q3) = fixture(&mut assets, image, region(2), None, [[230; 3], [5; 3]]);
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut styles = [LightStyle::default(); 256];
    styles[0].rgb = [1.0, 0.5, 0.25];
    styles[3].rgb = [0.5, 1.0, 2.0];
    let mut gpu = assets.image(image).unwrap().rgba.to_vec();
    for (presentation, expected) in [(q2, [50, 80, 200, 255]), (q1, [10, 10, 10, 255])] {
        assert!(
            updates
                .refresh(presentation, 0, &styles, &assets, |id, rect, pixels| {
                    assert_eq!(id, image);
                    assert_eq!(pixels, expected);
                    let start = (rect.y as usize * 4 + rect.x as usize) * 4;
                    gpu[start..start + pixels.len()].copy_from_slice(pixels);
                    true
                })
                .unwrap()
        );
    }
    styles[0].indexed_scale = 17;
    assert!(
        !updates
            .refresh(q2, 0, &styles, &assets, |_, _, _| false)
            .unwrap()
    );
    styles[0].rgb = [f32::NAN; 3];
    assert!(
        !updates
            .refresh(q3, 0, &styles, &assets, |_, _, _| false)
            .unwrap()
    );
    assert_eq!(&gpu[(4 + 2) * 4..(4 + 3) * 4], &[91; 4]);
    assert_eq!(&gpu[4 * 4..5 * 4], &[50, 80, 200, 255]);
    assert_eq!(&gpu[5 * 4..6 * 4], &[10, 10, 10, 255]);
}

#[test]
fn static_and_inline_scene_references_refresh_one_owned_atlas_residency() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    let (world, presentation) = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Scalar(Q1GlScale::OriginalOverbright)),
        [[64; 3], [0; 3]],
    );
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(
        world,
        presentation,
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::World {
            world,
            presentation,
            surfaces: Span { first: 0, count: 1 }
        },
        origin: Vec3([17.0, -4.0, 2.0]),
        ..SceneEntity::default()
    }));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let list = frame.finish();
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut submitted = 0;
    for command in list.commands() {
        if let qa_render::scene::Command::View(view) = command {
            let refs = list.surfaces(view.scene.surfaces);
            assert_eq!(refs.len(), 2);
            assert_eq!(refs[0].entity, None);
            assert!(refs[1].entity.is_some());
            for reference in refs {
                updates
                    .refresh(
                        reference.presentation,
                        reference.surface,
                        &view.refdef.lightstyles,
                        &assets,
                        |_, _, pixels| {
                            assert_eq!(pixels, [128, 128, 128, 255]);
                            submitted += 1;
                            true
                        },
                    )
                    .unwrap();
            }
        }
    }
    assert_eq!(submitted, 1);
}

#[test]
fn rejected_upload_and_nonfinite_style_do_not_publish_new_residency() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    let (_world, presentation) = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Rgb { modulate: 1.0 }),
        [[16; 3], [16; 3]],
    );
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut styles = [LightStyle::default(); 256];
    assert!(
        updates
            .refresh(presentation, 0, &styles, &assets, |_, _, _| false)
            .is_err()
    );
    assert!(
        updates
            .refresh(presentation, 0, &styles, &assets, |_, _, pixels| {
                assert_eq!(pixels, [32, 32, 32, 255]);
                true
            })
            .unwrap()
    );
    styles[0].rgb = [f32::NAN; 3];
    assert!(
        updates
            .refresh(presentation, 0, &styles, &assets, |_, _, _| false)
            .is_err()
    );
    styles[0].rgb = [0.5; 3];
    assert!(
        updates
            .refresh(presentation, 0, &styles, &assets, |_, _, pixels| {
                assert_eq!(pixels, [24, 24, 24, 255]);
                true
            })
            .unwrap()
    );
}

#[test]
fn one_world_variants_keep_conversion_uploads_and_residency_independent() {
    for reversed in [false, true] {
        let mut assets = qa_render::Assets::load().unwrap();
        let images =
            [[23; 64], [91; 64]].map(|pixels| assets.register_image(4, 4, &pixels).unwrap());
        let target = region(1);
        let mut variants = [
            (
                images[0],
                target,
                Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
            ),
            (
                images[1],
                target,
                Some(LightmapConversion::Rgb { modulate: 2.0 }),
            ),
        ];
        if reversed {
            variants.reverse();
        }
        let world = fixture_variants(&mut assets, &variants, [[20, 40, 80], [10, 20, 40]]);
        let registrations = assets.world(world).unwrap().presentations();
        assert_eq!(assets.worlds().len(), 1);
        assert_eq!(registrations.len(), 2);
        let q1 = registrations[usize::from(reversed)];
        let q2 = registrations[usize::from(!reversed)];
        assert_ne!(q1, q2);
        assert_eq!(assets.world_presentation(q1).unwrap().world(), world);
        assert_eq!(assets.world_presentation(q2).unwrap().world(), world);
        assert_eq!(
            assets.surface_binding(q1, 0).unwrap().mesh_indices,
            assets.surface_binding(q2, 0).unwrap().mesh_indices
        );
        let mut a = [LightStyle::default(); 256];
        a[0].indexed_scale = 128;
        let mut b = a;
        b[0].rgb = [1.0, 0.5, 0.25];
        b[3].rgb = [0.5, 1.0, 2.0];
        let mut updates = AtlasUpdates::load(&assets).unwrap();
        let mut uploaded = Vec::new();
        for (presentation, styles, image, expected) in [
            (q1, a, images[0], [20, 40, 80, 255]),
            (q2, b, images[1], [50, 80, 200, 255]),
        ] {
            assert!(
                updates
                    .refresh(presentation, 0, &styles, &assets, |id, rect, pixels| {
                        assert_eq!(id, image);
                        assert_eq!(rect, target);
                        assert_eq!(pixels, expected);
                        uploaded.push((id, pixels.to_vec()));
                        true
                    })
                    .unwrap()
            );
        }
        for (presentation, styles) in [(q1, a), (q2, b), (q1, a)] {
            assert!(
                !updates
                    .refresh(presentation, 0, &styles, &assets, |_, _, _| {
                        panic!("peer refresh must not invalidate this presentation")
                    })
                    .unwrap()
            );
        }
        a[0].indexed_scale = 256;
        a[3].indexed_scale = 128;
        assert!(
            updates
                .refresh(q1, 0, &a, &assets, |id, _, pixels| {
                    assert_eq!(id, images[0]);
                    assert_eq!(pixels, [25, 50, 100, 255]);
                    false
                })
                .is_err()
        );
        assert!(
            !updates
                .refresh(q2, 0, &b, &assets, |_, _, _| {
                    panic!("rejected peer upload must retain this presentation")
                })
                .unwrap()
        );
        assert!(
            updates
                .refresh(q1, 0, &a, &assets, |id, _, pixels| {
                    assert_eq!(id, images[0]);
                    assert_eq!(pixels, [25, 50, 100, 255]);
                    uploaded.push((id, pixels.to_vec()));
                    true
                })
                .unwrap()
        );
        assert_eq!(uploaded.len(), 3);
        assert_eq!(assets.image(images[0]).unwrap().rgba.as_ref(), &[23; 64]);
        assert_eq!(assets.image(images[1]).unwrap().rgba.as_ref(), &[91; 64]);
        assert!(
            updates
                .refresh(WorldPresentationId(u32::MAX), 0, &a, &assets, |_, _, _| {
                    panic!("unknown presentation must not upload")
                })
                .is_err()
        );
        assert!(
            updates
                .refresh(q1, 1, &a, &assets, |_, _, _| {
                    panic!("unknown surface must not upload")
                })
                .is_err()
        );
    }
}

#[test]
fn different_conversion_variants_cannot_alias_mutable_atlas_texels() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    fixture_variants(
        &mut assets,
        &[
            (
                image,
                region(0),
                Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
            ),
            (
                image,
                region(0),
                Some(LightmapConversion::Rgb { modulate: 1.0 }),
            ),
        ],
        [[16; 3], [0; 3]],
    );
    assert!(matches!(
        AtlasUpdates::load(&assets),
        Err("GL styled atlas regions overlap")
    ));
}

#[test]
fn appended_presentations_preserve_style_residency_and_grow_upload_scratch() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    let (_, original) = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        [[16; 3], [8; 3]],
    );
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut styles = [LightStyle::default(); 256];
    styles[0].indexed_scale = 128;
    assert!(
        updates
            .refresh(original, 0, &styles, &assets, |_, _, pixels| {
                assert_eq!(pixels, [16, 16, 16, 255]);
                true
            })
            .unwrap()
    );
    assert!(updates.prepare_append(&assets, &[]).unwrap().is_none());
    let expanded = AtlasRegion {
        width: 2,
        height: 2,
        ..region(2)
    };
    let (_, appended) = fixture(
        &mut assets,
        image,
        expanded,
        Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        [[32; 3], [16; 3]],
    );
    let mut candidate = updates.prepare_append(&assets, &[]).unwrap().unwrap();
    assert!(
        !updates
            .refresh(original, 0, &styles, &assets, |_, _, _| panic!(
                "old residency changed while staging"
            ))
            .unwrap()
    );
    assert!(
        updates
            .refresh(appended, 0, &styles, &assets, |_, _, _| panic!(
                "uncommitted presentation uploaded"
            ))
            .is_err()
    );
    assert!(
        !candidate
            .refresh(original, 0, &styles, &assets, |_, _, _| panic!(
                "admitted prefix was not retained"
            ))
            .unwrap()
    );
    let mut uploads = 0;
    assert!(
        candidate
            .refresh(appended, 0, &styles, &assets, |id, rect, pixels| {
                assert_eq!(id, image);
                assert_eq!(rect, expanded);
                assert_eq!(pixels, [32, 32, 32, 255].repeat(4));
                uploads += 1;
                true
            })
            .unwrap()
    );
    assert_eq!(uploads, 1);
    assert!(candidate.prepare_append(&assets, &[]).unwrap().is_none());
    assert!(
        !candidate
            .refresh(appended, 0, &styles, &assets, |_, _, _| panic!(
                "unchanged style uploaded twice"
            ))
            .unwrap()
    );
    styles[0].indexed_scale = 256;
    for presentation in [original, appended] {
        assert!(
            candidate
                .refresh(presentation, 0, &styles, &assets, |_, _, _| true)
                .unwrap()
        );
    }
}

#[test]
fn rejected_atlas_append_preserves_the_old_owner_and_allows_a_clean_retry() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    let (world, original) = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        [[16; 3], [0; 3]],
    );
    let retained = assets.counts();
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let styles = [LightStyle::default(); 256];
    assert!(
        updates
            .refresh(original, 0, &styles, &assets, |_, _, _| true)
            .unwrap()
    );
    let (rejected_world, rejected_presentation) = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Rgb { modulate: 1.0 }),
        [[32; 3], [0; 3]],
    );
    assert!(matches!(
        updates.prepare_append(&assets, &[]),
        Err("GL styled atlas regions overlap")
    ));
    assert!(
        !updates
            .refresh(original, 0, &styles, &assets, |_, _, _| panic!(
                "rejected candidate cleared residency"
            ))
            .unwrap()
    );
    assets.discard_registration_suffix(retained).unwrap();
    assert_eq!(assets.counts(), retained);
    assert_eq!(assets.world(world).unwrap().presentations()[0], original);
    assert!(assets.world(rejected_world).is_none());
    assert!(assets.world_presentation(rejected_presentation).is_none());
    assert!(updates.prepare_append(&assets, &[]).unwrap().is_none());
    let (retry_world, retry_presentation) = fixture(
        &mut assets,
        image,
        region(1),
        Some(LightmapConversion::Rgb { modulate: 1.0 }),
        [[32; 3], [0; 3]],
    );
    assert_eq!(retry_world, rejected_world);
    assert_eq!(retry_presentation, rejected_presentation);
    let mut candidate = updates.prepare_append(&assets, &[]).unwrap().unwrap();
    assert!(
        !candidate
            .refresh(original, 0, &styles, &assets, |_, _, _| panic!(
                "retry cleared old residency"
            ))
            .unwrap()
    );
    assert!(
        candidate
            .refresh(
                retry_presentation,
                0,
                &styles,
                &assets,
                |_, rect, pixels| {
                    assert_eq!(rect, region(1));
                    assert_eq!(pixels, [32, 32, 32, 255]);
                    true
                }
            )
            .unwrap()
    );
    assets.discard_registration_suffix(retained).unwrap();
    assert!(matches!(
        candidate.prepare_append(&assets, &[]),
        Err("GL styled presentation prefix shrank")
    ));
}

#[test]
fn exact_presentation_aliases_share_the_actual_style_residency_across_appends() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[23; 64]).unwrap();
    let target = region(0);
    let conversion = Some(LightmapConversion::Scalar(Q1GlScale::Standard));
    let world = fixture_variants(
        &mut assets,
        &[(image, target, conversion); 2],
        [[20, 40, 80], [10, 20, 40]],
    );
    let presentations = assets.world(world).unwrap().presentations();
    let (first, alias) = (presentations[0], presentations[1]);
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut a = [LightStyle::default(); 256];
    a[0].indexed_scale = 128;
    let mut b = a;
    b[0].indexed_scale = 256;
    b[3].indexed_scale = 128;
    let mut uploaded = Vec::new();
    for (presentation, styles, expected) in [
        (first, a, [20, 40, 80, 255]),
        (alias, b, [25, 50, 100, 255]),
        (first, a, [20, 40, 80, 255]),
    ] {
        assert!(
            updates
                .refresh(presentation, 0, &styles, &assets, |id, rect, pixels| {
                    assert_eq!(id, image);
                    assert_eq!(rect, target);
                    assert_eq!(pixels, expected);
                    uploaded.push(pixels.to_vec());
                    true
                })
                .unwrap()
        );
    }
    assert_eq!(
        uploaded.len(),
        3,
        "A/B/A must restore the texels actually resident in the shared image"
    );
    assert!(
        !updates
            .refresh(alias, 0, &a, &assets, |_, _, _| panic!(
                "the alias already contains A"
            ))
            .unwrap()
    );
    assert!(
        updates
            .refresh(alias, 0, &b, &assets, |_, _, pixels| {
                assert_eq!(pixels, [25, 50, 100, 255]);
                false
            })
            .is_err()
    );
    assert!(
        !updates
            .refresh(first, 0, &a, &assets, |_, _, _| panic!(
                "rejection changed residency"
            ))
            .unwrap()
    );
    assert!(
        updates
            .refresh(alias, 0, &b, &assets, |_, _, pixels| {
                assert_eq!(pixels, [25, 50, 100, 255]);
                true
            })
            .unwrap()
    );
    assert!(
        !updates
            .refresh(first, 0, &b, &assets, |_, _, _| panic!(
                "owner did not observe alias upload"
            ))
            .unwrap()
    );
    let (_, appended) = fixture(
        &mut assets,
        image,
        region(1),
        conversion,
        [[32; 3], [16; 3]],
    );
    let mut candidate = updates.prepare_append(&assets, &[]).unwrap().unwrap();
    for presentation in [first, alias] {
        assert!(
            !candidate
                .refresh(presentation, 0, &b, &assets, |_, _, _| panic!(
                    "append lost the shared owner state"
                ))
                .unwrap()
        );
    }
    assert!(
        updates
            .refresh(appended, 0, &a, &assets, |_, _, _| panic!(
                "uncommitted append reached upload"
            ))
            .is_err()
    );
    assert!(
        candidate
            .refresh(appended, 0, &a, &assets, |id, rect, pixels| {
                assert_eq!(id, image);
                assert_eq!(rect, region(1));
                assert_eq!(pixels, [32, 32, 32, 255]);
                true
            })
            .unwrap()
    );
    assert!(
        candidate
            .refresh(first, 0, &a, &assets, |_, _, pixels| {
                assert_eq!(pixels, [20, 40, 80, 255]);
                true
            })
            .unwrap()
    );
    assert!(
        !candidate
            .refresh(alias, 0, &a, &assets, |_, _, _| panic!(
                "alias topology was not preserved"
            ))
            .unwrap()
    );
    assert!(candidate.prepare_append(&assets, &[]).unwrap().is_none());
    assert_eq!(
        assets.image(image).unwrap().rgba.as_ref(),
        &[23; 64],
        "uploads never mutate admitted source pixels"
    );
}

#[test]
fn identical_rectangles_for_different_source_surfaces_are_not_aliases() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    fixture_variants_with_surface_count(
        &mut assets,
        &[(
            image,
            region(0),
            Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        )],
        [[16; 3], [0; 3]],
        2,
    );
    assert!(matches!(
        AtlasUpdates::load(&assets),
        Err("GL styled atlas regions overlap")
    ));
}

#[test]
fn revised_atlas_image_invalidates_only_its_staged_shared_residency() {
    let mut assets = qa_render::Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[0; 64]).unwrap();
    let conversion = Some(LightmapConversion::Scalar(Q1GlScale::Standard));
    let world = fixture_variants(
        &mut assets,
        &[
            (image, region(0), conversion),
            (image, region(0), conversion),
        ],
        [[16; 3], [8; 3]],
    );
    let presentations = assets.world(world).unwrap().presentations().to_vec();
    let other = assets.register_image(4, 4, &[0; 64]).unwrap();
    let (_, unaffected) = fixture(
        &mut assets,
        other,
        region(0),
        conversion,
        [[32; 3], [16; 3]],
    );
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut styles = [LightStyle::default(); 256];
    styles[0].indexed_scale = 128;
    assert!(
        updates
            .refresh(presentations[0], 0, &styles, &assets, |_, _, _| true)
            .unwrap()
    );
    assert!(
        !updates
            .refresh(presentations[1], 0, &styles, &assets, |_, _, _| panic!(
                "alias uploaded twice"
            ))
            .unwrap()
    );
    assert!(
        updates
            .refresh(unaffected, 0, &styles, &assets, |_, _, _| true)
            .unwrap()
    );
    let _committed = assets
        .replace_image(
            image,
            qa_render::assets::Image::new(4, 4, &[0; 64], None).unwrap(),
        )
        .unwrap();
    let candidate = updates.prepare_append(&assets, &[image]).unwrap().unwrap();
    drop(candidate);
    assert!(
        !updates
            .refresh(presentations[0], 0, &styles, &assets, |_, _, _| panic!(
                "rejected candidate changed residency"
            ))
            .unwrap()
    );
    let mut candidate = updates.prepare_append(&assets, &[image]).unwrap().unwrap();
    assert!(
        candidate
            .refresh(presentations[1], 0, &styles, &assets, |id, _, pixels| {
                assert_eq!(id, image);
                assert_eq!(pixels, [16, 16, 16, 255]);
                true
            })
            .unwrap()
    );
    assert!(
        !candidate
            .refresh(presentations[0], 0, &styles, &assets, |_, _, _| panic!(
                "shared owner uploaded twice"
            ))
            .unwrap()
    );
    assert!(
        !candidate
            .refresh(unaffected, 0, &styles, &assets, |_, _, _| panic!(
                "unrelated image residency reset"
            ))
            .unwrap()
    );
    assert!(candidate.prepare_append(&assets, &[]).unwrap().is_none());
}
