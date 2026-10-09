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
    let bounds = geometry.surfaces[0].bounds;
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

/// This unbinned oracle retains a barrier at each original draw rank. The
/// descriptor graph is prepared once, then each owned rover consumes that rank.
fn windowed(
    cpu: &mut CpuBackend,
    list: &CommandList,
    assets: &Assets,
    band_count: u32,
) -> (WorldStats, crate::BackendStats) {
    let mut world = cpu.world.take().unwrap();
    let budget = (world.config.total_cache_budget_bytes / 8 / band_count as usize) * 8;
    let mut bands = (0..band_count)
        .map(|_| {
            WorldBand::load(
                cpu.width,
                cpu.height,
                Arc::clone(&world.prepare.catalog),
                budget,
                CpuLimits::default().max_spans,
                RasterSelection::Unbinned,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let windows = (0..band_count)
        .map(|band| band * cpu.height / band_count..(band + 1) * cpu.height / band_count)
        .collect::<Vec<_>>();
    let mut stats = crate::BackendStats {
        rejected: list.rejected.min(u32::MAX as u64) as u32,
        ..crate::BackendStats::default()
    };
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
                stats.views += 1;
                stats.pending_lights += view.scene.lights.count;
                cpu.presentation = camera.refdef.cpu_presentation;
                cpu.time_ms = camera.refdef.time_ms;
                cpu.clear_depth(camera.refdef.viewport, camera.refdef.far);
                assert!(world.prepare.prepare_view(
                    &camera,
                    list,
                    view.scene,
                    assets,
                    &cpu.evaluator,
                    &mut stats
                ));
                for (band, rows) in bands.iter_mut().zip(&windows) {
                    band.render_opaque(
                        &world.prepare,
                        &camera,
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
                            &camera,
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
                            &camera,
                            list.entity(item.index),
                            rank as u32,
                            assets,
                            &mut stats,
                        ),
                        DrawKind::Poly => cpu.poly(
                            &camera,
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
                    cpu.view_blend(&camera.refdef, false, assets);
                }
            }
        }
    }
    for command in list.commands() {
        if let Command::View(view) = *command
            && view.refdef.blend_phase == crate::scene::BlendPhase::FinalPalette
        {
            cpu.view_blend(&view.refdef, true, assets);
        }
    }
    assert_eq!(stats.rejected, 0);
    let mut counters = world.prepare.geometry.stats;
    let mut cache = CacheStats::default();
    for band in &bands {
        counters = merge_stats(counters, band.stats);
        let current = band.cache.stats();
        cache.hits += current.hits;
        cache.fills += current.fills;
        cache.evictions += current.evictions;
        cache.rejected += current.rejected;
        cache.nonresident_fills += current.nonresident_fills;
        cache.state_fills += current.state_fills;
        cache.fill_bytes += current.fill_bytes;
        cache.evicted_bytes += current.evicted_bytes;
        cache.resident_bytes += current.resident_bytes;
        cache.peak_resident_bytes += current.peak_resident_bytes;
    }
    counters.cache = cache;
    cpu.world = Some(world);
    (counters, stats)
}

fn assert_band_counters(aggregate: WorldStats, bands: &[WorldStats]) {
    assert!(
        bands
            .iter()
            .all(|s| s.polygons == 0 && s.patch_polygons == 0)
    );
    macro_rules! raster_totals {
        ($($field:ident),+ $(,)?) => {
            $(assert_eq!(
                bands.iter().map(|s| s.$field).sum::<u64>(),
                aggregate.$field,
                stringify!($field)
            );)+
        };
    }
    raster_totals!(
        spans,
        pixels,
        sky_spans,
        sky_pixels,
        stage_spans,
        stage_pixels,
        curve_spans,
        curve_pixels,
        multistage_spans,
        multistage_pixels,
        indexed_spans,
        indexed_pixels,
        rgba_spans,
        rgba_pixels,
        rgba_hits,
        rgba_fills,
        rgba_evictions,
        rgba_rejected,
        rgba_minified_spans,
        factor_spans,
        factor_pixels,
        factor_hits,
        factor_fills,
        factor_evictions,
        factor_rejected,
        factor_fallback_spans,
        factor_minified_spans,
        factor_curve_spans,
        factor_curve_pixels,
        rejected,
    );
    assert_eq!(
        bands.iter().map(|s| s.cache.hits).sum::<u64>(),
        aggregate.cache.hits
    );
    assert_eq!(
        bands.iter().map(|s| s.cache.fills).sum::<u64>(),
        aggregate.cache.fills
    );
    assert_eq!(
        bands.iter().map(|s| s.cache.evictions).sum::<u64>(),
        aggregate.cache.evictions
    );
    assert_eq!(
        bands.iter().map(|s| s.cache.rejected).sum::<u64>(),
        aggregate.cache.rejected
    );
}

fn exact_rows(assets: &Assets, list: &CommandList) -> WorldStats {
    let mut serial = CpuBackend::load_with_assets(29, 19, assets).unwrap();
    let reference_backend = serial.render(list, assets);
    assert_eq!(reference_backend.rejected, 0);
    let reference = serial.world_stats();
    let mut unbinned = [WorldStats::default(); MAX_BANDS];
    for count in [1, 2, 4, 8, 19] {
        let mut cpu = CpuBackend::load_with_assets(29, 19, assets).unwrap();
        let (stats, backend) = windowed(&mut cpu, list, assets, count);
        assert_eq!(backend, reference_backend);
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
        if count <= MAX_BANDS as u32 {
            unbinned[count as usize - 1] = stats;
        }
    }
    for bands in [
        RasterBands::One,
        RasterBands::Two,
        RasterBands::Four,
        RasterBands::Eight,
    ] {
        let mut cpu = CpuBackend::load_with_limits(
            29,
            19,
            assets,
            CpuLimits {
                bands,
                ..CpuLimits::default()
            },
        )
        .unwrap();
        // Exercise parallel semantics even for these deliberately small fixtures.
        cpu.world
            .as_mut()
            .unwrap()
            .config
            .prepare_minimum_primitives_per_job = 0;
        let mut callbacks = 0usize;
        let result: Result<_, std::convert::Infallible> =
            cpu.render_with_dispatch(list, assets, |jobs| {
                callbacks += 1;
                assert_eq!(jobs.len(), bands.count());
                for job in jobs.iter_mut().rev() {
                    super::run_cpu_job(job);
                }
                Ok(())
            });
        assert_eq!(result.unwrap(), reference_backend);
        assert!(callbacks > 0);
        assert_eq!(cpu.pixels, serial.pixels);
        assert_eq!(
            cpu.inverse_depth
                .iter()
                .map(|f| f.to_bits())
                .collect::<Vec<_>>(),
            serial
                .inverse_depth
                .iter()
                .map(|f| f.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(cpu.depth_ranks, serial.depth_ranks);
        assert_eq!(cpu.indices, serial.indices);
        assert_eq!(cpu.palettes, serial.palettes);
        let aggregate = cpu.world_stats();
        assert_eq!(aggregate, unbinned[bands.count() - 1]);
        assert_eq!(aggregate.polygons, reference.polygons);
        assert_eq!(aggregate.patch_polygons, reference.patch_polygons);
        assert_eq!(aggregate.pixels, reference.pixels);
        let mut per_band = [WorldStats::default(); MAX_BANDS];
        assert_eq!(cpu.band_stats(&mut per_band), bands.count());
        assert_band_counters(aggregate, &per_band[..bands.count()]);
        assert!(
            per_band[bands.count()..]
                .iter()
                .all(|s| *s == WorldStats::default())
        );
        let config = cpu.raster_config();
        assert_eq!(config.bands, bands);
        assert_eq!(config.total_cache_budget_bytes, 32 * 1024 * 1024);
        assert_eq!(
            config.mip_layout_metadata_bytes,
            serial.raster_config().mip_layout_metadata_bytes
        );
        assert_eq!(
            config.allocated_cache_bytes,
            config.per_band_cache_bytes * bands.count()
        );
        assert!(config.per_band_cache_bytes >= config.mandatory_cache_bytes);
        assert_eq!(
            config.bin_index_capacity_bytes,
            cpu.world
                .as_ref()
                .unwrap()
                .prepare
                .catalog
                .primitive_capacity
                * bands.count()
                * size_of::<u32>()
        );
    }
    reference
}

fn stages(assets: &mut Assets, stages: &[Stage], settings: MaterialSettings) -> MaterialId {
    assets
        .register_material("prepared fixture", stages, settings)
        .unwrap()
}

#[test]
fn offset_row_windows_keep_three_stage_depth_blend_and_rank_bits() {
    let mut assets = Assets::load().unwrap();
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
fn curved_patch_grid_keeps_triangle_coverage_and_curve_counters() {
    let mut assets = Assets::load().unwrap();
    let image = assets
        .register_image(
            2,
            2,
            &[
                37, 53, 71, 255, 83, 101, 127, 255, 149, 167, 181, 255, 199, 223, 239, 255,
            ],
        )
        .unwrap();
    let material = stages(
        &mut assets,
        &[
            Stage {
                texture: StageTexture::Image(image),
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
                alpha_gen: AlphaGen::Const(0.5),
                blend: Some(StageBlend {
                    source: BlendFactor::SourceAlpha,
                    destination: BlendFactor::OneMinusSourceAlpha,
                }),
                depth_write: false,
                depth_func: DepthFunc::Equal,
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
        1.5,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |g| {
            // A quadratic 3x3 control patch with the center 0.5 units deeper.
            // Native PutPointsOnCurve at this no-subdivision tolerance leaves
            // a 3x3 grid, whose center is 0.125 units deeper (tr_curve.c).
            g.vertices.clear();
            for row in 0..3 {
                for column in 0..3 {
                    let u = column as f32 * 0.5;
                    let v = row as f32 * 0.5;
                    g.vertices.push(WorldVertex {
                        vertex: Vertex {
                            position: Vec3([
                                2.0 + if row == 1 && column == 1 { 0.5 } else { 0.0 },
                                3.0 - 6.0 * u,
                                3.0 - 6.0 * v,
                            ]),
                            texcoord: [u, v],
                            lightmap_coord: [u, v],
                            ..Vertex::default()
                        },
                        normal: Vec3([-1.0, 0.0, 0.0]),
                    });
                }
            }
            for row in 0..3 {
                for column in 0..3 {
                    let u = column as f32 * 0.5;
                    let v = row as f32 * 0.5;
                    let bu = 2.0 * u * (1.0 - u);
                    let bv = 2.0 * v * (1.0 - v);
                    let mut vertex = g.vertices[row * 3 + column];
                    vertex.vertex.position.0[0] = 2.0 + 0.5 * bu * bv;
                    g.vertices.push(vertex);
                }
            }
            g.indices.clear();
            g.boundaries.clear();
            for row in 0..2u32 {
                for column in 0..2u32 {
                    let a = 9 + row * 3 + column;
                    let b = a + 3;
                    for triangle in [[a, b, a + 1], [a + 1, b, b + 1]] {
                        g.boundaries.push(IndexRange {
                            first: g.indices.len() as u32,
                            count: 3,
                        });
                        g.indices.extend_from_slice(&triangle);
                    }
                }
            }
            let surface = &mut g.surfaces[0];
            surface.kind = GeometryKind::Patch;
            surface.vertices = IndexRange { first: 9, count: 9 };
            surface.indices = IndexRange {
                first: 0,
                count: 24,
            };
            surface.boundaries = IndexRange { first: 0, count: 8 };
            surface.plane = None;
            surface.bounds.maxs.0[0] = 2.125;
            surface.texture_coordinates = TextureCoordinates::Normalized;
            surface.patch = Some(PatchGrid {
                control_vertices: IndexRange { first: 0, count: 9 },
                control_dimensions: [3, 3],
                dimensions: [3, 3],
                width_lod_error: vec![0.0, 4.0, 0.0].into_boxed_slice(),
                height_lod_error: vec![0.0, 4.0, 0.0].into_boxed_slice(),
                lod_origin: Vec3([2.0, 0.0, 0.0]),
                lod_radius: 18.0f32.sqrt(),
            });
        },
    );
    let geometry = assets.world(world).unwrap().geometry();
    let surface = &geometry.surfaces[0];
    let grid = surface.patch.as_ref().unwrap();
    assert_eq!(grid.control_dimensions, [3, 3]);
    assert_eq!(grid.dimensions, [3, 3]);
    assert_eq!(surface.boundaries.count, 8);
    assert_eq!(geometry.vertices[4].vertex.position.0[0], 2.5);
    assert_eq!(geometry.vertices[13].vertex.position.0[0], 2.125);
    let counters = exact_rows(
        &assets,
        &packet(
            &assets,
            &[world],
            Refdef {
                time_ms: 731,
                ..test_view()
            },
        ),
    );
    assert_eq!(counters.patch_polygons, 8);
    assert_eq!(counters.patch_polygons, counters.polygons);
    assert!(counters.curve_spans > 0);
    assert!(counters.curve_pixels > 0);
    assert_eq!(counters.curve_spans, counters.stage_spans);
    assert_eq!(counters.curve_pixels, counters.stage_pixels);
}

#[test]
fn cached_rgba_and_changed_preparation_views_keep_row_output() {
    let mut assets = Assets::load().unwrap();
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
    let create_world = |assets: &mut Assets| {
        fixture_world(
            assets,
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
        )
    };
    let worlds: [WorldId; 8] = std::array::from_fn(|_| create_world(&mut assets));
    let frame = packet(&assets, &worlds, test_view());
    let admission = CpuBackend::load_with_assets(29, 19, &assets)
        .unwrap()
        .raster_config();
    let required = admission.mandatory_cache_bytes;
    assert!(required > 0);
    for bands in [RasterBands::Two, RasterBands::Four, RasterBands::Eight] {
        assert!(
            CpuBackend::load_with_limits(
                29,
                19,
                &assets,
                CpuLimits {
                    bands,
                    cache_bytes: required * bands.count() - 1,
                    ..CpuLimits::default()
                }
            )
            .is_err()
        );
        let accepted = CpuBackend::load_with_limits(
            29,
            19,
            &assets,
            CpuLimits {
                bands,
                cache_bytes: required * bands.count(),
                ..CpuLimits::default()
            },
        )
        .unwrap();
        assert_eq!(accepted.raster_config().mandatory_cache_bytes, required);
        assert_eq!(accepted.raster_config().per_band_cache_bytes, required);
    }

    let counters = exact_rows(&assets, &frame);
    assert!(counters.rgba_spans > 0);
    assert!(counters.rgba_pixels > 0);
    assert_eq!(counters.rgba_hits, 0);
    assert!(counters.rgba_fills > 0);

    // A view that cannot fit a private chunk uses the same bounded serial
    // preparer; losing the chunk must not lose surfaces or static stage counts.
    let mut bounded = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            bands: RasterBands::Eight,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    bounded
        .world
        .as_mut()
        .unwrap()
        .config
        .prepare_minimum_primitives_per_job = 0;
    bounded.world.as_mut().unwrap().lanes[0].coverage =
        vec![ProjectedVertex::default(); 3].into_boxed_slice();
    let fallback: Result<_, std::convert::Infallible> =
        bounded.render_with_dispatch(&frame, &assets, |jobs| {
            assert_eq!(jobs[0].kind(), JobKind::Raster);
            for job in jobs {
                super::run_cpu_job(job);
            }
            Ok(())
        });
    let mut serial = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(fallback.unwrap(), serial.render(&frame, &assets));
    assert_eq!(bounded.pixels, serial.pixels);
    assert_eq!(bounded.inverse_depth, serial.inverse_depth);
    assert_eq!(bounded.depth_ranks, serial.depth_ranks);
    let mut warm = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(warm.render(&frame, &assets).rejected, 0);
    let first_pixels = warm.pixels.clone();
    let first_depth = warm.inverse_depth.clone();
    let first_ranks = warm.depth_ranks.clone();
    assert_eq!(warm.render(&frame, &assets).rejected, 0);
    assert!(warm.world_stats().rgba_hits > 0);
    assert_eq!(warm.world_stats().rgba_fills, 0);
    assert_eq!(warm.pixels, first_pixels);
    assert_eq!(warm.inverse_depth, first_depth);
    assert_eq!(warm.depth_ranks, first_ranks);
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
fn product_color_memo_keeps_camera_motion_and_identity_refresh_output() {
    for (red, can_reject_rounding) in [([64, 96, 128, 96], false), ([0, 1, 2, 1], true)] {
        let mut assets = Assets::load().unwrap();
        let image = assets
            .register_image(
                2,
                2,
                &[
                    31, 47, 61, 255, 79, 97, 113, 255, 131, 151, 173, 255, 191, 211, 233, 255,
                ],
            )
            .unwrap();
        let light = assets.register_image(1, 1, &[173, 191, 211, 255]).unwrap();
        let material = stages(
            &mut assets,
            &[
                Stage {
                    texture: StageTexture::Image(image),
                    rgb_gen: RgbGen::Vertex,
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
                let boundary = g.boundaries[0];
                g.indices[boundary.indices()].rotate_left(2);
                for (index, vertex) in g.vertices.iter_mut().enumerate() {
                    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][index];
                    vertex.vertex.texcoord = [uv[0] - 0.375, uv[1] + 0.25];
                    vertex.vertex.lightmap_coord = [0.5; 2];
                    vertex.vertex.color = [red[index], 255, 255, 255];
                }
            },
        );
        let other = fixture_world(
            &mut assets,
            3.0,
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
                for (index, vertex) in g.vertices.iter_mut().enumerate() {
                    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][index];
                    vertex.vertex.texcoord = [uv[0] + 0.625, uv[1] - 0.375];
                    vertex.vertex.lightmap_coord = [0.5; 2];
                    vertex.vertex.color = [
                        [40, 80, 120, 255],
                        [80, 120, 160, 255],
                        [120, 160, 200, 255],
                        [80, 120, 160, 255],
                    ][index];
                }
            },
        );
        let mut cpu = CpuBackend::load_with_limits(
            29,
            19,
            &assets,
            CpuLimits {
                bands: RasterBands::Four,
                ..CpuLimits::default()
            },
        )
        .unwrap();
        for (frame_index, (identity_light, refreshes)) in [
            (1.0, 1),
            (1.0, 1),
            (1.0, 1),
            (0.5, 2),
            (0.5, 2),
            (0.25, 3),
            (0.25, 3),
            (1.0, 4),
        ]
        .into_iter()
        .enumerate()
        {
            let motion = frame_index.saturating_sub(1) as f32;
            let view = Refdef {
                identity_light,
                origin: Vec3([0.125 * motion, 0.03125 * motion, -0.03125 * motion]),
                ..test_view()
            };
            let frame = packet(&assets, &[world], view);
            assert_eq!(cpu.render(&frame, &assets).rejected, 0);
            let mut fresh = CpuBackend::load_with_limits(
                29,
                19,
                &assets,
                CpuLimits {
                    bands: RasterBands::Four,
                    ..CpuLimits::default()
                },
            )
            .unwrap();
            assert_eq!(fresh.render(&frame, &assets).rejected, 0);
            assert_eq!(cpu.pixels, fresh.pixels);
            assert_eq!(
                cpu.inverse_depth
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                fresh
                    .inverse_depth
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
            assert_eq!(cpu.depth_ranks, fresh.depth_ranks);
            assert_eq!(cpu.indices, fresh.indices);
            assert_eq!(cpu.palettes, fresh.palettes);
            let counters = cpu.world_stats();
            let valid = !can_reject_rounding || identity_light != 0.5;
            if valid {
                assert!(counters.rgba_pixels > 0);
                assert_eq!(counters.stage_spans, 0);
                if frame_index == 1 {
                    assert_eq!(counters.rgba_fills, 0);
                    assert!(counters.rgba_hits > 0);
                }
            } else {
                assert_eq!(counters.rgba_spans, 0);
                assert!(counters.stage_pixels > 0);
            }
            let prepared = &mut cpu.world.as_mut().unwrap().prepare;
            assert_eq!(prepared.rgba_colors.len(), prepared.catalog.rgba.len());
            let recipe_id = prepared.catalog.boundary_offsets[world.0 as usize];
            assert_eq!(prepared.rgba_colors[recipe_id].refreshes(), refreshes);
            if valid {
                let geometry = assets.world(world).unwrap().geometry();
                let boundary = geometry.boundaries[0];
                let recipe = prepared.catalog.rgba[recipe_id].as_ref().unwrap();
                let expected = geometry.indices[boundary.indices()]
                    .iter()
                    .map(|&index| {
                        recipe
                            .coordinate(geometry.vertices[index as usize].vertex.position)
                            .unwrap()
                            .map(f32::to_bits)
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    prepared
                        .geometry
                        .clip
                        .sources(boundary.count as usize)
                        .unwrap()
                        .iter()
                        .map(|vertex| vertex.texcoord.map(f32::to_bits))
                        .collect::<Vec<_>>(),
                    expected
                );
            }
        }
        let first_id =
            cpu.world.as_ref().unwrap().prepare.catalog.boundary_offsets[world.0 as usize];
        let other_id =
            cpu.world.as_ref().unwrap().prepare.catalog.boundary_offsets[other.0 as usize];
        assert_ne!(first_id, other_id);
        assert_eq!(
            cpu.world.as_ref().unwrap().prepare.rgba_colors[other_id].refreshes(),
            0
        );
        // Both worlds use local surface/boundary id0. Their recipe slots must
        // retain independent colors despite sharing the material and images.
        for worlds in [&[other][..], &[other][..], &[world, other][..]] {
            let frame = packet(
                &assets,
                worlds,
                Refdef {
                    identity_light: 0.5,
                    ..test_view()
                },
            );
            assert_eq!(cpu.render(&frame, &assets).rejected, 0);
            let mut fresh = CpuBackend::load_with_limits(
                29,
                19,
                &assets,
                CpuLimits {
                    bands: RasterBands::Four,
                    ..CpuLimits::default()
                },
            )
            .unwrap();
            assert_eq!(fresh.render(&frame, &assets).rejected, 0);
            assert_eq!(cpu.pixels, fresh.pixels);
            assert_eq!(cpu.depth_ranks, fresh.depth_ranks);
            assert_eq!(
                cpu.inverse_depth
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                fresh
                    .inverse_depth
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            );
            let colors = &cpu.world.as_ref().unwrap().prepare.rgba_colors;
            assert_eq!(
                colors[first_id].refreshes(),
                if worlds.len() == 2 { 5 } else { 4 }
            );
            assert_eq!(colors[other_id].refreshes(), 1);
        }
    }
}

#[test]
fn non_affine_vertex_colors_keep_factor_cache_counter_merge() {
    let mut assets = Assets::load().unwrap();
    let image = assets
        .register_image(
            2,
            2,
            &[
                31, 47, 61, 255, 79, 97, 113, 255, 131, 151, 173, 255, 191, 211, 233, 255,
            ],
        )
        .unwrap();
    let light = assets.register_image(1, 1, &[173, 191, 211, 255]).unwrap();
    let material = stages(
        &mut assets,
        &[
            Stage {
                texture: StageTexture::Image(image),
                rgb_gen: RgbGen::ExactVertex,
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
            for (index, vertex) in g.vertices.iter_mut().enumerate() {
                vertex.vertex.texcoord = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][index];
                vertex.vertex.lightmap_coord = [0.5; 2];
                vertex.vertex.color = [
                    [32, 64, 96, 255],
                    [192, 160, 128, 255],
                    [64, 96, 128, 255],
                    [160, 128, 96, 255],
                ][index];
            }
        },
    );
    let list = packet(&assets, &[world], test_view());
    let counters = exact_rows(&assets, &list);
    assert!(counters.factor_spans > 0);
    assert!(counters.factor_pixels > 0);
    assert_eq!(counters.factor_hits, 0);
    assert!(counters.factor_fills > 0);
    assert_eq!(counters.rgba_spans, 0);
    assert_eq!(counters.factor_hits, counters.cache.hits);
    assert_eq!(counters.factor_fills, counters.cache.fills);
    let mut warm = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    warm.render(&list, &assets);
    let pixels = warm.pixels.to_vec();
    let depth = warm.inverse_depth.to_vec();
    let ranks = warm.depth_ranks.to_vec();
    warm.render(&list, &assets);
    let reused = warm.world_stats();
    assert!(reused.factor_hits > 0);
    assert_eq!(reused.factor_fills, 0);
    assert_eq!(warm.pixels.as_ref(), pixels);
    assert_eq!(warm.inverse_depth.as_ref(), depth);
    assert_eq!(warm.depth_ranks.as_ref(), ranks);
}

#[test]
fn simultaneous_sky_materials_retain_both_box_and_cloud_ranges() {
    let mut assets = Assets::load().unwrap();
    let red = assets.register_image(1, 1, &[30, 2, 1, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[1, 3, 40, 255]).unwrap();
    let mut worlds = Vec::new();
    let mut materials = Vec::new();
    for (index, image) in [red, blue].into_iter().enumerate() {
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
                sort: 2.0 + index as f32 * 2.0,
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
        materials.push(material);
        worlds.push(fixture_world(
            &mut assets,
            2.0,
            if index == 0 { 0.3 } else { 1.5 },
            SurfaceMaterial {
                material,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::Unpartitioned,
            |g| {
                if index == 0 {
                    for vertex in &mut g.vertices {
                        vertex.vertex.position.0[1] += 1.25;
                    }
                    g.surfaces[0].bounds.mins.0[1] += 1.25;
                    g.surfaces[0].bounds.maxs.0[1] += 1.25;
                }
            },
        ));
    }
    let overlay = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(blue),
            alpha_gen: AlphaGen::Const(0.5),
            blend: Some(StageBlend {
                source: BlendFactor::SourceAlpha,
                destination: BlendFactor::OneMinusSourceAlpha,
            }),
            depth_write: false,
            ..Stage::default()
        }],
        MaterialSettings {
            sort: 3.0,
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    worlds.push(fixture_world(
        &mut assets,
        1.0,
        0.3,
        SurfaceMaterial {
            material: overlay,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |_| {},
    ));
    // Another world contributes a disjoint region to the first sky material.
    // The shared material sort must union both sources, draw sky A once, then
    // the regular deferred surface, and finally sky B.
    worlds.push(fixture_world(
        &mut assets,
        2.0,
        0.3,
        SurfaceMaterial {
            material: materials[0],
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |g| {
            for vertex in &mut g.vertices {
                vertex.vertex.position.0[1] -= 1.25;
            }
            g.surfaces[0].bounds.mins.0[1] -= 1.25;
            g.surfaces[0].bounds.maxs.0[1] -= 1.25;
        },
    ));
    let frame = packet(&assets, &worlds, test_view());
    let counters = exact_rows(&assets, &frame);
    assert!(counters.sky_pixels > 0);
    assert!(counters.stage_pixels > 0);
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
        &camera,
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
    let draws = &raster.prepare.draws[..raster.prepare.draw_count];
    assert_eq!(draws.len(), 4);
    let sky_positions = draws
        .iter()
        .enumerate()
        .filter_map(|(rank, draw)| matches!(draw, PreparedDraw::Sky { .. }).then_some(rank))
        .collect::<Vec<_>>();
    let regular = draws
        .iter()
        .position(|draw| matches!(draw, PreparedDraw::Surface(_)))
        .unwrap();
    assert!(sky_positions[0] < regular && regular < sky_positions[1]);
    assert_eq!(
        draws
            .iter()
            .filter(|draw| matches!(draw, PreparedDraw::Skip))
            .count(),
        1
    );
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
fn indexed_wall_on_eye_plane_skips_without_rejecting_nearby_drawable_wall() {
    use crate::surface_cache::{IndexedLighting, IndexedTexture};
    let mut assets = Assets::load().unwrap();
    let palette = indexed_palette(&mut assets);
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![17; (16 >> mip) * (16 >> mip)]);
    let image = assets
        .register_indexed_image(
            IndexedTexture::load(
                16,
                16,
                std::array::from_fn(|mip| mips[mip].as_slice()),
                false,
            )
            .unwrap(),
            palette,
        )
        .unwrap();
    let material = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        }],
        MaterialSettings::default(),
    );
    // Retail base1 face 2154 is a drawable wall; its eye-plane projection
    // acquires a tiny signed area from the 135-degree camera's f32 rounding.
    let wall = fixture_world(
        &mut assets,
        2.0,
        0.5,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        Some(137),
        GeometryPartition::SplitBsp,
        |geometry| {
            let points = [
                [128.0, 128.0, 16.0],
                [128.0, 152.0, 16.0],
                [128.0, 152.0, 64.0],
                [128.0, 152.0, 112.0],
                [128.0, 128.0, 112.0],
            ];
            let normal = Vec3([-1.0, 0.0, 0.0]);
            geometry.vertices = points
                .into_iter()
                .map(|position| WorldVertex {
                    vertex: Vertex {
                        position: Vec3(position),
                        ..Vertex::default()
                    },
                    normal,
                })
                .collect();
            geometry.indices = vec![0, 1, 2, 0, 2, 4, 0, 1, 2, 3, 4];
            geometry.boundaries[0].count = 5;
            let surface = &mut geometry.surfaces[0];
            surface.vertices.count = 5;
            surface.plane = Some(Plane {
                normal,
                distance: -128.0,
                axis: None,
            });
            surface.bounds = Bounds {
                mins: Vec3([128.0, 128.0, 16.0]),
                maxs: Vec3([128.0, 152.0, 112.0]),
            };
            surface.texture_projection = [[0.0, 1.0, 0.0, 0.0], [0.0, 0.0, -1.0, 0.0]];
            surface.texture_minima = [128, -112];
            surface.texture_extents = [32, 96];
            surface.lightmap_grid = [3, 7];
            surface.light_samples.count = 21;
            geometry.light_samples = vec![[137; 3]; 21];
        },
    );
    let basis = qa_core::math::angle_vectors(Vec3([0.0, 135.0, 0.0]));
    let view = Refdef {
        viewport: Viewport {
            x: 0,
            y: 0,
            width: 640,
            height: 400,
        },
        origin: Vec3([128.0, -320.0, 63.0]),
        axes: [basis.forward, -basis.right, basis.up],
        fov: [120.0, 94.538_95],
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::BrightestRgb,
            ambient: 0,
            fullbright: false,
        },
        ..Refdef::default()
    };
    let mut cpu = CpuBackend::load_with_assets(640, 400, &assets).unwrap();
    let edge_on = cpu.render(&packet(&assets, &[wall], view), &assets);
    assert_eq!(edge_on.rejected, 0);
    assert_eq!(edge_on.surfaces, 0);
    assert_eq!(cpu.world_stats().indexed_pixels, 0);
    let nearby = Refdef {
        origin: Vec3([64.0, -320.0, 63.0]),
        ..view
    };
    let drawn = cpu.render(&packet(&assets, &[wall], nearby), &assets);
    assert_eq!(drawn.rejected, 0);
    assert_eq!(drawn.surfaces, 1);
    assert!(cpu.world_stats().indexed_pixels > 0);
}

#[test]
fn native_cache_and_full_seat_layered_sky_keep_one_row_bits() {
    use crate::surface_cache::{IndexedLighting, IndexedTexture};
    let mut assets = Assets::load().unwrap();
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
                4096,
                RasterSelection::Band(0),
            )
            .is_ok()
        );
        assert!(
            WorldBand::load(
                29,
                19,
                Arc::clone(&raster.prepare.catalog),
                total / bands - 1,
                4096,
                RasterSelection::Band(0),
            )
            .is_err()
        );
    }
}

#[test]
fn native_cube_background_prepares_planes_once_and_preserves_row_projection() {
    use crate::surface_cache::{IndexedLighting, IndexedTexture};
    let mut assets = Assets::load().unwrap();
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
    let frame = packet(&assets, &[world], view);
    let counters = exact_rows(&assets, &frame);
    assert_eq!(counters.polygons, 0);
    assert_eq!(
        counters.sky_pixels,
        u64::from(view.viewport.width * view.viewport.height)
    );
    let mut cpu = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            bands: RasterBands::Eight,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    let prepared = &cpu.world.as_ref().unwrap().prepare;
    assert!(prepared.background.is_some());
    assert_eq!(prepared.geometry.primitive_count, 0);
    for band in 0..8 {
        assert!(
            prepared
                .bins
                .range(RasterSelection::Band(band), [0, 0])
                .is_empty()
        );
    }
    let single_row = Refdef {
        viewport: Viewport {
            y: 8,
            height: 1,
            ..view.viewport
        },
        ..view
    };
    let counters = exact_rows(&assets, &packet(&assets, &[world], single_row));
    assert_eq!(counters.polygons, 0);
    assert_eq!(counters.sky_pixels, u64::from(single_row.viewport.width));
}

#[test]
fn opaque_only_draws_dispatch_once_and_error_counts_remain_collectable() {
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[91, 117, 143, 255]).unwrap();
    let material = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
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
    let frame = packet(&assets, &[world], test_view());
    let mut cpu = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            bands: RasterBands::Four,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    let mut calls = 0;
    let failed = cpu.render_with_dispatch(&frame, &assets, |jobs| {
        calls += 1;
        super::run_cpu_job(&mut jobs[0]);
        Err("dispatch rejected")
    });
    assert_eq!(failed, Err("dispatch rejected"));
    assert_eq!(calls, 1);
    let mut per_band = [WorldStats::default(); MAX_BANDS];
    assert_eq!(cpu.band_stats(&mut per_band), 4);
    assert!(per_band[0].pixels > 0);
    assert!(per_band[1..4].iter().all(|stats| stats.pixels == 0));
    assert_eq!(cpu.world_stats().pixels, per_band[0].pixels);
    let mut callbacks = 0;
    let observed = std::cell::Cell::new(0);
    let complete: Result<_, std::convert::Infallible> = cpu.render_observed(
        &frame,
        &assets,
        |jobs| {
            assert_eq!(observed.get(), 2);
            callbacks += 1;
            for job in jobs {
                super::run_cpu_job(job);
            }
            Ok(())
        },
        |point| match point {
            super::super::PreparePoint::Begin => {
                assert_eq!(observed.replace(1), 0);
            }
            super::super::PreparePoint::End => {
                assert_eq!(observed.replace(2), 1);
            }
        },
    );
    assert_eq!(complete.unwrap().rejected, 0);
    assert_eq!(callbacks, 1);
    assert_eq!(observed.get(), 2);
    let mut fresh = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    assert_eq!(fresh.render(&frame, &assets).rejected, 0);
    assert_eq!(fresh.pixels, cpu.pixels);
    assert_eq!(fresh.depth_ranks, cpu.depth_ranks);
}

#[test]
fn selected_band_load_keeps_total_budget_and_rejects_insufficient_native_share() {
    use crate::surface_cache::IndexedTexture;
    let mut assets = Assets::load().unwrap();
    let palette = indexed_palette(&mut assets);
    let mip_bytes: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![17; (16 >> mip) * (16 >> mip)]);
    let image = assets
        .register_indexed_image(
            IndexedTexture::load(
                16,
                16,
                std::array::from_fn(|mip| mip_bytes[mip].as_slice()),
                false,
            )
            .unwrap(),
            palette,
        )
        .unwrap();
    let material = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    fixture_world(
        &mut assets,
        2.0,
        1.0,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        Some(137),
        GeometryPartition::Unpartitioned,
        |_| {},
    );
    let base = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let required = base.raster_config().mandatory_cache_bytes;
    assert!(required > 0);
    for (rows, budget, expected) in [
        (19, required * 4, RasterBands::Four),
        (3, required * 8, RasterBands::Two),
        (1, required * 8, RasterBands::One),
    ] {
        let cpu = CpuBackend::load_with_limits(
            29,
            rows,
            &assets,
            CpuLimits {
                bands: RasterBands::Eight,
                auto_bands: true,
                cache_bytes: budget,
                ..CpuLimits::default()
            },
        )
        .unwrap();
        assert_eq!(cpu.raster_config().bands, expected);
        assert!(
            cpu.raster_config().mandatory_cache_bytes <= cpu.raster_config().per_band_cache_bytes
        );
    }
    for bands in [
        RasterBands::One,
        RasterBands::Two,
        RasterBands::Four,
        RasterBands::Eight,
    ] {
        let count = bands.count();
        assert!(
            CpuBackend::load_with_limits(
                29,
                19,
                &assets,
                CpuLimits {
                    bands,
                    cache_bytes: required * count - 1,
                    ..CpuLimits::default()
                }
            )
            .is_err()
        );
        let cpu = CpuBackend::load_with_limits(
            29,
            19,
            &assets,
            CpuLimits {
                bands,
                cache_bytes: required * count,
                ..CpuLimits::default()
            },
        )
        .unwrap();
        let config = cpu.raster_config();
        assert_eq!(config.allocated_cache_bytes, required * count);
        assert_eq!(config.per_band_cache_bytes, required);
        let padded = CpuBackend::load_with_limits(
            29,
            19,
            &assets,
            CpuLimits {
                bands,
                cache_bytes: 32 * 1024 * 1024 + 13,
                ..CpuLimits::default()
            },
        )
        .unwrap();
        let config = padded.raster_config();
        assert_eq!(config.total_cache_budget_bytes, 32 * 1024 * 1024 + 13);
        assert_eq!(
            config.per_band_cache_bytes,
            (config.total_cache_budget_bytes / 8 / count) * 8
        );
        assert_eq!(
            config.allocated_cache_bytes,
            config.per_band_cache_bytes * count
        );
        assert!(config.allocated_cache_bytes <= config.total_cache_budget_bytes);
    }
    // A map chart larger than the old 4 MiB band share must keep all requested
    // automatic lanes, while an explicit fixed budget still reduces them.
    fixture_world(
        &mut assets,
        3.0,
        1.0,
        SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |geometry| geometry.surfaces[0].texture_extents = [4096, 2048],
    );
    let automatic = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            bands: RasterBands::Eight,
            auto_bands: true,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    assert_eq!(automatic.raster_config().bands, RasterBands::Eight);
    assert!(automatic.raster_config().per_band_cache_bytes >= 8 * 1024 * 1024);
    let fixed = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            bands: RasterBands::Eight,
            auto_bands: true,
            cache_bytes: 32 * 1024 * 1024,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    assert_eq!(fixed.raster_config().bands, RasterBands::Four);
}

#[test]
fn coverage_bins_keep_odd_global_rows_and_uncertain_primitive_order() {
    let viewport = test_view().viewport;
    let mut primitives = Vec::new();
    let mut coverage = Vec::new();
    let mut append = |points: [[f32; 2]; 4]| {
        primitives.push(Primitive {
            first_coverage: coverage.len(),
            coverage_count: points.len(),
            ..Primitive::default()
        });
        coverage.extend(points.map(|xy| ProjectedVertex {
            xy,
            inverse_depth: 0.25,
            texcoord_over_depth: [0.0; 2],
        }));
    };
    append([[5.0, 4.0], [20.0, 4.0], [20.0, 7.0], [5.0, 7.0]]);
    append([[5.0, -2.0], [20.0, -2.0], [20.0, 24.0], [5.0, 24.0]]);
    append([[5.0, 4.1], [20.0, 4.1], [20.0, 4.25], [5.0, 4.25]]);
    append([[0.0, 5.0], [1e20, 6.0], [1e20, 10.0], [0.0, 10.0]]);
    let mut bins = CoverageBins::load(19, 8, primitives.len()).unwrap();
    assert_eq!(bins.capacity_bytes(), 8 * 4 * size_of::<u32>());
    bins.rebuild(
        viewport,
        DepthPolicy::PlaneDepth,
        primitives.len(),
        &primitives,
        &coverage,
    );
    for (band, expected) in [
        &[][..],
        &[1, 3][..],
        &[0, 1, 3][..],
        &[1, 3][..],
        &[1, 3][..],
        &[1, 3][..],
        &[1, 3][..],
        &[][..],
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(bins.range(RasterSelection::Band(band), [0, 4]), expected);
    }
    assert_eq!(bins.range(RasterSelection::Band(2), [1, 3]), &[1]);
    assert_eq!(bins.range(RasterSelection::Unbinned, [1, 3]), &[1, 2]);
    bins.rebuild(
        Viewport {
            y: 8,
            height: 1,
            ..viewport
        },
        DepthPolicy::BspKeys,
        primitives.len(),
        &primitives,
        &coverage,
    );
    for band in 0..8 {
        let expected = if band == 3 { &[1, 3][..] } else { &[][..] };
        assert_eq!(bins.range(RasterSelection::Band(band), [0, 4]), expected);
    }
    bins.rebuild(viewport, DepthPolicy::PlaneDepth, 0, &[], &[]);
    for band in 0..8 {
        assert!(bins.range(RasterSelection::Band(band), [0, 4]).is_empty());
    }
    assert!(CoverageBins::load(1, 8, 1).is_err());
    assert!(CoverageBins::load(19, 8, usize::MAX).is_err());
}

#[test]
fn single_offset_camera_row_uses_one_bin_with_exact_unbinned_output() {
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[91, 117, 143, 255]).unwrap();
    let material = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
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
        viewport: Viewport {
            y: 8,
            height: 1,
            ..test_view().viewport
        },
        ..test_view()
    };
    let frame = packet(&assets, &[world], view);
    let counters = exact_rows(&assets, &frame);
    assert_eq!(counters.polygons, 1);
    assert_eq!(counters.pixels, u64::from(view.viewport.width));
    let mut cpu = CpuBackend::load_with_limits(
        29,
        19,
        &assets,
        CpuLimits {
            bands: RasterBands::Eight,
            ..CpuLimits::default()
        },
    )
    .unwrap();
    assert_eq!(cpu.render(&frame, &assets).rejected, 0);
    let bins = &cpu.world.as_ref().unwrap().prepare.bins;
    for band in 0..8 {
        let expected = if band == 3 { &[0][..] } else { &[][..] };
        assert_eq!(bins.range(RasterSelection::Band(band), [0, 1]), expected);
    }
}

#[test]
fn public_dispatch_keeps_deferred_entity_poly_hud_and_overlapping_view_order() {
    let mut assets = Assets::load().unwrap();
    let red = assets.register_image(1, 1, &[193, 31, 47, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[17, 53, 211, 255]).unwrap();
    let opaque = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(red),
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    );
    let translucent = stages(
        &mut assets,
        &[Stage {
            texture: StageTexture::Image(blue),
            alpha_gen: AlphaGen::Const(0.375),
            depth_write: false,
            blend: Some(StageBlend {
                source: BlendFactor::SourceAlpha,
                destination: BlendFactor::OneMinusSourceAlpha,
            }),
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            sort: 6.0,
            ..MaterialSettings::default()
        },
    );
    let back = fixture_world(
        &mut assets,
        2.0,
        1.5,
        SurfaceMaterial {
            material: opaque,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |_| {},
    );
    let front = fixture_world(
        &mut assets,
        2.0,
        0.75,
        SurfaceMaterial {
            material: translucent,
            ..SurfaceMaterial::default()
        },
        None,
        GeometryPartition::Unpartitioned,
        |_| {},
    );
    let vertices = [
        [2.0, 1.0, 1.0],
        [2.0, -1.0, 1.0],
        [2.0, -1.0, -1.0],
        [2.0, 1.0, -1.0],
    ]
    .map(|p| Vertex {
        position: Vec3(p),
        ..Vertex::default()
    });
    let model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], opaque)
        .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    assert!(frame.add_world(
        back,
        &[VisibleSurface {
            surface: 0,
            depth_key: 9000
        }]
    ));
    assert!(frame.add_world(
        front,
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.add_entity(crate::scene::SceneEntity {
        model,
        color: [127, 255, 191, 255],
        ..crate::scene::SceneEntity::default()
    }));
    assert!(frame.add_poly(translucent, &vertices));
    assert!(frame.render_scene(
        Refdef {
            blend: [0.2, 0.1, 0.3, 0.25],
            ..test_view()
        },
        &[],
        &assets
    ));
    assert!(frame.draw_2d(crate::scene::Draw2d {
        rect: [4.0, 4.0, 13.0, 3.0],
        texcoords: [0.0, 0.0, 1.0, 1.0],
        color: [255; 4],
        material: opaque
    }));
    frame.clear_scene();
    assert!(frame.add_world(
        front,
        &[VisibleSurface {
            surface: 0,
            depth_key: 9000
        }]
    ));
    assert!(frame.add_world(
        back,
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.add_entity(crate::scene::SceneEntity {
        model,
        color: [255, 127, 191, 255],
        ..crate::scene::SceneEntity::default()
    }));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                x: 7,
                y: 5,
                width: 13,
                height: 11
            },
            blend: [0.1, 0.3, 0.2, 0.5],
            ..test_view()
        },
        &[],
        &assets
    ));
    assert!(frame.draw_2d(crate::scene::Draw2d {
        rect: [3.0, 9.0, 7.0, 2.0],
        texcoords: [0.0, 0.0, 1.0, 1.0],
        color: [255; 4],
        material: translucent
    }));
    exact_rows(&assets, &frame.finish());
}

#[test]
fn consecutive_prepared_ranks_batch_converted_sky_and_stop_at_external_draws() {
    let mut assets = Assets::load().unwrap();
    let image = assets.register_image(1, 1, &[71, 89, 103, 255]).unwrap();
    let materials: [MaterialId; 8] = std::array::from_fn(|index| {
        let sky = matches!(index, 2 | 3);
        let overlay = matches!(index, 1 | 5 | 6 | 7);
        stages(
            &mut assets,
            &[Stage {
                texture: StageTexture::Image(image),
                texgen: if sky {
                    TexCoordGen::CloudSky {
                        radius: 4096.0,
                        height: 384.0,
                    }
                } else {
                    TexCoordGen::Texture
                },
                alpha_gen: if overlay {
                    AlphaGen::Const(0.375)
                } else {
                    AlphaGen::Identity
                },
                depth_write: !overlay,
                blend: overlay.then_some(StageBlend {
                    source: BlendFactor::SourceAlpha,
                    destination: BlendFactor::OneMinusSourceAlpha,
                }),
                ..Stage::default()
            }],
            MaterialSettings {
                sort: index as f32 + 1.0,
                cull: Cull::None,
                sky: sky.then_some(Sky::Cube {
                    outer_box: Some([image; 6]),
                    inner_box: None,
                    clouds: crate::sky::CloudSphere::native(384.0),
                    rotation: None,
                    params: CubeSkyParams::default(),
                }),
                ..MaterialSettings::default()
            },
        )
    });
    let worlds: Vec<_> = [0, 1, 5, 7]
        .into_iter()
        .map(|index| {
            fixture_world(
                &mut assets,
                2.0,
                if index == 0 { 0.35 } else { 1.5 },
                SurfaceMaterial {
                    material: materials[index],
                    ..SurfaceMaterial::default()
                },
                None,
                GeometryPartition::Unpartitioned,
                |_| {},
            )
        })
        .collect();
    let vertices = [
        [2.0, 3.0, 3.0],
        [2.0, -3.0, 3.0],
        [2.0, -3.0, -3.0],
        [2.0, 3.0, -3.0],
    ]
    .map(|p| Vertex {
        position: Vec3(p),
        ..Vertex::default()
    });
    let sky_model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], materials[2])
        .unwrap();
    let external_model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], materials[4])
        .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([3, 5, 7, 255]).unwrap();
    for &world in &worlds {
        assert!(frame.add_world(
            world,
            &[VisibleSurface {
                surface: 0,
                depth_key: 0
            }]
        ));
    }
    for model in [sky_model, external_model] {
        assert!(frame.add_entity(crate::scene::SceneEntity {
            model,
            ..crate::scene::SceneEntity::default()
        }));
    }
    assert!(frame.add_poly(materials[3], &vertices));
    assert!(frame.add_poly(materials[6], &vertices));
    assert!(frame.render_scene(test_view(), &[], &assets));
    let frame = frame.finish();
    exact_rows(&assets, &frame);
    for bands in [
        RasterBands::One,
        RasterBands::Two,
        RasterBands::Four,
        RasterBands::Eight,
    ] {
        let mut cpu = CpuBackend::load_with_limits(
            29,
            19,
            &assets,
            CpuLimits {
                bands,
                ..CpuLimits::default()
            },
        )
        .unwrap();
        cpu.world
            .as_mut()
            .unwrap()
            .config
            .prepare_minimum_primitives_per_job = 0;
        let mut calls = 0;
        let mut preparations = 0;
        let result: Result<_, std::convert::Infallible> =
            cpu.render_with_dispatch(&frame, &assets, |jobs| {
                match jobs[0].kind() {
                    JobKind::Prepare => {
                        assert_eq!(calls, 0);
                        preparations += 1;
                        assert!(jobs.iter().all(|job| job.rows().is_none()));
                    }
                    JobKind::Raster => calls += 1,
                }
                for job in jobs.iter_mut().rev() {
                    super::run_cpu_job(job);
                }
                Ok(())
            });
        assert_eq!(result.unwrap().rejected, 0);
        // Opaque, then three prepared runs separated by external entity/poly.
        assert_eq!(calls, 4);
        assert_eq!(
            preparations,
            usize::from(bands.count() > 1 && bands.count() <= worlds.len())
        );
        let prepared = &cpu.world.as_ref().unwrap().prepare;
        assert!(matches!(
            &prepared.draws[..prepared.draw_count],
            [
                PreparedDraw::Skip,
                PreparedDraw::Surface(_),
                PreparedDraw::Sky { .. },
                PreparedDraw::Sky { .. },
                PreparedDraw::External,
                PreparedDraw::Surface(_),
                PreparedDraw::External,
                PreparedDraw::Surface(_)
            ]
        ));
    }

    let limits = CpuLimits {
        bands: RasterBands::Four,
        ..CpuLimits::default()
    };
    let mut paused = CpuBackend::load_with_limits(29, 19, &assets, limits).unwrap();
    paused
        .world
        .as_mut()
        .unwrap()
        .config
        .prepare_minimum_primitives_per_job = 0;
    let mut calls = 0;
    let result = paused.render_with_dispatch(&frame, &assets, |jobs| {
        if jobs[0].kind() == JobKind::Raster {
            calls += 1;
        }
        if jobs[0].kind() == JobKind::Raster && calls == 2 {
            return Err(());
        }
        for job in jobs {
            super::run_cpu_job(job);
        }
        Ok(())
    });
    assert_eq!(result, Err(()));
    assert_eq!(calls, 2);
    let mut before = [WorldStats::default(); MAX_BANDS];
    assert_eq!(paused.band_stats(&mut before), 4);

    let mut failed = CpuBackend::load_with_limits(29, 19, &assets, limits).unwrap();
    failed
        .world
        .as_mut()
        .unwrap()
        .config
        .prepare_minimum_primitives_per_job = 0;
    let mut calls = 0;
    let result = failed.render_with_dispatch(&frame, &assets, |jobs| {
        if jobs[0].kind() == JobKind::Raster {
            calls += 1;
        }
        if jobs[0].kind() == JobKind::Raster && calls == 2 {
            super::run_cpu_job(&mut jobs[0]);
            return Err(());
        }
        for job in jobs {
            super::run_cpu_job(job);
        }
        Ok(())
    });
    assert_eq!(result, Err(()));
    assert_eq!(calls, 2);
    let mut attempted = [WorldStats::default(); MAX_BANDS];
    assert_eq!(failed.band_stats(&mut attempted), 4);
    assert!(attempted[0].pixels > before[0].pixels);
    assert_eq!(&attempted[1..4], &before[1..4]);
    let other_rows = (19 / 4) * 29;
    assert_eq!(&failed.pixels[other_rows..], &paused.pixels[other_rows..]);
    assert_eq!(
        &failed.depth_ranks[other_rows..],
        &paused.depth_ranks[other_rows..]
    );
    assert_band_counters(failed.world_stats(), &attempted[..4]);

    let mut preparation_failed = CpuBackend::load_with_limits(29, 19, &assets, limits).unwrap();
    preparation_failed
        .world
        .as_mut()
        .unwrap()
        .config
        .prepare_minimum_primitives_per_job = 0;
    let result = preparation_failed.render_with_dispatch(&frame, &assets, |jobs| {
        assert_eq!(jobs[0].kind(), JobKind::Prepare);
        super::run_cpu_job(&mut jobs[0]);
        Err("preparation rejected")
    });
    assert_eq!(result, Err("preparation rejected"));
    assert_eq!(preparation_failed.world_stats().pixels, 0);
    assert!(preparation_failed.depth_ranks.iter().all(|&rank| rank == 0));
    assert!(
        preparation_failed
            .pixels
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([3, 5, 7, 255]))
    );
    let retry: Result<_, std::convert::Infallible> =
        preparation_failed.render_with_dispatch(&frame, &assets, |jobs| {
            for job in jobs.iter_mut().rev() {
                super::run_cpu_job(job);
                // The output loan is consumed once, including preparation jobs.
                super::run_cpu_job(job);
            }
            Ok(())
        });
    let mut fresh = CpuBackend::load_with_limits(29, 19, &assets, limits).unwrap();
    assert_eq!(retry.unwrap(), fresh.render(&frame, &assets));
    assert_eq!(preparation_failed.pixels, fresh.pixels);
    assert_eq!(preparation_failed.inverse_depth, fresh.inverse_depth);
    assert_eq!(preparation_failed.depth_ranks, fresh.depth_ranks);
    assert_eq!(preparation_failed.world_stats(), fresh.world_stats());
}
