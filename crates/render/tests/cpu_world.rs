use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_formats::bsp::IndexRange;
use qa_render::{
    Assets, BlendPhase, CommandList, CpuPresentation, Draw2d, FrontEnd, ImageId, Limits, PaletteId,
    PaletteOperation, PaletteShift, PaletteTransform, PerspectiveStep, Refdef, Viewport,
    assets::{
        AlphaTest, Cull, Filter, MaterialId, MaterialSettings, Sampler, Stage, StageTexture, Vertex,
    },
    cpu::{CpuBackend, CpuLimits},
    surface_cache::{IndexedLighting, IndexedTexture, PaletteLighting},
    world::{SurfaceMaterial, VisibleSurface, WorldId, geometry::*},
};
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};

fn palette(assets: &mut Assets, grades: bool) -> PaletteId {
    let colors: Vec<u8> = (0..256)
        .flat_map(|index| [index as u8, index as u8, index as u8])
        .collect();
    let colormap: Vec<u8> = (0..64)
        .flat_map(|grade| (0..256).map(move |index| if grades { grade as u8 } else { index as u8 }))
        .collect();
    assets
        .register_palette(PaletteLighting::load(&colors, &colormap, None, 224).unwrap())
        .unwrap()
}
fn texture(assets: &mut Assets, palette: PaletteId, index: u8, fence: bool) -> ImageId {
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
        let width = 16 >> mip;
        (0..width * width)
            .map(|pixel| {
                if fence && pixel % width < width / 2 {
                    255
                } else {
                    index
                }
            })
            .collect()
    });
    let texture =
        IndexedTexture::load(16, 16, std::array::from_fn(|i| mips[i].as_slice()), fence).unwrap();
    assets.register_indexed_image(texture, palette).unwrap()
}
fn material(assets: &mut Assets, image: ImageId, fence: bool) -> MaterialId {
    assets
        .register_material(
            "fixture",
            &[Stage {
                texture: StageTexture::Image(image),
                alpha_test: if fence {
                    AlphaTest::GreaterZero
                } else {
                    AlphaTest::None
                },
                sampler: Sampler {
                    filter: Filter::Nearest,
                    ..Sampler::default()
                },
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}
fn world(
    assets: &mut Assets,
    depth: f32,
    material: MaterialId,
    light: Option<u8>,
    partition: GeometryPartition,
) -> WorldId {
    let points = [
        [depth, depth, depth],
        [depth, -depth, depth],
        [depth, -depth, -depth],
        [depth, depth, -depth],
    ];
    let vertices: Vec<_> = points
        .into_iter()
        .enumerate()
        .map(|(i, position)| WorldVertex {
            vertex: Vertex {
                position: Vec3(position),
                texcoord: [[0.0, 0.0], [16.0, 0.0], [16.0, 16.0], [0.0, 16.0]][i],
                ..Vertex::default()
            },
            normal: Vec3([-1.0, 0.0, 0.0]),
        })
        .collect();
    let bounds = Bounds {
        mins: Vec3([depth, -depth, -depth]),
        maxs: Vec3([depth, depth, depth]),
    };
    let geometry = WorldGeometry {
        partition,
        world_has_lightdata: light.is_some(),
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3, 0, 1, 2, 3],
        boundaries: vec![IndexRange { first: 6, count: 4 }],
        surfaces: vec![WorldSurface {
            source_id: 0,
            kind: GeometryKind::Polygon,
            vertices: IndexRange { first: 0, count: 4 },
            indices: IndexRange { first: 0, count: 6 },
            boundaries: IndexRange { first: 0, count: 1 },
            plane: Some(Plane {
                normal: Vec3([-1.0, 0.0, 0.0]),
                distance: -depth,
                axis: None,
            }),
            bounds,
            texture_coordinates: TextureCoordinates::Texels,
            texture_projection: [[0.0, -8.0 / depth, 0.0, 8.0], [0.0, 0.0, -8.0 / depth, 8.0]],
            texture_minima: [0; 2],
            texture_extents: [16; 2],
            lightmap_grid: [2; 2],
            styles: [0, 255, 255, 255],
            light_source: if light.is_some() {
                LightSource::Samples
            } else {
                LightSource::None
            },
            light_encoding: LightEncoding::Luminance,
            light_samples: IndexRange {
                first: 0,
                count: if light.is_some() { 4 } else { 0 },
            },
            source_texture: None,
            source_texture_info: None,
            source_shader: None,
            source_flags: 0,
            source_contents: 0,
            no_draw: false,
            source_fog: -1,
            source_brush_side: -1,
            source_lightmap: -1,
            lightmap_rect: [0; 4],
            lightmap_origin: Vec3::default(),
            lightmap_vectors: [Vec3::default(); 3],
            patch: None,
        }],
        light_samples: light.map_or_else(Vec::new, |sample| vec![[sample; 3]; 4]),
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
            surfaces: SurfaceSpan { first: 0, count: 1 },
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
                material,
                texture_scale: [1.0 / 16.0; 2],
                ..SurfaceMaterial::default()
            }],
        )
        .unwrap()
}
fn view(palette: PaletteId) -> Refdef {
    Refdef {
        viewport: Viewport {
            width: 8,
            height: 8,
            ..Viewport::default()
        },
        near: 0.25,
        far: 64.0,
        fov: [90.0; 2],
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: false,
        },
        blend_phase: BlendPhase::FinalPalette,
        ..Refdef::default()
    }
}
fn packet(
    frontend: &mut FrontEnd,
    worlds: &[(WorldId, u32)],
    view: Refdef,
    assets: &Assets,
) -> CommandList {
    let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
    for &(world, depth_key) in worlds {
        assert!(frame.add_world(
            world,
            &[VisibleSurface {
                surface: 0,
                depth_key
            }]
        ));
    }
    assert!(frame.render_scene(view, &[], assets));
    frame.finish()
}

#[test]
fn native_indexed_world_uses_cache_and_updates_style_generation() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, true);
    let image = texture(&mut assets, palette, 7, false);
    let material = material(&mut assets, image, false);
    let world = world(
        &mut assets,
        2.0,
        material,
        Some(64),
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for (scale, grade, step) in [
        (256, 47, PerspectiveStep::Eight),
        (512, 31, PerspectiveStep::Sixteen),
    ] {
        let mut view = view(palette);
        view.lightstyles[0].indexed_scale = scale;
        view.perspective_step = step;
        let packet = packet(&mut frontend, &[(world, 10)], view, &assets);
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.surfaces, 1);
        assert_eq!(stats.triangles, 0);
        assert!(
            cpu.pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes([grade, grade, grade, 255]))
        );
        assert_eq!(cpu.world_stats().pixels, 64);
        assert!(frontend.recycle(packet).is_ok());
    }
    assert_eq!(cpu.world_stats().cache.fills, 2);
}

#[test]
fn cached_fence_holes_keep_the_opaque_world_behind_them() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, false);
    let background_image = texture(&mut assets, palette, 20, false);
    let fence_image = texture(&mut assets, palette, 10, true);
    let background_material = material(&mut assets, background_image, false);
    let fence_material = material(&mut assets, fence_image, true);
    let background = world(
        &mut assets,
        4.0,
        background_material,
        None,
        GeometryPartition::Unpartitioned,
    );
    let fence = world(
        &mut assets,
        2.0,
        fence_material,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_limits(
        8,
        8,
        &assets,
        CpuLimits {
            cache_bytes: 512,
            max_spans: 1,
        },
    )
    .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let packet = packet(
        &mut frontend,
        &[(background, 0), (fence, 999)],
        view(palette),
        &assets,
    );
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.surfaces, 2);
    assert_eq!(stats.triangles, 0);
    for y in 0..8 {
        for x in 0..8 {
            let value = if x < 4 { 20 } else { 10 };
            assert_eq!(
                cpu.pixels()[y * 8 + x],
                u32::from_le_bytes([value, value, value, 255])
            );
        }
    }
}

#[test]
fn overlapping_certified_worlds_ignore_their_unrelated_bsp_keys() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, false);
    let back_image = texture(&mut assets, palette, 20, false);
    let front_image = texture(&mut assets, palette, 10, false);
    let back_material = material(&mut assets, back_image, false);
    let front_material = material(&mut assets, front_image, false);
    let back = world(
        &mut assets,
        4.0,
        back_material,
        None,
        GeometryPartition::SplitBsp,
    );
    let front = world(
        &mut assets,
        2.0,
        front_material,
        None,
        GeometryPartition::SplitBsp,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let packet = packet(
        &mut frontend,
        &[(back, 0), (front, 999)],
        view(palette),
        &assets,
    );
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([10, 10, 10, 255]))
    );
}

#[test]
fn rgb_world_uses_edge_spans_and_shared_stage_sampling() {
    let mut assets = Assets::load();
    let image = assets.register_image(1, 1, &[11, 22, 33, 255]).unwrap();
    let material = material(&mut assets, image, false);
    let world = world(
        &mut assets,
        2.0,
        material,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let packet = packet(
        &mut frontend,
        &[(world, 0)],
        Refdef {
            cpu_presentation: CpuPresentation::Rgb,
            ..view(PaletteId(0))
        },
        &assets,
    );
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.triangles, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([11, 22, 33, 255]))
    );
}

fn transformed_hud(operation: PaletteOperation) -> Vec<u32> {
    let mut assets = Assets::load();
    let colors = [5, 19, 101].repeat(256);
    let colormap: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    let palette = assets
        .register_palette(PaletteLighting::load(&colors, &colormap, None, 224).unwrap())
        .unwrap();
    let image = texture(&mut assets, palette, 7, false);
    let material = material(&mut assets, image, false);
    let mut cpu = CpuBackend::load(4, 4).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let transform = PaletteTransform {
        operation,
        shifts: [
            PaletteShift {
                destination: [255, 0, 0],
                percent: 25,
            },
            PaletteShift {
                destination: [0, 255, 0],
                percent: 37,
            },
            PaletteShift {
                destination: [0, 0, 255],
                percent: 49,
            },
            PaletteShift {
                destination: [128; 3],
                percent: 61,
            },
        ],
        gamma: std::array::from_fn(|index| 255 - index as u8),
    };
    let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 4,
                height: 2,
                ..Viewport::default()
            },
            blend_viewport: Some(Viewport {
                width: 4,
                height: 4,
                ..Viewport::default()
            }),
            palette_transform: Some(transform),
            blend: [0.2, 0.4, 0.8, 0.25],
            ..view(palette)
        },
        &[],
        &assets
    ));
    assert!(frame.draw_2d(Draw2d {
        rect: [0.0, 0.0, 4.0, 4.0],
        texcoords: [0.0, 0.0, 1.0, 1.0],
        color: [255; 4],
        material
    }));
    let packet = frame.finish();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    cpu.pixels().to_vec()
}
#[test]
fn final_native_palette_preserves_four_integer_rounds_and_covers_indexed_hud() {
    // Four native integer rounds yield [44,61,115], then inverse gamma.
    assert!(
        transformed_hud(PaletteOperation::SequentialShifts)
            .iter()
            .all(|&color| color == u32::from_le_bytes([211, 194, 140, 255]))
    );
}
#[test]
fn native_combined_palette_blend_truncates_before_gamma_lookup() {
    // ref_soft truncates [16.5,39.75,126.75] before gamma lookup.
    assert!(
        transformed_hud(PaletteOperation::ScreenBlend)
            .iter()
            .all(|&color| color == u32::from_le_bytes([239, 216, 129, 255]))
    );
}

#[test]
fn coincident_worlds_use_shared_draw_rank_for_lequal_ties() {
    let mut assets = Assets::load();
    let red_image = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
    let blue_image = assets.register_image(1, 1, &[0, 0, 255, 255]).unwrap();
    let red_material = material(&mut assets, red_image, false);
    let blue_material = material(&mut assets, blue_image, false);
    let red = world(
        &mut assets,
        2.0,
        red_material,
        None,
        GeometryPartition::SplitBsp,
    );
    let blue = world(
        &mut assets,
        2.0,
        blue_material,
        None,
        GeometryPartition::SplitBsp,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let view = Refdef {
        cpu_presentation: CpuPresentation::Rgb,
        ..view(PaletteId(0))
    };
    for references in [
        [(red, 0), (blue, 9000)],
        [(blue, 9000), (red, 0)],
        [(red, 9000), (blue, 0)],
    ] {
        let packet = packet(&mut frontend, &references, view, &assets);
        // Material order puts blue after red independently of raw PVS order.
        let refs = packet.surfaces(
            match packet.commands().iter().find_map(|command| {
                if let qa_render::Command::View(view) = command {
                    Some(view.scene.surfaces)
                } else {
                    None
                }
            }) {
                Some(range) => range,
                None => panic!("missing view"),
            },
        );
        let red_rank = refs
            .iter()
            .find(|reference| reference.world == red)
            .unwrap()
            .draw_rank;
        let blue_rank = refs
            .iter()
            .find(|reference| reference.world == blue)
            .unwrap()
            .draw_rank;
        assert!(blue_rank > red_rank);
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert!(
            cpu.pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes([0, 0, 255, 255]))
        );
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn coincident_entity_and_world_obey_the_same_shared_draw_order() {
    for entity_sort in [2.0, 4.0] {
        let mut assets = Assets::load();
        let red = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
        let blue = assets.register_image(1, 1, &[0, 0, 255, 255]).unwrap();
        let entity_material = assets
            .register_material(
                "entity",
                &[Stage {
                    texture: StageTexture::Image(red),
                    ..Stage::default()
                }],
                MaterialSettings {
                    sort: entity_sort,
                    cull: Cull::None,
                    ..MaterialSettings::default()
                },
            )
            .unwrap();
        let world_material = material(&mut assets, blue, false);
        let world = world(
            &mut assets,
            2.0,
            world_material,
            None,
            GeometryPartition::Unpartitioned,
        );
        let positions = [
            [2.0, 2.0, 2.0],
            [2.0, -2.0, 2.0],
            [2.0, -2.0, -2.0],
            [2.0, 2.0, -2.0],
        ];
        let vertices = positions.map(|position| Vertex {
            position: Vec3(position),
            ..Vertex::default()
        });
        let model = assets
            .register_model(&vertices, &[0, 1, 2, 0, 2, 3], entity_material)
            .unwrap();
        let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_world(
            world,
            &[VisibleSurface {
                surface: 0,
                depth_key: 9000
            }]
        ));
        assert!(frame.add_entity(qa_render::SceneEntity {
            model,
            ..qa_render::SceneEntity::default()
        }));
        assert!(frame.render_scene(
            Refdef {
                cpu_presentation: CpuPresentation::Rgb,
                ..view(PaletteId(0))
            },
            &[],
            &assets
        ));
        let packet = frame.finish();
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.triangles, 2);
        let expected = if entity_sort < 3.0 {
            [0, 0, 255, 255]
        } else {
            [255, 0, 0, 255]
        };
        assert!(
            cpu.pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes(expected))
        );
    }
}
