use super::super::CpuBackend;
use super::*;
use crate::assets::{CubeSkyParams, Filter, Sampler};
use crate::scene::{Command, FrontEnd, Limits, Refdef, Viewport};
use crate::shader::{StageBlend, TexMod};
use crate::world::geometry::*;
use crate::world::{SurfaceMaterial, VisibleSurface};
use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_formats::bsp::IndexRange;
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};

fn fixture_world(
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

fn test_view() -> Refdef {
    Refdef {
        viewport: Viewport {
            x: 5,
            y: 3,
            width: 17,
            height: 13,
        },
        near: 0.25,
        far: 64.0,
        fov: [90.0; 2],
        cpu_presentation: CpuPresentation::Rgb,
        ..Refdef::default()
    }
}

fn packet(assets: &Assets, worlds: &[WorldId], refdef: Refdef) -> CommandList {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    for (index, &world) in worlds.iter().enumerate() {
        assert!(frame.add_world(
            world,
            &[VisibleSurface {
                surface: 0,
                depth_key: index as u32 * 9000
            }]
        ));
    }
    assert!(frame.render_scene(refdef, &[], assets));
    frame.finish()
}

fn row_buffers(cpu: &mut CpuBackend, rows: std::ops::Range<u32>) -> Buffers<'_> {
    let first = rows.start as usize * cpu.width as usize;
    let last = rows.end as usize * cpu.width as usize;
    Buffers {
        first_row: rows.start,
        frame_height: cpu.height,
        pixels: &mut cpu.pixels[first..last],
        inverse_depth: &mut cpu.inverse_depth[first..last],
        depth_ranks: &mut cpu.depth_ranks[first..last],
        indices: &mut cpu.indices[first..last],
        palettes: &mut cpu.palettes[first..last],
    }
}

/// This oracle changes only scanner row ownership. The common descriptor
/// graph is prepared once per view and reused by every independently owned rover.
fn windowed(
    cpu: &mut CpuBackend,
    list: &CommandList,
    assets: &Assets,
    band_count: u32,
) -> WorldStats {
    let mut world = cpu.world.take().unwrap();
    let budget = CpuLimits::default().cache_bytes / band_count as usize;
    let mut bands = (0..band_count)
        .map(|_| {
            WorldBand::load(
                cpu.width,
                cpu.height,
                Arc::clone(&world.prepare.catalog),
                budget,
                CpuLimits::default().max_spans,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let windows = (0..band_count)
        .map(|band| band * cpu.height / band_count..(band + 1) * cpu.height / band_count)
        .collect::<Vec<_>>();
    let mut stats = crate::BackendStats::default();
    for command in list.commands() {
        match *command {
            Command::Empty => {}
            Command::Clear(color) => {
                cpu.pixels.fill(u32::from_le_bytes(color));
                cpu.inverse_depth.fill(0.0);
                cpu.depth_ranks.fill(0);
                cpu.palettes.fill(u32::MAX);
                cpu.presentation = CpuPresentation::Rgb;
            }
            Command::Draw2d(draw) => cpu.draw_2d(draw, assets, &mut stats),
            Command::View(view) => {
                let camera = Camera::load(view.refdef, cpu.width, cpu.height).unwrap();
                cpu.presentation = camera.refdef.cpu_presentation;
                cpu.time_ms = camera.refdef.time_ms;
                cpu.clear_depth(camera.refdef.viewport, camera.refdef.far);
                assert!(world.prepare.prepare_view(
                    camera,
                    list,
                    view.scene,
                    assets,
                    &cpu.evaluator,
                    &mut stats
                ));
                for (band, rows) in bands.iter_mut().zip(&windows) {
                    band.render_opaque(
                        &world.prepare,
                        camera,
                        assets,
                        row_buffers(cpu, rows.clone()),
                        &mut stats,
                    );
                }
                for (rank, item) in list.draws(view.scene.draws).iter().enumerate() {
                    let mut handled = None;
                    for (band, rows) in bands.iter_mut().zip(&windows) {
                        let result = band.draw_item(
                            &world.prepare,
                            camera,
                            rank,
                            assets,
                            row_buffers(cpu, rows.clone()),
                            &mut stats,
                        );
                        if let Some(previous) = handled {
                            assert_eq!(previous, result);
                        }
                        handled = Some(result);
                    }
                    if handled == Some(true) {
                        continue;
                    }
                    match item.kind {
                        DrawKind::Entity => cpu.entity(
                            camera,
                            list.entity(item.index),
                            rank as u32,
                            assets,
                            &mut stats,
                        ),
                        DrawKind::Poly => cpu.poly(
                            camera,
                            list.poly(item.index),
                            rank as u32,
                            list,
                            assets,
                            &mut stats,
                        ),
                        DrawKind::Surface => {}
                    }
                }
                if camera.refdef.blend_phase == crate::scene::BlendPhase::AfterView {
                    cpu.view_blend(camera.refdef, false, assets);
                }
            }
        }
    }
    for command in list.commands() {
        if let Command::View(view) = *command
            && view.refdef.blend_phase == crate::scene::BlendPhase::FinalPalette
        {
            cpu.view_blend(view.refdef, true, assets);
        }
    }
    assert_eq!(stats.rejected, 0);
    let mut counters = world.prepare.stats;
    for band in &bands {
        counters = merge_stats(counters, band.stats);
    }
    cpu.world = Some(world);
    counters
}

fn exact_rows(assets: &Assets, list: &CommandList) {
    let mut serial = CpuBackend::load_with_assets(29, 19, assets).unwrap();
    assert_eq!(serial.render(list, assets).rejected, 0);
    let reference = serial.world_stats();
    for count in [1, 2, 4, 8, 19] {
        let mut cpu = CpuBackend::load_with_assets(29, 19, assets).unwrap();
        let stats = windowed(&mut cpu, list, assets, count);
        assert_eq!(cpu.pixels, serial.pixels, "pixels for {count} row windows");
        assert_eq!(
            cpu.inverse_depth
                .iter()
                .map(|f| f.to_bits())
                .collect::<Vec<_>>(),
            serial
                .inverse_depth
                .iter()
                .map(|f| f.to_bits())
                .collect::<Vec<_>>(),
            "depth for {count}"
        );
        assert_eq!(cpu.depth_ranks, serial.depth_ranks, "ranks for {count}");
        assert_eq!(cpu.indices, serial.indices, "indices for {count}");
        assert_eq!(cpu.palettes, serial.palettes, "palettes for {count}");
        assert_eq!(stats.polygons, reference.polygons);
        assert_eq!(stats.patch_polygons, reference.patch_polygons);
        assert_eq!(stats.pixels, reference.pixels);
    }
}

fn stages(assets: &mut Assets, stages: &[Stage], settings: MaterialSettings) -> MaterialId {
    assets
        .register_material("prepared fixture", stages, settings)
        .unwrap()
}

#[test]
fn offset_row_windows_keep_three_stage_depth_blend_and_rank_bits() {
    let mut assets = Assets::load();
    let image = assets
        .register_image(
            2,
            2,
            &[
                17, 31, 47, 255, 67, 83, 101, 255, 127, 149, 167, 255, 181, 199, 223, 255,
            ],
        )
        .unwrap();
    let stages_ = [
        Stage {
            texture: StageTexture::Image(image),
            rgb_gen: RgbGen::Vertex,
            tcmods: [
                Some(crate::assets::TcMod::Script(TexMod::Turbulent {
                    base: 0.0,
                    amplitude: 0.125,
                    phase: 0.25,
                    frequency: 0.5,
                })),
                None,
                None,
                None,
            ],
            ..Stage::default()
        },
        Stage {
            texture: StageTexture::Image(image),
            depth_func: DepthFunc::Equal,
            depth_write: false,
            blend: Some(StageBlend {
                source: BlendFactor::DestinationColor,
                destination: BlendFactor::Zero,
            }),
            ..Stage::default()
        },
        Stage {
            texture: StageTexture::Image(image),
            alpha_gen: AlphaGen::Const(0.5),
            depth_write: false,
            blend: Some(StageBlend {
                source: BlendFactor::SourceAlpha,
                destination: BlendFactor::OneMinusSourceAlpha,
            }),
            ..Stage::default()
        },
    ];
    let settings = MaterialSettings {
        cull: Cull::None,
        deforms: [
            Some(crate::shader::Deform::Bulge {
                width: 2.5,
                height: 0.125,
                speed: 0.375,
            }),
            None,
            None,
        ],
        ..MaterialSettings::default()
    };
    let material = stages(&mut assets, &stages_, settings);
    let world = fixture_world(
        &mut assets,
        2.0,
        1.5,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |g| {
            for (i, v) in g.vertices.iter_mut().enumerate() {
                v.vertex.texcoord = [i as f32 * 0.375 - 0.25, i as f32 * 0.125];
                v.vertex.color = [37 + i as u8 * 41, 219 - i as u8 * 29, 181, 255];
            }
        },
    );
    let view = Refdef {
        time_ms: 731,
        ..test_view()
    };
    exact_rows(&assets, &packet(&assets, &[world], view));
}

#[test]
fn cached_rgba_and_changed_preparation_views_keep_row_output() {
    let mut assets = Assets::load();
    let image = assets
        .register_image(
            2,
            2,
            &[
                17, 31, 47, 255, 67, 83, 101, 255, 127, 149, 167, 255, 181, 199, 223, 255,
            ],
        )
        .unwrap();
    let light = assets
        .register_image(
            2,
            2,
            &[
                51, 67, 83, 255, 101, 127, 149, 255, 167, 181, 199, 255, 223, 239, 251, 255,
            ],
        )
        .unwrap();
    let material = stages(
        &mut assets,
        &[
            Stage {
                texture: StageTexture::Image(image),
                ..Stage::default()
            },
            Stage {
                texture: StageTexture::Lightmap,
                texgen: TexCoordGen::Lightmap,
                depth_write: false,
                depth_func: DepthFunc::Equal,
                blend: Some(StageBlend {
                    source: BlendFactor::DestinationColor,
                    destination: BlendFactor::Zero,
                }),
                ..Stage::default()
            },
        ],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let world = fixture_world(
        &mut assets,
        2.0,
        1.0,
        SurfaceMaterial {
            material,
            lightmap: light,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |g| {
            g.surfaces[0].texture_coordinates = TextureCoordinates::Normalized;
            for (i, v) in g.vertices.iter_mut().enumerate() {
                let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i];
                v.vertex.texcoord = uv;
                v.vertex.lightmap_coord = uv;
            }
        },
    );
    let frame = packet(&assets, &[world], test_view());
    exact_rows(&assets, &frame);
    let mut old = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let mut old_rows = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assets
        .prepare_image_with_rgba(
            image,
            &[
                211, 13, 29, 255, 199, 31, 47, 255, 181, 53, 71, 255, 167, 83, 101, 255,
            ],
            crate::assets::upload::UploadParams::default(),
        )
        .unwrap();
    assert_eq!(old.render(&frame, &assets).rejected, 0);
    windowed(&mut old_rows, &frame, &assets, 19);
    assert_eq!(old_rows.pixels, old.pixels);
    assert_eq!(old_rows.depth_ranks, old.depth_ranks);
}

#[test]
fn simultaneous_sky_materials_retain_both_box_and_cloud_ranges() {
    let mut assets = Assets::load();
    let red = assets.register_image(1, 1, &[30, 2, 1, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[1, 3, 40, 255]).unwrap();
    let mut worlds = Vec::new();
    for image in [red, blue] {
        let material = stages(
            &mut assets,
            &[
                Stage {
                    texture: StageTexture::Image(image),
                    texgen: TexCoordGen::CloudSky {
                        radius: 4096.0,
                        height: 384.0,
                    },
                    ..Stage::default()
                },
                Stage {
                    texture: StageTexture::Image(image),
                    texgen: TexCoordGen::CloudSky {
                        radius: 4096.0,
                        height: 384.0,
                    },
                    blend: Some(StageBlend {
                        source: BlendFactor::One,
                        destination: BlendFactor::One,
                    }),
                    depth_write: false,
                    ..Stage::default()
                },
            ],
            MaterialSettings {
                sort: 2.0,
                cull: Cull::None,
                sky: Some(Sky::Cube {
                    outer_box: Some([image; 6]),
                    inner_box: None,
                    clouds: crate::sky::CloudSphere::native(384.0),
                    rotation: None,
                    params: CubeSkyParams::default(),
                }),
                ..MaterialSettings::default()
            },
        );
        worlds.push(fixture_world(
            &mut assets,
            2.0,
            1.5,
            SurfaceMaterial {
                material,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::Unpartitioned,
            |_| {},
        ));
    }
    let frame = packet(&assets, &worlds, test_view());
    exact_rows(&assets, &frame);
    let mut raster = WorldRaster::load(
        29,
        19,
        &assets,
        CpuLimits::default(),
        &StageEvaluator::load(),
    )
    .unwrap();
    let Command::View(view) = frame
        .commands()
        .iter()
        .find(|c| matches!(c, Command::View(_)))
        .copied()
        .unwrap()
    else {
        unreachable!()
    };
    let camera = Camera::load(view.refdef, 29, 19).unwrap();
    assert!(raster.prepare.prepare_view(
        camera,
        &frame,
        view.scene,
        &assets,
        &StageEvaluator::load(),
        &mut crate::BackendStats::default()
    ));
    let sky_draws = raster.prepare.draws[..raster.prepare.draw_count]
        .iter()
        .filter_map(|draw| {
            if let PreparedDraw::Sky { boxes, clouds } = draw {
                Some((*boxes, *clouds))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(sky_draws.len(), 2);
    for (boxes, clouds) in sky_draws {
        assert!(boxes[0] < boxes[1]);
        assert!(clouds[0] < clouds[1]);
    }
}

fn indexed_palette(assets: &mut Assets) -> crate::assets::PaletteId {
    let colors = (0..=255u8).flat_map(|c| [c, c, c]).collect::<Vec<_>>();
    let colormap = (0..64).flat_map(|_| 0..=255u8).collect::<Vec<_>>();
    assets
        .register_palette(
            crate::surface_cache::PaletteLighting::load(&colors, &colormap, None, 224).unwrap(),
        )
        .unwrap()
}

#[test]
fn native_cache_and_full_seat_layered_sky_keep_one_row_bits() {
    use crate::surface_cache::{IndexedLighting, IndexedTexture};
    let mut assets = Assets::load();
    let palette = indexed_palette(&mut assets);
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
        (0..(16 >> mip) * (16 >> mip))
            .map(|i| (17 + i % 211) as u8)
            .collect()
    });
    let image = assets
        .register_indexed_image(
            IndexedTexture::load(16, 16, std::array::from_fn(|m| mips[m].as_slice()), false)
                .unwrap(),
            palette,
        )
        .unwrap();
    let solid = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
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
    );
    let wall = fixture_world(
        &mut assets,
        2.0,
        0.5,
        SurfaceMaterial {
            material: solid,
            ..SurfaceMaterial::default()
        },
        Some(137),
        GeometryPartition::Unpartitioned,
        |_| {},
    );
    let layers = [
        (0..128 * 128)
            .map(|i| (32 + i % 191) as u8)
            .collect::<Vec<_>>(),
        (0..128 * 128)
            .map(|i| if i % 5 == 0 { 0 } else { (63 + i % 157) as u8 })
            .collect::<Vec<_>>(),
    ];
    let images = std::array::from_fn(|layer| {
        assets
            .register_indexed_image(
                IndexedTexture::load_base(128, 128, &layers[layer], (layer == 1).then_some(0))
                    .unwrap(),
                palette,
            )
            .unwrap()
    });
    let sphere = crate::sky::LayeredSphere::NATIVE;
    let sky_stages = std::array::from_fn::<_, 2, _>(|layer| Stage {
        texture: StageTexture::Image(images[layer]),
        texgen: TexCoordGen::LayeredSky {
            flatten_z: sphere.flatten_z,
            projected_scale: sphere.projected_scale,
            texture_size: sphere.texture_size,
            scroll_speed: sphere.scroll_speeds[layer],
        },
        sampler: Sampler {
            mipmaps: false,
            ..Sampler::default()
        },
        depth_write: layer == 0,
        blend: (layer == 1).then_some(StageBlend {
            source: BlendFactor::SourceAlpha,
            destination: BlendFactor::OneMinusSourceAlpha,
        }),
        ..Stage::default()
    });
    let sky_material = stages(
        &mut assets,
        &sky_stages,
        MaterialSettings {
            cull: Cull::None,
            sort: 2.0,
            sky: Some(Sky::Layered { images, sphere }),
            ..MaterialSettings::default()
        },
    );
    let sky = fixture_world(
        &mut assets,
        8.0,
        1.5,
        SurfaceMaterial {
            material: sky_material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |_| {},
    );
    let view = Refdef {
        time_ms: 7123,
        blend_viewport: Some(Viewport {
            x: 2,
            y: 1,
            width: 23,
            height: 17,
        }),
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: false,
        },
        ..test_view()
    };
    exact_rows(&assets, &packet(&assets, &[wall, sky], view));
    let raster = WorldRaster::load(
        29,
        19,
        &assets,
        CpuLimits::default(),
        &StageEvaluator::load(),
    )
    .unwrap();
    let required = raster.prepare.catalog.mandatory_cache_bytes;
    assert!(required > 0);
    for bands in [1, 2, 4, 8] {
        let total = required * bands;
        assert_eq!(total / bands, required);
        assert!(
            WorldBand::load(
                29,
                19,
                Arc::clone(&raster.prepare.catalog),
                total / bands,
                4096
            )
            .is_ok()
        );
        assert!(
            WorldBand::load(
                29,
                19,
                Arc::clone(&raster.prepare.catalog),
                total / bands - 1,
                4096
            )
            .is_err()
        );
    }
}

#[test]
fn native_cube_background_prepares_planes_once_and_preserves_row_projection() {
    use crate::surface_cache::{IndexedLighting, IndexedTexture};
    let mut assets = Assets::load();
    let palette = indexed_palette(&mut assets);
    let images = std::array::from_fn(|face| {
        let indices = (0..64)
            .map(|pixel| (11 + face * 29 + pixel % 23) as u8)
            .collect::<Vec<_>>();
        assets
            .register_indexed_image(
                IndexedTexture::load_base(8, 8, &indices, None).unwrap(),
                palette,
            )
            .unwrap()
    });
    let material = stages(
        &mut assets,
        &[],
        MaterialSettings {
            cull: Cull::None,
            sky: Some(Sky::Cube {
                outer_box: Some(images),
                inner_box: None,
                clouds: crate::sky::CloudSphere::native(384.0),
                rotation: Some(crate::sky::Rotation {
                    axis: Vec3([0.0, 0.0, 1.0]),
                    degrees_per_second: 37.0,
                }),
                params: CubeSkyParams {
                    cpu_background: true,
                    cpu_rotation: false,
                    ..CubeSkyParams::default()
                },
            }),
            ..MaterialSettings::default()
        },
    );
    let world = fixture_world(
        &mut assets,
        2.0,
        1.5,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |_| {},
    );
    let view = Refdef {
        time_ms: 9017,
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::NativeRgb,
            ambient: 0,
            fullbright: false,
        },
        ..test_view()
    };
    exact_rows(&assets, &packet(&assets, &[world], view));
}
