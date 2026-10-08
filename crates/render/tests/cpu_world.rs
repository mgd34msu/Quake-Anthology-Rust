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

fn cloud_material(assets: &mut Assets, stages: &[Stage]) -> MaterialId {
    cloud_material_with_cull(assets, stages, Cull::None)
}

fn cloud_material_with_cull(assets: &mut Assets, stages: &[Stage], cull: Cull) -> MaterialId {
    assets
        .register_material(
            "cloud fixture",
            stages,
            MaterialSettings {
                sort: 2.0,
                cull,
                sky: Some(qa_render::assets::Sky::Cube {
                    outer_box: None,
                    inner_box: None,
                    clouds: qa_render::sky::CloudSphere::native(384.0),
                    rotation: None,
                    params: qa_render::assets::CubeSkyParams::default(),
                }),
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}

fn cloud_stage(image: ImageId) -> Stage {
    Stage {
        texture: StageTexture::Image(image),
        texgen: qa_render::shader::TexCoordGen::CloudSky {
            radius: 4096.0,
            height: 384.0,
        },
        sampler: Sampler {
            filter: Filter::Nearest,
            ..Sampler::default()
        },
        ..Stage::default()
    }
}

#[test]
fn cloud_grid_stages_draw_once_across_world_and_poly_sources() {
    use qa_render::{
        assets::TcMod,
        shader::{BlendFactor, StageBlend, TexMod},
    };
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[20, 30, 40, 255]).unwrap();
    let add = assets.register_image(1, 1, &[5, 6, 7, 255]).unwrap();
    // Retail tim_hell uses two scrolling/scaled stages and additive clouds.
    let cloud = cloud_material(
        &mut assets,
        &[
            Stage {
                tcmods: [
                    Some(TcMod::Script(TexMod::Scroll([0.05, 0.1]))),
                    Some(TcMod::Script(TexMod::Scale([2.0, 2.0]))),
                    None,
                    None,
                ],
                ..cloud_stage(base)
            },
            Stage {
                blend: Some(StageBlend {
                    source: BlendFactor::One,
                    destination: BlendFactor::One,
                }),
                depth_write: false,
                tcmods: [
                    Some(TcMod::Script(TexMod::Scroll([0.05, 0.06]))),
                    Some(TcMod::Script(TexMod::Scale([3.0, 2.0]))),
                    None,
                    None,
                ],
                ..cloud_stage(add)
            },
        ],
    );
    let sky = world(
        &mut assets,
        2.0,
        cloud,
        None,
        GeometryPartition::Unpartitioned,
    );
    let model_vertices = [
        [2.0, 2.0, 2.0],
        [2.0, -2.0, 2.0],
        [2.0, -2.0, -2.0],
        [2.0, 2.0, -2.0],
    ]
    .map(|position| Vertex {
        position: Vec3(position),
        ..Vertex::default()
    });
    let model = assets
        .register_model(&model_vertices, &[0, 1, 2, 0, 2, 3], cloud)
        .unwrap();
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for time_ms in [0, 10_000] {
        let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_world(
            sky,
            &[VisibleSurface {
                surface: 0,
                depth_key: 0
            }]
        ));
        let vertices = [
            [2.0, 2.0, 2.0],
            [2.0, -2.0, 2.0],
            [2.0, -2.0, -2.0],
            [2.0, 2.0, -2.0],
        ]
        .map(|position| Vertex {
            position: Vec3(position),
            ..Vertex::default()
        });
        assert!(frame.add_poly(cloud, &vertices));
        assert!(frame.add_entity(qa_render::SceneEntity {
            model,
            ..qa_render::SceneEntity::default()
        }));
        assert!(frame.render_scene(
            Refdef {
                cpu_presentation: CpuPresentation::Rgb,
                time_ms,
                ..view(PaletteId(0))
            },
            &[],
            &assets
        ));
        let packet = frame.finish();
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.triangles, 0);
        assert!(cpu.world_stats().spans > 0);
        assert!(
            cpu.pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes([25, 36, 47, 255]))
        );
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn cloud_uv_interpolates_native_grid_diagonal_before_ordered_texmods() {
    use qa_render::{assets::TcMod, shader::TexMod};
    let mut assets = Assets::load();
    let rgba: Vec<_> = (0..128)
        .flat_map(|y| (0..128).flat_map(move |x| [x as u8, y as u8, 0, 255]))
        .collect();
    let image = assets.register_image(128, 128, &rgba).unwrap();
    let clouds = cloud_material(
        &mut assets,
        &[Stage {
            tcmods: [
                Some(TcMod::Script(TexMod::Scroll([0.05, 0.1]))),
                Some(TcMod::Script(TexMod::Scale([2.0, 2.0]))),
                None,
                None,
            ],
            ..cloud_stage(image)
        }],
    );
    let sky = world(
        &mut assets,
        2.0,
        clouds,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let packet = packet(
        &mut frontend,
        &[(sky, 0)],
        Refdef {
            time_ms: 1000,
            cpu_presentation: CpuPresentation::Rgb,
            ..view(PaletteId(0))
        },
        &assets,
    );
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    // Pixel (3,3) lies on FillCloudySkySide's native diagonal between grid
    // (s,t)=(-.25,.25) and (0,0), so it interpolates their UVs equally.
    // R_InitSkyTexCoords intersects the radius4480 sphere centered at Z=-4096.
    let native_uv = |direction: [f64; 3]| {
        let length: f64 = direction.iter().map(|value| value * value).sum();
        let radius = 4096.0;
        let height = 384.0;
        let discriminant = direction[2] * direction[2] * radius * radius
            + length * (2.0 * radius * height + height * height);
        let p = (-direction[2] * radius + discriminant.sqrt()) / length;
        [
            (direction[0] * p / (radius + height)).acos(),
            (direction[1] * p / (radius + height)).acos(),
        ]
    };
    let a = native_uv([1.0, 0.25, 0.25]);
    let b = native_uv([1.0, 0.0, 0.0]);
    let uv = std::array::from_fn::<_, 2, _>(|axis| {
        ((a[axis] + b[axis]) * 0.5 + [0.05, 0.1][axis]) * 2.0
    });
    let texel = uv.map(|value| ((value - value.floor()) * 128.0) as u8);
    assert_eq!(
        cpu.pixels()[3 * 8 + 3],
        u32::from_le_bytes([texel[0], texel[1], 0, 255])
    );
}

#[test]
fn cloud_far_depth_does_not_occlude_world_behind_its_generated_cube() {
    let mut assets = Assets::load();
    let red = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 255, 255]).unwrap();
    let clouds = cloud_material(&mut assets, &[cloud_stage(red)]);
    let solid = material(&mut assets, blue, false);
    let sky = world(
        &mut assets,
        2.0,
        clouds,
        None,
        GeometryPartition::Unpartitioned,
    );
    // zFar/1.75 puts the cloud cube at 36.57, in front of this wall. Native
    // depthRange(1,1) must still let the wall occlude it at every pixel.
    let wall = world(
        &mut assets,
        48.0,
        solid,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let packet = packet(
        &mut frontend,
        &[(sky, 0), (wall, 9000)],
        Refdef {
            cpu_presentation: CpuPresentation::Rgb,
            ..view(PaletteId(0))
        },
        &assets,
    );
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([0, 0, 255, 255]))
    );
}

#[test]
fn cloud_native_generated_winding_obeys_front_and_back_culling() {
    for reverse_source in [false, true] {
        for cull in [Cull::Front, Cull::Back] {
            let mut assets = Assets::load();
            let red = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
            let clouds = cloud_material_with_cull(&mut assets, &[cloud_stage(red)], cull);
            let sky = world_mutated(
                &mut assets,
                2.0,
                1.0,
                clouds,
                None,
                GeometryPartition::Unpartitioned,
                |geometry| {
                    if reverse_source {
                        geometry.indices[..6].copy_from_slice(&[0, 2, 1, 0, 3, 2]);
                        geometry.indices[6..].reverse();
                    }
                },
            );
            let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
            let mut frontend = FrontEnd::load(Limits::default()).unwrap();
            for reverse_projected_winding in [false, true] {
                let axes = [
                    Vec3([1.0, 0.0, 0.0]),
                    Vec3([0.0, if reverse_projected_winding { -1.0 } else { 1.0 }, 0.0]),
                    Vec3([0.0, 0.0, 1.0]),
                ];
                let packet = packet(
                    &mut frontend,
                    &[(sky, 0)],
                    Refdef {
                        cpu_presentation: CpuPresentation::Rgb,
                        axes,
                        ..view(PaletteId(0))
                    },
                    &assets,
                );
                assert_eq!(cpu.render(&packet, &assets).rejected, 0);
                // tr_sky.c FillCloudySkySide generates its own winding regardless
                // of source winding; tr_shade.c applies the material's cullType.
                let expected = if (cull == Cull::Front) != reverse_projected_winding {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 0, 255]
                };
                assert!(
                    cpu.pixels()
                        .iter()
                        .all(|&pixel| pixel == u32::from_le_bytes(expected))
                );
                assert!(frontend.recycle(packet).is_ok());
            }
        }
    }
}

#[test]
fn equal_cloud_stage_matches_clear_far_depth_and_rejects_nearer_geometry() {
    let mut assets = Assets::load();
    let red = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 255, 255]).unwrap();
    let clouds = cloud_material(
        &mut assets,
        &[Stage {
            depth_func: qa_render::assets::DepthFunc::Equal,
            depth_write: false,
            ..cloud_stage(red)
        }],
    );
    let solid = material(&mut assets, blue, false);
    let sky = world(
        &mut assets,
        2.0,
        clouds,
        None,
        GeometryPartition::Unpartitioned,
    );
    let wall = world(
        &mut assets,
        48.0,
        solid,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for (references, expected) in [
        (vec![(sky, 0)], [255, 0, 0, 255]),
        (vec![(sky, 0), (wall, 9000)], [0, 0, 255, 255]),
    ] {
        let packet = packet(
            &mut frontend,
            &references,
            Refdef {
                cpu_presentation: CpuPresentation::Rgb,
                ..view(PaletteId(0))
            },
            &assets,
        );
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert!(
            cpu.pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes(expected))
        );
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn clipped_cube_uses_native_face_uv_and_typed_rotation() {
    use qa_render::{
        assets::{CubeSkyParams, Sky},
        sky::{CloudSphere, Rotation},
    };
    let mut assets = Assets::load();
    let images = std::array::from_fn(|face| {
        // A 2x2 native-oriented image makes texture axes observable.
        let base = (face as u8 + 1) * 20;
        let bytes: Vec<_> = [base, base + 1, base + 2, base + 3]
            .into_iter()
            .flat_map(|value| [value, value, value, 255])
            .collect();
        assets.register_image(2, 2, &bytes).unwrap()
    });
    let sky_material = assets
        .register_material(
            "rotating cube",
            &[],
            MaterialSettings {
                sort: 2.0,
                cull: Cull::None,
                sky: Some(Sky::Cube {
                    outer_box: Some(images),
                    inner_box: None,
                    clouds: CloudSphere::native(384.0),
                    rotation: Some(Rotation {
                        axis: Vec3([0.0, 0.0, 1.0]),
                        degrees_per_second: 90.0,
                    }),
                    params: CubeSkyParams {
                        sampler: Sampler {
                            filter: Filter::Nearest,
                            ..CubeSkyParams::default().sampler
                        },
                        ..CubeSkyParams::default()
                    },
                }),
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let sky = world(
        &mut assets,
        2.0,
        sky_material,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for (time_ms, base) in [(0, 20u8), (1000, 80u8)] {
        let packet = packet(
            &mut frontend,
            &[(sky, 0)],
            Refdef {
                cpu_presentation: CpuPresentation::Rgb,
                time_ms,
                ..view(PaletteId(0))
            },
            &assets,
        );
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        for y in 0..8 {
            for x in 0..8 {
                let color = base + (y / 4) as u8 * 2 + (x / 4) as u8;
                assert_eq!(
                    cpu.pixels()[y * 8 + x],
                    u32::from_le_bytes([color, color, color, 255])
                );
            }
        }
        assert!(frontend.recycle(packet).is_ok());
    }
}

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
    world_with_extent(assets, depth, 1.0, material, light, partition)
}
fn world_with_extent(
    assets: &mut Assets,
    depth: f32,
    extent: f32,
    material: MaterialId,
    light: Option<u8>,
    partition: GeometryPartition,
) -> WorldId {
    world_mutated(assets, depth, extent, material, light, partition, |_| {})
}
fn world_mutated(
    assets: &mut Assets,
    depth: f32,
    extent: f32,
    material: MaterialId,
    light: Option<u8>,
    partition: GeometryPartition,
    mutate: impl FnOnce(&mut WorldGeometry),
) -> WorldId {
    world_bound(
        assets,
        depth,
        extent,
        SurfaceMaterial {
            material,
            texture_scale: [1.0 / 16.0; 2],
            ..SurfaceMaterial::default()
        },
        light,
        partition,
        mutate,
    )
}
fn world_bound(
    assets: &mut Assets,
    depth: f32,
    extent: f32,
    binding: SurfaceMaterial,
    light: Option<u8>,
    partition: GeometryPartition,
    mutate: impl FnOnce(&mut WorldGeometry),
) -> WorldId {
    let size = depth * extent;
    let points = [
        [depth, size, size],
        [depth, -size, size],
        [depth, -size, -size],
        [depth, size, -size],
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
        mins: Vec3([depth, -size, -size]),
        maxs: Vec3([depth, size, size]),
    };
    let mut geometry = WorldGeometry {
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
            texture_projection: [[0.0, -8.0 / size, 0.0, 8.0], [0.0, 0.0, -8.0 / size, 8.0]],
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
    mutate(&mut geometry);
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
        .register_world_with_bindings(geometry, visibility, &[binding])
        .unwrap()
}

fn rgba_pair(assets: &mut Assets, base: ImageId, light_first: bool, identity: bool) -> MaterialId {
    use qa_render::shader::{BlendFactor, RgbGen, StageBlend, TexCoordGen};
    let base_stage = Stage {
        texture: StageTexture::Image(base),
        sampler: Sampler {
            filter: Filter::Nearest,
            ..Sampler::default()
        },
        rgb_gen: if identity {
            RgbGen::IdentityLighting
        } else {
            RgbGen::Identity
        },
        ..Stage::default()
    };
    let light_stage = Stage {
        texture: StageTexture::Lightmap,
        texgen: TexCoordGen::Lightmap,
        sampler: Sampler {
            wrap: qa_render::assets::Wrap::Clamp,
            filter: Filter::Nearest,
            mipmaps: false,
        },
        ..Stage::default()
    };
    let mut stages = if light_first {
        [light_stage, base_stage]
    } else {
        [base_stage, light_stage]
    };
    stages[1].blend = Some(StageBlend {
        source: BlendFactor::DestinationColor,
        destination: BlendFactor::Zero,
    });
    stages[1].depth_write = false;
    stages[1].depth_func = qa_render::assets::DepthFunc::Equal;
    assets
        .register_material(
            "static RGB pair",
            &stages,
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}

fn rgba_world(
    assets: &mut Assets,
    material: MaterialId,
    lightmap: ImageId,
    uv: [[f32; 2]; 4],
    light_uv: [[f32; 2]; 4],
    region: Option<qa_render::lightmap::AtlasRegion>,
    depth: f32,
) -> WorldId {
    world_bound(
        assets,
        depth,
        1.0,
        SurfaceMaterial {
            material,
            lightmap,
            lightmap_region: region,
            texture_scale: [1.0; 2],
        },
        None,
        GeometryPartition::Unpartitioned,
        |geometry| {
            geometry.surfaces[0].texture_coordinates = TextureCoordinates::Normalized;
            geometry.surfaces[0].texture_extents = [0; 2];
            for (index, vertex) in geometry.vertices.iter_mut().enumerate() {
                vertex.vertex.texcoord = uv[index];
                vertex.vertex.lightmap_coord = light_uv[index];
            }
        },
    )
}

const UNIT_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
fn rgba_view() -> Refdef {
    Refdef {
        cpu_presentation: CpuPresentation::Rgb,
        ..view(PaletteId(0))
    }
}

#[test]
fn static_base_lightmap_pairs_cache_in_both_orders_and_reuse_blocks() {
    for light_first in [false, true] {
        let mut assets = Assets::load();
        let base = assets.register_image(1, 1, &[127, 151, 173, 255]).unwrap();
        let light = assets.register_image(1, 1, &[93, 101, 117, 255]).unwrap();
        let material = rgba_pair(&mut assets, base, light_first, false);
        let world = rgba_world(&mut assets, material, light, UNIT_UV, UNIT_UV, None, 2.0);
        let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let first = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
        assert_eq!(cpu.render(&first, &assets).rejected, 0);
        let expected = u32::from_le_bytes([46, 60, 79, 255]);
        assert!(cpu.pixels().iter().all(|&pixel| pixel == expected));
        assert_eq!(cpu.world_stats().stage_spans, 0);
        assert_eq!(cpu.world_stats().rgba_pixels, 64);
        assert!(cpu.world_stats().rgba_fills > 0);
        let fills = cpu.world_stats().cache.fills;
        assert!(frontend.recycle(first).is_ok());
        let second = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
        assert_eq!(cpu.render(&second, &assets).rejected, 0);
        assert_eq!(cpu.world_stats().cache.fills, fills);
        assert_eq!(cpu.world_stats().rgba_fills, 0);
        assert!(cpu.world_stats().rgba_hits > 0);
        assert!(cpu.pixels().iter().all(|&pixel| pixel == expected));
    }
}

#[test]
fn static_cache_preserves_negative_constant_and_one_dimensional_uv_area() {
    let mut assets = Assets::load();
    let base = assets
        .register_image(2, 1, &[200, 0, 0, 255, 0, 200, 0, 255])
        .unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let uv_sets = [
        UNIT_UV,
        UNIT_UV.map(|uv| [uv[0] - 1.0, uv[1] - 1.0]),
        [[-0.25, 0.0]; 4],
        UNIT_UV.map(|uv| [uv[0], 0.0]),
    ];
    let worlds = uv_sets.map(|uv| rgba_world(&mut assets, material, light, uv, UNIT_UV, None, 2.0));
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut reference = Vec::new();
    for (index, world) in worlds.into_iter().enumerate() {
        let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
        assert_eq!(cpu.render(&frame, &assets).rejected, 0);
        assert_eq!(cpu.world_stats().rgba_pixels, 64);
        assert_eq!(cpu.world_stats().stage_pixels, 0);
        if index == 0 {
            reference = cpu.pixels().to_vec();
        } else if index == 2 {
            assert!(
                cpu.pixels()
                    .iter()
                    .all(|&pixel| pixel == u32::from_le_bytes([0, 200, 0, 255]))
            );
        } else {
            assert_eq!(cpu.pixels(), reference);
        }
        assert!(frontend.recycle(frame).is_ok());
    }
}

#[test]
fn cached_face_lightmap_clamps_taps_to_its_atlas_rectangle() {
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[255; 4]).unwrap();
    let light = assets
        .register_image(
            4,
            1,
            &[
                255, 0, 0, 255, 64, 64, 64, 255, 64, 64, 64, 255, 0, 255, 0, 255,
            ],
        )
        .unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let world = rgba_world(
        &mut assets,
        material,
        light,
        UNIT_UV,
        UNIT_UV,
        Some(qa_render::lightmap::AtlasRegion {
            page: 99,
            x: 1,
            y: 0,
            width: 2,
            height: 1,
        }),
        2.0,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().stage_spans, 0);
    for pixel in cpu.pixels() {
        let color = pixel.to_le_bytes();
        assert_eq!(color[0], color[1]);
        assert_eq!(color[1], color[2]);
        assert_eq!(color[0], 64);
    }
    assert_eq!(cpu.pixels()[0].to_le_bytes(), [64, 64, 64, 255]);
    assert_eq!(cpu.pixels()[7].to_le_bytes(), [64, 64, 64, 255]);
}

#[test]
fn cache_uses_native_mips_and_invalidates_explicit_identity_light() {
    use qa_render::assets::upload::{MipmapBuild, UploadParams};
    let mut assets = Assets::load();
    let source: Vec<_> = (0..64)
        .flat_map(|y| {
            (0..64).flat_map(move |x| {
                let value = if (x + y) % 2 == 0 { 0 } else { 200 };
                [value, value, value, 255]
            })
        })
        .collect();
    let base = assets.register_image(64, 64, &source).unwrap();
    assets
        .prepare_image(
            base,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = rgba_pair(&mut assets, base, false, true);
    let world = rgba_world(&mut assets, material, light, UNIT_UV, UNIT_UV, None, 2.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let first = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&first, &assets).rejected, 0);
    assert!(cpu.world_stats().rgba_minified_spans > 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([100, 100, 100, 255]))
    );
    let fills = cpu.world_stats().cache.fills;
    assert!(frontend.recycle(first).is_ok());
    let dim = packet(
        &mut frontend,
        &[(world, 0)],
        Refdef {
            identity_light: 0.5,
            ..rgba_view()
        },
        &assets,
    );
    assert_eq!(cpu.render(&dim, &assets).rejected, 0);
    assert!(cpu.world_stats().cache.fills > fills);
    let dimmed = (100.0_f32 * (0.5 * 255.0) as u8 as f32 / 255.0).round() as u8;
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([dimmed, dimmed, dimmed, 255]))
    );
    assert!(frontend.recycle(dim).is_ok());
    let near = packet(
        &mut frontend,
        &[(world, 0)],
        Refdef {
            fov: [10.0; 2],
            ..rgba_view()
        },
        &assets,
    );
    assert_eq!(cpu.render(&near, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_minified_spans, 0);
    assert!(cpu.pixels().iter().any(|pixel| pixel.to_le_bytes()[0] == 0));
    assert!(
        cpu.pixels()
            .iter()
            .any(|pixel| pixel.to_le_bytes()[0] == 200)
    );
}

#[test]
fn cached_worlds_keep_independent_lightmaps_and_shared_depth_order() {
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[255; 4]).unwrap();
    let red = assets.register_image(1, 1, &[200, 0, 0, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 200, 255]).unwrap();
    let material = rgba_pair(&mut assets, base, true, false);
    let front = rgba_world(&mut assets, material, red, UNIT_UV, UNIT_UV, None, 2.0);
    let rear = rgba_world(&mut assets, material, blue, UNIT_UV, UNIT_UV, None, 4.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(
        &mut frontend,
        &[(rear, 0), (front, 9000)],
        rgba_view(),
        &assets,
    );
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_pixels, 64);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([200, 0, 0, 255]))
    );
    assert!(frontend.recycle(frame).is_ok());
    let clipped = packet(
        &mut frontend,
        &[(front, 0)],
        Refdef {
            near: 3.0,
            ..rgba_view()
        },
        &assets,
    );
    assert_eq!(cpu.render(&clipped, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_pixels, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|pixel| pixel.to_le_bytes()[..3] == [0; 3])
    );
    assert!(frontend.recycle(clipped).is_ok());
    let second = packet(&mut frontend, &[(rear, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&second, &assets).rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([0, 0, 200, 255]))
    );
}

#[test]
fn nearest_cache_mip_keeps_native_integer_texel_boundaries() {
    use qa_render::assets::upload::{MipmapBuild, UploadParams};
    let mut assets = Assets::load();
    let base = assets
        .register_image(
            4,
            1,
            &[
                200, 0, 0, 255, 200, 0, 0, 255, 0, 200, 0, 255, 0, 200, 0, 255,
            ],
        )
        .unwrap();
    assets
        .prepare_image(
            base,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let world = rgba_world(
        &mut assets,
        material,
        light,
        UNIT_UV.map(|uv| [uv[0] * 4.0, 0.5]),
        UNIT_UV,
        None,
        2.0,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert!(cpu.world_stats().rgba_minified_spans > 0);
    assert_eq!(cpu.world_stats().stage_spans, 0);
    for row in cpu.pixels().chunks_exact(8) {
        for (x, pixel) in row.iter().enumerate() {
            assert_eq!(
                pixel.to_le_bytes(),
                if x % 2 == 0 {
                    [200, 0, 0, 255]
                } else {
                    [0, 200, 0, 255]
                }
            );
        }
    }
}

#[test]
fn insufficient_aligned_cache_budget_keeps_generic_world_sampling() {
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[127, 151, 173, 255]).unwrap();
    let light = assets.register_image(1, 1, &[93, 101, 117, 255]).unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let world = rgba_world(&mut assets, material, light, UNIT_UV, UNIT_UV, None, 2.0);
    let mut cpu = CpuBackend::load_with_limits(
        8,
        8,
        &assets,
        CpuLimits {
            cache_bytes: 36,
            max_spans: 4096,
        },
    )
    .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert_eq!(cpu.world_stats().cache.fills, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel.to_le_bytes() == [46, 60, 79, 255])
    );
}

#[test]
fn transformed_2d_coordinates_select_the_native_prepared_mip() {
    use qa_render::{
        assets::{
            TcMod,
            upload::{MipmapBuild, UploadParams},
        },
        shader::TexMod,
    };
    let mut assets = Assets::load();
    let pixels: Vec<_> = (0..256)
        .flat_map(|y| {
            (0..256).flat_map(move |x| {
                let value = if ((x >> 3) + (y >> 3)) % 2 == 0 {
                    0
                } else {
                    200
                };
                [value, value, value, 255]
            })
        })
        .collect();
    let image = assets.register_image(256, 256, &pixels).unwrap();
    assets
        .prepare_image(
            image,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let material = assets
        .register_material(
            "scaled prepared 2D image",
            &[Stage {
                texture: StageTexture::Image(image),
                sampler: Sampler {
                    filter: Filter::Nearest,
                    ..Sampler::default()
                },
                tcmods: [
                    Some(TcMod::Script(TexMod::Scale([16.0; 2]))),
                    None,
                    None,
                    None,
                ],
                ..Stage::default()
            }],
            MaterialSettings::default(),
        )
        .unwrap();
    let mut cpu = CpuBackend::load(64, 64).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.draw_2d(Draw2d {
        rect: [0.0, 0.0, 64.0, 64.0],
        texcoords: [0.0, 0.0, 1.0, 1.0],
        material,
        color: [255; 4],
    }));
    let frame = frame.finish();
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    // The evaluated scale selects mip6. Raw gradients select mip2, whose
    // samples land on black cells instead of the averaged gray native mip.
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel.to_le_bytes() == [100, 100, 100, 255])
    );
}

#[test]
fn linear_rank_one_texture_keeps_filtering_before_byte_rounding() {
    use qa_render::shader::{BlendFactor, StageBlend, TexCoordGen};
    let mut assets = Assets::load();
    let pixels: Vec<_> = [0, 0, 1, 3]
        .into_iter()
        .flat_map(|value| [value, value, value, 255])
        .collect();
    let base = assets.register_image(2, 2, &pixels).unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = assets
        .register_material(
            "rank-one filtered texture",
            &[
                Stage {
                    texture: StageTexture::Image(base),
                    sampler: Sampler {
                        wrap: qa_render::assets::Wrap::Clamp,
                        filter: Filter::Linear,
                        mipmaps: false,
                    },
                    ..Stage::default()
                },
                Stage {
                    texture: StageTexture::Lightmap,
                    texgen: TexCoordGen::Lightmap,
                    blend: Some(StageBlend {
                        source: BlendFactor::DestinationColor,
                        destination: BlendFactor::Zero,
                    }),
                    depth_write: false,
                    depth_func: qa_render::assets::DepthFunc::Equal,
                    ..Stage::default()
                },
            ],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let uv = UNIT_UV.map(|uv| [uv[0] - 0.0625, 0.5]);
    let world = rgba_world(&mut assets, material, light, uv, UNIT_UV, None, 2.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    assert_eq!(cpu.pixels()[4].to_le_bytes(), [1, 1, 1, 255]);
}

#[test]
fn variable_linear_lightmap_preserves_independent_stage_filtering() {
    use qa_render::shader::{BlendFactor, StageBlend, TexCoordGen};
    let mut assets = Assets::load();
    let base = assets
        .register_image(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255])
        .unwrap();
    let light = assets
        .register_image(2, 1, &[255, 255, 255, 255, 0, 0, 0, 255])
        .unwrap();
    let sampler = Sampler {
        wrap: qa_render::assets::Wrap::Clamp,
        filter: Filter::Linear,
        mipmaps: false,
    };
    let material = assets
        .register_material(
            "independent filters",
            &[
                Stage {
                    texture: StageTexture::Image(base),
                    sampler,
                    ..Stage::default()
                },
                Stage {
                    texture: StageTexture::Lightmap,
                    texgen: TexCoordGen::Lightmap,
                    sampler,
                    blend: Some(StageBlend {
                        source: BlendFactor::DestinationColor,
                        destination: BlendFactor::Zero,
                    }),
                    depth_write: false,
                    depth_func: qa_render::assets::DepthFunc::Equal,
                    ..Stage::default()
                },
            ],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let uv = UNIT_UV.map(|uv| [uv[0] - 0.0625, 0.5]);
    let world = rgba_world(&mut assets, material, light, uv, uv, None, 2.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    // Separate filters at UV .5: first stage rounds127.5 to128; .5 light
    // multiplies that byte to64. A filtered baked product would be black.
    assert_eq!(cpu.pixels()[4].to_le_bytes(), [64, 64, 64, 255]);
}

#[test]
fn linear_identity_pair_keeps_independent_sampling_across_view_lighting() {
    use qa_render::shader::{BlendFactor, RgbGen, StageBlend, TexCoordGen};
    let mut assets = Assets::load();
    let base = assets
        .register_image(2, 1, &[0, 0, 0, 255, 255, 255, 255, 255])
        .unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let sampler = Sampler {
        wrap: qa_render::assets::Wrap::Clamp,
        filter: Filter::Linear,
        mipmaps: false,
    };
    let material = assets
        .register_material(
            "view identity gate",
            &[
                Stage {
                    texture: StageTexture::Image(base),
                    sampler,
                    rgb_gen: RgbGen::IdentityLighting,
                    ..Stage::default()
                },
                Stage {
                    texture: StageTexture::Lightmap,
                    texgen: TexCoordGen::Lightmap,
                    blend: Some(StageBlend {
                        source: BlendFactor::DestinationColor,
                        destination: BlendFactor::Zero,
                    }),
                    depth_write: false,
                    depth_func: qa_render::assets::DepthFunc::Equal,
                    ..Stage::default()
                },
            ],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let uv = UNIT_UV.map(|uv| [uv[0] - 0.0625, uv[1]]);
    let world = rgba_world(&mut assets, material, light, uv, UNIT_UV, None, 2.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let first = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&first, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    assert_eq!(cpu.pixels()[4].to_le_bytes(), [128, 128, 128, 255]);
    assert!(frontend.recycle(first).is_ok());
    let dim = packet(
        &mut frontend,
        &[(world, 0)],
        Refdef {
            identity_light: 0.5,
            ..rgba_view()
        },
        &assets,
    );
    assert_eq!(cpu.render(&dim, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    assert_eq!(cpu.pixels()[4].to_le_bytes(), [64, 64, 64, 255]);
}

#[test]
fn changed_prepared_image_disables_stale_cold_recipe_until_reload() {
    use qa_render::assets::upload::UploadParams;
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[200, 0, 0, 255]).unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let world = rgba_world(&mut assets, material, light, UNIT_UV, UNIT_UV, None, 2.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let first = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&first, &assets).rejected, 0);
    assert!(cpu.world_stats().rgba_spans > 0);
    assert!(frontend.recycle(first).is_ok());
    assets
        .prepare_image_with_rgba(base, &[0, 200, 0, 255], UploadParams::default())
        .unwrap();
    let second = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&second, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|pixel| pixel.to_le_bytes() == [0, 200, 0, 255])
    );
}

#[test]
fn prepared_lightmap_region_mismatch_is_scoped_before_sampling() {
    use qa_render::assets::upload::{ExtentRound, UploadExtent, UploadParams};
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[255; 4]).unwrap();
    let light = assets.register_image(8, 8, &[255; 8 * 8 * 4]).unwrap();
    assets
        .prepare_image(
            light,
            UploadParams {
                extent: UploadExtent::PowerOfTwo {
                    round: ExtentRound::Up,
                    drop: 1,
                    max_dimension: 8,
                },
                ..UploadParams::default()
            },
        )
        .unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let world = rgba_world(
        &mut assets,
        material,
        light,
        UNIT_UV,
        UNIT_UV,
        Some(qa_render::lightmap::AtlasRegion {
            page: 0,
            x: 4,
            y: 0,
            width: 4,
            height: 4,
        }),
        2.0,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert!(cpu.render(&frame, &assets).rejected > 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
}

#[test]
fn tiny_uv_basis_keeps_finite_generic_geometry_when_cache_fields_overflow() {
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[255; 4]).unwrap();
    let light = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let uv = UNIT_UV.map(|uv| [uv[0] * 1.0e-39, uv[1] * 1.0e-39]);
    let world = rgba_world(&mut assets, material, light, uv, UNIT_UV, None, 2.0);
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert_eq!(cpu.world_stats().rgba_spans, 0);
    assert!(cpu.world_stats().stage_spans > 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|pixel| pixel.to_le_bytes() == [255; 4])
    );
}

#[test]
fn cached_boundary_keeps_attributes_through_partial_near_clipping() {
    let mut assets = Assets::load();
    let base = assets.register_image(1, 1, &[127, 151, 173, 255]).unwrap();
    let light = assets.register_image(1, 1, &[93, 101, 117, 255]).unwrap();
    let material = rgba_pair(&mut assets, base, false, false);
    let world = world_bound(
        &mut assets,
        2.0,
        1.0,
        SurfaceMaterial {
            material,
            lightmap: light,
            texture_scale: [1.0; 2],
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |geometry| {
            for (index, vertex) in geometry.vertices.iter_mut().enumerate() {
                vertex.vertex.position.0[0] = if index == 0 || index == 3 { 0.05 } else { 2.0 };
                vertex.vertex.texcoord = UNIT_UV[index];
                vertex.vertex.lightmap_coord = UNIT_UV[index];
            }
            geometry.surfaces[0].texture_coordinates = TextureCoordinates::Normalized;
            geometry.surfaces[0].texture_extents = [0; 2];
        },
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let frame = packet(&mut frontend, &[(world, 0)], rgba_view(), &assets);
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    assert!(cpu.world_stats().rgba_pixels > 0);
    assert_eq!(cpu.world_stats().stage_spans, 0);
    assert!(
        cpu.pixels()
            .iter()
            .filter(|pixel| pixel.to_le_bytes()[..3] != [0; 3])
            .all(|pixel| pixel.to_le_bytes() == [46, 60, 79, 255])
    );
}

#[test]
fn retail_collinear_face_is_skipped_but_nonempty_zero_extent_is_rejected() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, false);
    let image = texture(&mut assets, palette, 11, false);
    let material = material(&mut assets, image, false);
    let retail = world_mutated(
        &mut assets,
        2.0,
        1.0,
        material,
        None,
        GeometryPartition::Unpartitioned,
        |geometry| {
            let points = [
                [-288.0, 1456.0, -112.0],
                [-320.0, 1456.0, -112.0],
                [-352.0, 1456.0, -112.0],
                [-320.0, 1456.0, -112.0],
            ];
            for (vertex, point) in geometry.vertices.iter_mut().zip(points) {
                vertex.vertex.position = Vec3(point);
            }
            let surface = &mut geometry.surfaces[0];
            surface.plane = Some(Plane {
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: -112.0,
                axis: Some(qa_core::primitives::Axis::Z),
            });
            surface.bounds = Bounds {
                mins: Vec3([-352.0, 1456.0, -112.0]),
                maxs: Vec3([-288.0, 1456.0, -112.0]),
            };
            surface.texture_projection = [[1.0, 0.0, 0.0, 0.0], [0.0, -1.0, 0.0, 0.0]];
            surface.texture_minima = [-352, -1456];
            surface.texture_extents = [64, 0];
        },
    );
    let control = world_mutated(
        &mut assets,
        2.0,
        1.0,
        material,
        None,
        GeometryPartition::Unpartitioned,
        |geometry| {
            // A genuinely visible quad with a constant T projection has no
            // texture-space height, while its four world positions have area.
            geometry.surfaces[0].texture_projection[1] = [1.0, 0.0, 0.0, 0.0];
            geometry.surfaces[0].texture_extents[1] = 0;
        },
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let empty = packet(&mut frontend, &[(retail, 0)], view(palette), &assets);
    let stats = cpu.render(&empty, &assets);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.surfaces, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([0, 0, 0, 255]))
    );
    assert!(frontend.recycle(empty).is_ok());
    let visible = packet(&mut frontend, &[(control, 0)], view(palette), &assets);
    assert_eq!(cpu.render(&visible, &assets).rejected, 1);
}

#[test]
fn positional_bulge_keeps_raw_collinear_world_vertices_drawable() {
    use qa_render::shader::Deform;
    let mut assets = Assets::load();
    let white = assets.register_image(1, 1, &[255; 4]).unwrap();
    let material = assets
        .register_material(
            "bulged collinear surface",
            &[Stage {
                texture: StageTexture::Image(white),
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                deforms: [
                    Some(Deform::Bulge {
                        width: std::f32::consts::TAU,
                        height: 16.0,
                        speed: 0.0,
                    }),
                    None,
                    None,
                ],
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    let world = world_mutated(
        &mut assets,
        32.0,
        0.5,
        material,
        None,
        GeometryPartition::Unpartitioned,
        |geometry| {
            geometry.vertices.truncate(3);
            for (vertex, (position, s)) in geometry.vertices.iter_mut().zip([
                ([32.0, -16.0, 0.0], 0.0),
                ([32.0, 0.0, 0.0], 0.25),
                ([32.0, 16.0, 0.0], 1.0),
            ]) {
                vertex.vertex.position = Vec3(position);
                vertex.vertex.texcoord = [s, 0.0];
                vertex.vertex.normal = Vec3([0.0, 0.0, 1.0]);
                vertex.normal = vertex.vertex.normal;
            }
            geometry.indices = vec![0, 1, 2, 0, 1, 2];
            geometry.boundaries[0] = IndexRange { first: 3, count: 3 };
            let surface = &mut geometry.surfaces[0];
            surface.vertices.count = 3;
            surface.indices.count = 3;
            surface.bounds = Bounds {
                mins: Vec3([32.0, -16.0, 0.0]),
                maxs: Vec3([32.0, 16.0, 0.0]),
            };
            surface.texture_coordinates = TextureCoordinates::Normalized;
            surface.texture_extents = [16, 0];
        },
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
    assert_eq!(stats.surfaces, 1);
    assert_eq!(stats.triangles, 0);
    assert!(
        cpu.pixels()
            .iter()
            .any(|&pixel| pixel == u32::from_le_bytes([255; 4]))
    );
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

fn layered_material(
    assets: &mut Assets,
    palette: PaletteId,
    back: &[u8],
    front: &[u8],
) -> MaterialId {
    use qa_render::{
        assets::{Sky, TcGen},
        shader::{BlendFactor, StageBlend},
        sky::LayeredSphere,
    };
    let images = [
        assets
            .register_indexed_image(
                IndexedTexture::load_base(128, 128, back, None).unwrap(),
                palette,
            )
            .unwrap(),
        assets
            .register_indexed_image(
                IndexedTexture::load_base(128, 128, front, Some(0)).unwrap(),
                palette,
            )
            .unwrap(),
    ];
    let sphere = LayeredSphere::NATIVE;
    let stages = std::array::from_fn::<_, 2, _>(|layer| Stage {
        texture: StageTexture::Image(images[layer]),
        texgen: TcGen::LayeredSky {
            flatten_z: sphere.flatten_z,
            projected_scale: sphere.projected_scale,
            texture_size: sphere.texture_size,
            scroll_speed: sphere.scroll_speeds[layer],
        },
        blend: (layer == 1).then_some(StageBlend {
            source: BlendFactor::SourceAlpha,
            destination: BlendFactor::OneMinusSourceAlpha,
        }),
        depth_write: layer == 0,
        sampler: Sampler {
            mipmaps: false,
            ..Sampler::default()
        },
        ..Stage::default()
    });
    assets
        .register_material(
            "layered sky",
            &stages,
            MaterialSettings {
                cull: Cull::None,
                sort: 2.0,
                sky: Some(Sky::Layered { images, sphere }),
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}

#[test]
fn layered_sky_occludes_far_world_and_retains_unlit_masked_indices() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, false);
    let image = texture(&mut assets, palette, 20, false);
    let solid = material(&mut assets, image, false);
    let sky_material = layered_material(
        &mut assets,
        palette,
        &vec![11; 128 * 128],
        &vec![0; 128 * 128],
    );
    let sky = world(
        &mut assets,
        2.0,
        sky_material,
        Some(0),
        GeometryPartition::Unpartitioned,
    );
    let far = world(
        &mut assets,
        4.0,
        solid,
        None,
        GeometryPartition::Unpartitioned,
    );
    let near = world(
        &mut assets,
        1.0,
        solid,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for references in [[(sky, 9000), (far, 0)], [(far, 0), (sky, 9000)]] {
        let packet = packet(&mut frontend, &references, view(palette), &assets);
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.triangles, 0);
        assert!(
            cpu.pixels()
                .iter()
                .all(|&pixel| pixel == u32::from_le_bytes([11, 11, 11, 255]))
        );
        assert!(frontend.recycle(packet).is_ok());
    }
    let packet = packet(
        &mut frontend,
        &[(near, 9000), (sky, 0)],
        view(palette),
        &assets,
    );
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([20, 20, 20, 255]))
    );
}

#[test]
fn layered_sky_uses_full_seat_integer_center_and_ignores_fov() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, false);
    let back: Vec<_> = (0..128 * 128).map(|i| (i % 128) as u8).collect();
    let material = layered_material(&mut assets, palette, &back, &vec![0; 128 * 128]);
    let sky = world(
        &mut assets,
        2.0,
        material,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(16, 10, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for fov in [65.0, 110.0] {
        let refdef = Refdef {
            viewport: Viewport {
                x: 4,
                y: 4,
                width: 1,
                height: 1,
            },
            blend_viewport: Some(Viewport {
                x: 0,
                y: 0,
                width: 8,
                height: 10,
            }),
            fov: [fov; 2],
            ..view(palette)
        };
        let packet = packet(&mut frontend, &[(sky, 0)], refdef, &assets);
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        // Extent is the 1-pixel viewport, but center is the full seat (4,5).
        // Normalize (4096,0,24576), then truncate 378*x to texel62.
        assert_eq!(
            cpu.pixels()[4 * 16 + 4],
            u32::from_le_bytes([62, 62, 62, 255])
        );
        assert_eq!(
            cpu.pixels()[4 * 16 + 12],
            u32::from_le_bytes([0, 0, 0, 255])
        );
        assert!(frontend.recycle(packet).is_ok());
    }
}

#[test]
fn layered_sky_indices_bypass_world_colormap_lighting() {
    let mut assets = Assets::load();
    // Every supplied colormap row changes index11, including the fullbright
    // row. Sky indices are palette inputs rather than lit cache texels.
    let palette = palette(&mut assets, true);
    let material = layered_material(
        &mut assets,
        palette,
        &vec![11; 128 * 128],
        &vec![0; 128 * 128],
    );
    let sky = world(
        &mut assets,
        2.0,
        material,
        Some(0),
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let packet = packet(&mut frontend, &[(sky, 0)], view(palette), &assets);
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([11, 11, 11, 255]))
    );
}

#[test]
fn layered_sky_front_scroll_truncates_its_extra_shift_separately() {
    let mut assets = Assets::load();
    let palette = palette(&mut assets, false);
    let mut front = vec![0; 128 * 128];
    front[122] = 201;
    front[256 + 124] = 202;
    let material = layered_material(&mut assets, palette, &vec![11; 128 * 128], &front);
    let sky = world(
        &mut assets,
        2.0,
        material,
        None,
        GeometryPartition::Unpartitioned,
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for (time_ms, color) in [(100, 201), (125, 202)] {
        let packet = packet(
            &mut frontend,
            &[(sky, 0)],
            Refdef {
                viewport: Viewport {
                    x: 4,
                    y: 4,
                    width: 1,
                    height: 1,
                },
                time_ms,
                ..view(palette)
            },
            &assets,
        );
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert_eq!(
            cpu.pixels()[4 * 8 + 4],
            u32::from_le_bytes([color, color, color, 255])
        );
        assert!(frontend.recycle(packet).is_ok());
    }
}

fn cube_material(assets: &mut Assets, cpu_rotation: bool) -> MaterialId {
    use qa_render::{
        assets::{CubeSkyParams, Sky},
        sky::{CloudSphere, Rotation},
    };
    let images = std::array::from_fn(|face| {
        let indices: Vec<_> = (0..16).map(|pixel| 10 + face as u8 * 20 + pixel).collect();
        let indexed = IndexedTexture::load_base(4, 4, &indices, None).unwrap();
        // Distinct GL dimensions/content must not determine CPU sampling.
        assets
            .register_rgba_with_indexed(1, 1, &[200, 200, 200, 255], indexed)
            .unwrap()
    });
    assets
        .register_material(
            "cube sky",
            &[],
            MaterialSettings {
                cull: Cull::None,
                sort: 2.0,
                sky: Some(Sky::Cube {
                    outer_box: Some(images),
                    inner_box: None,
                    clouds: CloudSphere::native(512.0),
                    rotation: Some(Rotation {
                        axis: Vec3([0.0, 0.0, 1.0]),
                        degrees_per_second: 90.0,
                    }),
                    params: CubeSkyParams {
                        cpu_background: true,
                        cpu_rotation,
                        ..CubeSkyParams::default()
                    },
                }),
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}

#[test]
fn cube_background_uses_original_index_dimensions_and_typed_rotation_policy() {
    for cpu_rotation in [false, true] {
        let mut assets = Assets::load();
        let palette = palette(&mut assets, false);
        let sky_material = cube_material(&mut assets, cpu_rotation);
        let sky = world_with_extent(
            &mut assets,
            2.0,
            0.2,
            sky_material,
            None,
            GeometryPartition::Unpartitioned,
        );
        let solid_image = texture(&mut assets, palette, 200, false);
        let solid = material(&mut assets, solid_image, false);
        let foreground = world_with_extent(
            &mut assets,
            1.0,
            0.2,
            solid,
            None,
            GeometryPartition::Unpartitioned,
        );
        let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        for time_ms in [0, 1000] {
            let packet = packet(
                &mut frontend,
                &[(sky, 0)],
                Refdef {
                    time_ms,
                    ..view(palette)
                },
                &assets,
            );
            let stats = cpu.render(&packet, &assets);
            assert_eq!(stats.rejected, 0);
            assert_eq!(stats.triangles, 0);
            let face_base = if cpu_rotation && time_ms != 0 { 70 } else { 10 };
            for y in 0..8 {
                for x in 0..8 {
                    let color = face_base + (y / 2) as u8 * 4 + (x / 2) as u8;
                    assert_eq!(
                        cpu.pixels()[y * 8 + x],
                        u32::from_le_bytes([color, color, color, 255])
                    );
                }
            }
            assert!(frontend.recycle(packet).is_ok());
        }
        let packet = packet(
            &mut frontend,
            &[(sky, 0), (foreground, 9000)],
            view(palette),
            &assets,
        );
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert_eq!(
            cpu.pixels()[4 * 8 + 4],
            u32::from_le_bytes([200, 200, 200, 255])
        );
        assert_eq!(cpu.pixels()[0], u32::from_le_bytes([10, 10, 10, 255]));
    }
}
