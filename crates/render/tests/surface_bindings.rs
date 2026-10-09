use qa_core::primitives::{Bounds, Vec3};
use qa_formats::bsp::IndexRange;
use qa_render::{
    Assets, Vertex,
    lightmap::{AtlasBuilder, AtlasRegion, PAGE_SIZE},
    world::{
        SurfaceMaterial,
        geometry::{
            GeometryKind, GeometryPartition, LightEncoding, LightSource, PatchStats,
            TextureCoordinates, WorldGeometry, WorldSurface, WorldVertex,
        },
    },
};
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};

fn inputs(count: usize) -> (WorldGeometry, VisibilityWorld) {
    let bounds = Bounds {
        mins: Vec3([1.0, -1.0, -1.0]),
        maxs: Vec3([1.0, 1.0, 1.0]),
    };
    let geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: true,
        vertices: [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 0.0, 1.0]]
            .map(|position| WorldVertex {
                vertex: Vertex {
                    position: Vec3(position),
                    lightmap_coord: [0.25, 0.75],
                    ..Vertex::default()
                },
                normal: Vec3([-1.0, 0.0, 0.0]),
            })
            .into(),
        indices: vec![0, 1, 2],
        boundaries: vec![IndexRange { first: 0, count: 3 }],
        surfaces: (0..count)
            .map(|index| WorldSurface {
                source_id: index as u32,
                kind: GeometryKind::Triangles,
                vertices: IndexRange { first: 0, count: 3 },
                indices: IndexRange { first: 0, count: 3 },
                boundaries: IndexRange { first: 0, count: 1 },
                plane: None,
                bounds,
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.0; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [16; 2],
                lightmap_grid: [2; 2],
                styles: [0, 255, 255, 255],
                light_source: LightSource::Page(0),
                light_encoding: LightEncoding::Rgb,
                light_samples: IndexRange::default(),
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
            })
            .collect(),
        light_samples: vec![],
        models: vec![],
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
            surfaces: SurfaceSpan {
                first: 0,
                count: count as u32,
            },
        }],
        (0..count as u32).collect(),
        count,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    (geometry, visibility)
}

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
        lightmap: image,
        lightmap_region: Some(region),
        ..SurfaceMaterial::default()
    });
    let (geometry, visibility) = inputs(2);
    let id = assets
        .register_world_with_bindings(geometry, visibility, &materials)
        .unwrap();
    let world = assets.world(id).unwrap();
    for (binding, region, expected) in [
        (world.bindings()[0], red, [17, 0, 0, 255]),
        (world.bindings()[1], blue, [0, 0, 231, 255]),
    ] {
        assert_eq!(binding.lightmap, image);
        assert_eq!(binding.lightmap_region, Some(region));
        assert_eq!(binding.material, materials[0].material);
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
                .register_world_with_bindings(
                    geometry,
                    visibility,
                    &[SurfaceMaterial {
                        lightmap: image,
                        lightmap_region: Some(region),
                        ..SurfaceMaterial::default()
                    }]
                )
                .is_err()
        );
        assert_eq!(assets.worlds().len(), worlds_before);
        assert_eq!(assets.models().len(), models_before);
    }
    let (geometry, visibility) = inputs(1);
    let id = assets
        .register_world_with_bindings(
            geometry,
            visibility,
            &[SurfaceMaterial {
                lightmap: image,
                lightmap_region: Some(valid),
                ..SurfaceMaterial::default()
            }],
        )
        .unwrap();
    assert_eq!(
        assets.world(id).unwrap().bindings()[0].lightmap_region,
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
        .register_world_with_bindings(
            geometry,
            visibility,
            &[SurfaceMaterial {
                lightmap: image,
                ..SurfaceMaterial::default()
            }],
        )
        .unwrap();
    let world = assets.world(id).unwrap();
    assert_eq!(world.bindings()[0].lightmap_region, None);
    assert_eq!(
        world.geometry().vertices[0].vertex.lightmap_coord,
        [0.25, 0.75]
    );
    assert_eq!(
        assets.model(world.mesh()).unwrap().vertices[0].lightmap_coord,
        [0.25, 0.75]
    );
}
