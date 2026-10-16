#[path = "support/world_surfaces.rs"]
mod world_surfaces;
use qa_render::{
    Assets,
    lightmap::{AtlasBuilder, AtlasRegion, PAGE_SIZE},
    world::SurfaceMaterial,
};
use world_surfaces::inputs;

#[test]
#[expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "Keep the independent packed-pixel or triangle oracle and incomplete-tail expectations unchanged"
)]
fn registered_packed_faces_keep_distinct_corners_on_one_numeric_image() {
    let mut packer = AtlasBuilder::load(PAGE_SIZE, 1).unwrap();
    let red = packer.insert(2, 2, &[17, 0, 0].repeat(4)).unwrap();
    let blue = packer.insert(2, 2, &[0, 0, 231].repeat(4)).unwrap();
    assert_eq!((red.page, blue.page, blue.x), (0, 0, red.x + red.width));
    let atlas = packer.finish();
    let rgba: Vec<_> = atlas
        .page(0)
        .unwrap()
        .chunks_exact(3)
        .flat_map(|color| [color[0], color[1], color[2], 255])
        .collect();
    let mut assets = Assets::load().unwrap();
    assets.register_image(1, 1, &[41; 4]).unwrap();
    let image = assets.register_image(PAGE_SIZE, PAGE_SIZE, &rgba).unwrap();
    assert_ne!(image.0, red.page);
    let materials = [red, blue].map(|region| SurfaceMaterial {
        shader: qa_render::ShaderBinding {
            lightmap: image,
            lightmap_region: Some(region),
            ..(SurfaceMaterial::default()).shader
        },
        ..SurfaceMaterial::default()
    });
    let (geometry, visibility) = inputs(2);
    let id = assets
        .register_world(
            geometry,
            visibility,
            &[qa_render::world::WorldPresentationInput::new(&materials)],
        )
        .unwrap();
    let world = assets.world(id).unwrap();
    for (binding, region, expected) in [
        (
            assets
                .world_presentation(world.presentations()[0])
                .unwrap()
                .bindings()[0],
            red,
            [17, 0, 0, 255],
        ),
        (
            assets
                .world_presentation(world.presentations()[0])
                .unwrap()
                .bindings()[1],
            blue,
            [0, 0, 231, 255],
        ),
    ] {
        let binding = assets.shader_binding(binding.shader).unwrap();
        assert_eq!(binding.lightmap, image);
        assert_eq!(binding.lightmap_region, Some(region));
        assert_eq!(binding.material, materials[0].shader.material);
        let pixels = &assets.image(binding.lightmap).unwrap().rgba;
        for sample in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            let uv = binding.lightmap_region.unwrap().uv_at_sample(sample);
            let [x, y] = uv.map(|value| (value * PAGE_SIZE as f32).floor() as usize);
            assert!(x >= region.x as usize && x < (region.x + region.width) as usize);
            assert!(y >= region.y as usize && y < (region.y + region.height) as usize);
            let offset = (y * PAGE_SIZE as usize + x) * 4;
            assert_eq!(&pixels[offset..offset + 4], expected);
        }
    }
}

#[test]
fn packed_bounds_fail_once_before_world_or_mesh_registration() {
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(4, 4, &[128; 64]).unwrap();
    let worlds_before = assets.worlds().len();
    let models_before = assets.models().len();
    let valid = AtlasRegion {
        page: 19,
        page_size: 128,
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    for region in [
        AtlasRegion { width: 0, ..valid },
        AtlasRegion { height: 0, ..valid },
        AtlasRegion { x: 1, ..valid },
        AtlasRegion { y: 1, ..valid },
        AtlasRegion {
            x: u32::MAX,
            width: 2,
            ..valid
        },
        AtlasRegion {
            y: u32::MAX,
            height: 2,
            ..valid
        },
    ] {
        let (geometry, visibility) = inputs(1);
        assert!(
            assets
                .register_world(
                    geometry,
                    visibility,
                    &[qa_render::world::WorldPresentationInput::new(&[
                        SurfaceMaterial {
                            shader: qa_render::ShaderBinding {
                                lightmap: image,
                                lightmap_region: Some(region),
                                ..(SurfaceMaterial::default()).shader
                            },
                            ..SurfaceMaterial::default()
                        }
                    ])]
                )
                .is_err()
        );
        assert_eq!(assets.worlds().len(), worlds_before);
        assert_eq!(assets.models().len(), models_before);
    }
    let (geometry, visibility) = inputs(1);
    let id = assets
        .register_world(
            geometry,
            visibility,
            &[qa_render::world::WorldPresentationInput::new(&[
                SurfaceMaterial {
                    shader: qa_render::ShaderBinding {
                        lightmap: image,
                        lightmap_region: Some(valid),
                        ..(SurfaceMaterial::default()).shader
                    },
                    ..SurfaceMaterial::default()
                },
            ])],
        )
        .unwrap();
    assert_eq!(
        assets
            .shader_binding(
                assets
                    .world_presentation(assets.world(id).unwrap().presentations()[0])
                    .unwrap()
                    .bindings()[0]
                    .shader
            )
            .unwrap()
            .lightmap_region,
        Some(valid)
    );
}

#[test]
fn authored_full_page_keeps_uvs_and_default_region_none() {
    let mut assets = Assets::load().unwrap();
    let image = assets
        .register_image(
            PAGE_SIZE,
            PAGE_SIZE,
            &vec![255; (PAGE_SIZE * PAGE_SIZE * 4) as usize],
        )
        .unwrap();
    let (geometry, visibility) = inputs(1);
    let id = assets
        .register_world(
            geometry,
            visibility,
            &[qa_render::world::WorldPresentationInput::new(&[
                SurfaceMaterial {
                    shader: qa_render::ShaderBinding {
                        lightmap: image,
                        ..(SurfaceMaterial::default()).shader
                    },
                    ..SurfaceMaterial::default()
                },
            ])],
        )
        .unwrap();
    let world = assets.world(id).unwrap();
    assert_eq!(
        assets
            .shader_binding(
                assets
                    .world_presentation(world.presentations()[0])
                    .unwrap()
                    .bindings()[0]
                    .shader
            )
            .unwrap()
            .lightmap_region,
        None
    );
    assert_eq!(
        world.geometry().vertices[0].vertex.lightmap_coord,
        [0.25, 0.75]
    );
    assert_eq!(
        assets.model(world.mesh()).unwrap().vertices[0].lightmap_coord,
        [0.25, 0.75]
    );
}

#[test]
fn invalid_later_presentation_does_not_publish_a_world_mesh_or_partial_variant() {
    let mut assets = Assets::load().unwrap();
    let before = (
        assets.worlds().len(),
        assets.models().len(),
        assets.world_presentations().len(),
    );
    let valid = [SurfaceMaterial::default()];
    let invalid = [SurfaceMaterial {
        shader: qa_render::ShaderBinding::material(
            qa_render::MaterialId(u32::MAX),
            qa_render::world::geometry::LightSource::None,
        ),
        ..Default::default()
    }];
    let (geometry, visibility) = inputs(1);
    assert!(
        assets
            .register_world(
                geometry,
                visibility,
                &[
                    qa_render::world::WorldPresentationInput::new(&valid),
                    qa_render::world::WorldPresentationInput::new(&invalid)
                ]
            )
            .is_err()
    );
    assert_eq!(
        (
            assets.worlds().len(),
            assets.models().len(),
            assets.world_presentations().len()
        ),
        before
    );
    let (geometry, visibility) = inputs(1);
    assert!(assets.register_world(geometry, visibility, &[]).is_err());
    assert_eq!(
        (
            assets.worlds().len(),
            assets.models().len(),
            assets.world_presentations().len()
        ),
        before
    );
}
