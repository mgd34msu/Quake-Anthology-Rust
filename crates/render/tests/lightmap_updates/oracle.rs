use super::*;
use qa_core::primitives::{Bounds, Vec3};
use qa_render::assets::{ImageId, Vertex};
use qa_render::scene::{FrontEnd, Limits, Refdef, SceneEntity, SceneModel, Span};
use qa_render::world::geometry::{
    GeometryKind, GeometryPartition, LightEncoding, LightSource, PatchStats, TextureCoordinates,
    WorldGeometry, WorldModel, WorldSurface, WorldVertex,
};
use qa_render::world::{SurfaceMaterial, VisibleSurface, WorldId};
use qa_world::visibility::{PvsRows, VisLeaf, VisibilityWorld};

fn fixture(
    assets: &mut qa_render::Assets,
    image: ImageId,
    region: AtlasRegion,
    conversion: Option<LightmapConversion>,
    samples: [[u8; 3]; 2],
) -> WorldId {
    let bounds = Bounds {
        mins: Vec3([4.0, -1.0, -1.0]),
        maxs: Vec3([4.0, 1.0, 1.0]),
    };
    let model = WorldModel {
        bounds,
        origin: Vec3::default(),
        surfaces: Span { first: 0, count: 1 },
    };
    let geometry = WorldGeometry {
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
    let visibility = VisibilityWorld::load(
        vec![],
        vec![],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds,
            surfaces: Span { first: 0, count: 1 },
        }],
        vec![0],
        1,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    assets
        .register_world_with_bindings(
            geometry,
            visibility,
            &[SurfaceMaterial {
                lightmap: image,
                lightmap_region: Some(region),
                lightmap_conversion: conversion,
                ..SurfaceMaterial::default()
            }],
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
fn dynamic_atlas_ownership_rejects_aliases_and_overlap_but_admits_adjacent_regions() {
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
    let world = fixture(
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
                .refresh(world, 0, &styles, &assets, |id, rect, pixels| {
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
            .refresh(world, 0, &a, &assets, |_, _, _| {
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
    let q2 = fixture(
        &mut assets,
        image,
        region(0),
        Some(LightmapConversion::Rgb { modulate: 2.0 }),
        [[20, 40, 80], [10, 20, 40]],
    );
    let q1 = fixture(
        &mut assets,
        image,
        region(1),
        Some(LightmapConversion::Scalar(Q1GlScale::Standard)),
        [[9, 8, 7], [1, 2, 3]],
    );
    let q3 = fixture(&mut assets, image, region(2), None, [[230; 3], [5; 3]]);
    let mut updates = AtlasUpdates::load(&assets).unwrap();
    let mut styles = [LightStyle::default(); 256];
    styles[0].rgb = [1.0, 0.5, 0.25];
    styles[3].rgb = [0.5, 1.0, 2.0];
    let mut gpu = assets.image(image).unwrap().rgba.to_vec();
    for (world, expected) in [(q2, [50, 80, 200, 255]), (q1, [10, 10, 10, 255])] {
        assert!(
            updates
                .refresh(world, 0, &styles, &assets, |id, rect, pixels| {
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
    let world = fixture(
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
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::World {
            world,
            surfaces: Span { first: 0, count: 1 }
        },
        origin: Vec3([17.0, -4.0, 2.0]),
        ..SceneEntity::default()
    }));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
                        reference.world,
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
    let world = fixture(
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
            .refresh(world, 0, &styles, &assets, |_, _, _| false)
            .is_err()
    );
    assert!(
        updates
            .refresh(world, 0, &styles, &assets, |_, _, pixels| {
                assert_eq!(pixels, [32, 32, 32, 255]);
                true
            })
            .unwrap()
    );
    styles[0].rgb = [f32::NAN; 3];
    assert!(
        updates
            .refresh(world, 0, &styles, &assets, |_, _, _| false)
            .is_err()
    );
    styles[0].rgb = [0.5; 3];
    assert!(
        updates
            .refresh(world, 0, &styles, &assets, |_, _, pixels| {
                assert_eq!(pixels, [24, 24, 24, 255]);
                true
            })
            .unwrap()
    );
}
