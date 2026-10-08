use crate::assets::{
    Assets, DepthFunc, Filter, ImageId, MaterialId, MaterialSettings, Sampler, Stage, StageTexture,
    TextureIntensity, Vertex, Wrap,
    upload::{MipmapBuild, UploadParams},
};
use crate::cpu::{CpuBackend, CpuLimits};
use crate::lightmap::AtlasRegion;
use crate::scene::{CommandList, CpuPresentation, FrontEnd, Limits, Refdef, Viewport};
use crate::shader::{BlendFactor, Cull, RgbGen, StageBlend, TexCoordGen};
use crate::world::{SurfaceMaterial, VisibleSurface, WorldId, geometry::*};
use qa_core::primitives::{Bounds, Vec3};
use qa_formats::bsp::IndexRange;
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};

fn fixture_world(
    assets: &mut Assets,
    binding: SurfaceMaterial,
    near_clip: bool,
    constant_uv: bool,
) -> WorldId {
    let positions = [
        [2.0, 2.0, 2.0],
        [2.0, -2.0, 2.0],
        [2.0, -2.0, -2.0],
        [2.0, 2.0, -2.0],
    ];
    let uv = [[-1.25, -0.25], [2.25, -0.25], [2.25, 1.75], [-1.25, 1.75]];
    let vertices = positions
        .into_iter()
        .enumerate()
        .map(|(i, mut position)| {
            if near_clip && (i == 0 || i == 3) {
                position[0] = 0.05;
            }
            WorldVertex {
                vertex: Vertex {
                    position: Vec3(position),
                    texcoord: if constant_uv { [-0.25, 0.5] } else { uv[i] },
                    lightmap_coord: [[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]][i],
                    color: [
                        [255, 91, 173, 255],
                        [79, 255, 131, 192],
                        [211, 139, 255, 128],
                        [103, 191, 71, 224],
                    ][i],
                    ..Vertex::default()
                },
                normal: Vec3([-1.0, 0.0, 0.0]),
            }
        })
        .collect();
    let bounds = Bounds {
        mins: Vec3([0.05, -2.0, -2.0]),
        maxs: Vec3([2.0, 2.0, 2.0]),
    };
    let geometry = WorldGeometry {
        partition: GeometryPartition::Unpartitioned,
        world_has_lightdata: false,
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3, 0, 1, 2, 3],
        boundaries: vec![IndexRange { first: 6, count: 4 }],
        surfaces: vec![WorldSurface {
            source_id: 0,
            kind: GeometryKind::Polygon,
            vertices: IndexRange { first: 0, count: 4 },
            indices: IndexRange { first: 0, count: 6 },
            boundaries: IndexRange { first: 0, count: 1 },
            plane: None,
            bounds,
            texture_coordinates: TextureCoordinates::Normalized,
            texture_projection: [[0.0; 4]; 2],
            texture_minima: [0; 2],
            texture_extents: [0; 2],
            lightmap_grid: [0; 2],
            styles: [255; 4],
            light_source: LightSource::Page(0),
            light_encoding: LightEncoding::Rgb,
            light_samples: IndexRange { first: 0, count: 0 },
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
        light_samples: vec![],
        models: vec![],
        patch_stats: PatchStats::default(),
    };
    let vis = VisibilityWorld::load(
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
        .register_world_with_bindings(geometry, vis, &[binding])
        .unwrap()
}

fn material(
    assets: &mut Assets,
    base: ImageId,
    reverse: bool,
    linear: bool,
    depth: DepthFunc,
    alternate_multiply: bool,
) -> MaterialId {
    let base_stage = Stage {
        texture: StageTexture::Image(base),
        rgb_gen: RgbGen::Vertex,
        texture_intensity: TextureIntensity::NeutralizeUpload,
        sampler: Sampler {
            wrap: Wrap::Repeat,
            filter: if linear {
                Filter::Linear
            } else {
                Filter::Nearest
            },
            mipmaps: true,
        },
        ..Stage::default()
    };
    let light = Stage {
        texture: StageTexture::Lightmap,
        texgen: TexCoordGen::Lightmap,
        rgb_gen: RgbGen::IdentityLighting,
        sampler: Sampler {
            wrap: Wrap::Clamp,
            filter: Filter::Linear,
            mipmaps: true,
        },
        ..Stage::default()
    };
    let mut stages = if reverse {
        [light, base_stage]
    } else {
        [base_stage, light]
    };
    stages[1].blend = Some(if alternate_multiply {
        StageBlend {
            source: BlendFactor::Zero,
            destination: BlendFactor::SourceColor,
        }
    } else {
        StageBlend {
            source: BlendFactor::DestinationColor,
            destination: BlendFactor::Zero,
        }
    });
    stages[1].depth_write = false;
    stages[1].depth_func = depth;
    assets
        .register_material(
            "fixed factor oracle",
            &stages,
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}

fn packet(assets: &Assets, worlds: &[WorldId]) -> CommandList {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([17, 29, 43, 255]).unwrap();
    for (index, &world) in worlds.iter().enumerate() {
        assert!(frame.add_world(
            world,
            &[VisibleSurface {
                surface: 0,
                depth_key: (worlds.len() - index) as u32
            }]
        ));
    }
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 8,
                height: 8,
                ..Viewport::default()
            },
            near: 0.25,
            far: 64.0,
            fov: [90.0; 2],
            identity_light: 0.5,
            cpu_presentation: CpuPresentation::Rgb,
            ..Refdef::default()
        },
        &[],
        assets
    ));
    frame.finish()
}

fn assert_buffers(fixed: &CpuBackend, generic: &CpuBackend) {
    assert_eq!(fixed.pixels(), generic.pixels());
    assert_eq!(
        fixed
            .inverse_depth
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>(),
        generic
            .inverse_depth
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(fixed.depth_ranks, generic.depth_ranks);
}

fn compare(
    reverse: bool,
    linear: bool,
    near_clip: bool,
    constant_uv: bool,
    region: bool,
    budget: usize,
    depth: DepthFunc,
    alternate_multiply: bool,
) {
    let mut assets = Assets::load();
    let bytes: Vec<_> = (0..64)
        .flat_map(|i| {
            [
                (i * 31 % 256) as u8,
                (i * 73 % 256) as u8,
                (i * 11 % 256) as u8,
                (127 + i * 2) as u8,
            ]
        })
        .collect();
    let base = assets.register_image(8, 8, &bytes).unwrap();
    let light = assets
        .register_image(8, 8, &bytes.iter().rev().copied().collect::<Vec<_>>())
        .unwrap();
    let material = material(
        &mut assets,
        base,
        reverse,
        linear,
        depth,
        alternate_multiply,
    );
    let binding = SurfaceMaterial {
        material,
        lightmap: light,
        lightmap_region: region.then_some(AtlasRegion {
            page: 0,
            x: 2,
            y: 2,
            width: 4,
            height: 4,
        }),
        texture_scale: [1.0; 2],
    };
    let world = fixture_world(&mut assets, binding, near_clip, constant_uv);
    let other = fixture_world(&mut assets, binding, false, true);
    let mut generic = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    // The old numeric preparation revision makes the existing renderer use its
    // generic path, while both backends consume this exact new packet/assets.
    assets
        .prepare_image(
            base,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                inverse_intensity: 0.75,
                ..UploadParams::default()
            },
        )
        .unwrap();
    if !region {
        assets
            .prepare_image(
                light,
                UploadParams {
                    mipmaps: MipmapBuild::Box,
                    ..UploadParams::default()
                },
            )
            .unwrap();
    }
    let mut fixed = CpuBackend::load_with_limits(
        8,
        8,
        &assets,
        CpuLimits {
            cache_bytes: budget,
            max_spans: 16,
        },
    )
    .unwrap();
    let packet = packet(&assets, &[other, world]);
    assert_eq!(generic.render(&packet, &assets).rejected, 0);
    assert_eq!(fixed.render(&packet, &assets).rejected, 0);
    assert!(generic.world_stats().stage_spans > 0);
    assert_eq!(fixed.world_stats().stage_spans, 0);
    assert!(fixed.world_stats().factor_spans > 0);
    assert_buffers(&fixed, &generic);
    if budget < 8 {
        assert_eq!(fixed.world_stats().factor_fills, 0);
        assert!(fixed.world_stats().factor_fallback_spans > 0);
        assert!(fixed.world_stats().factor_rejected > 0);
    } else {
        assert!(fixed.world_stats().factor_fills > 0);
        assert_eq!(fixed.world_stats().factor_fallback_spans, 0);
    }
}

#[test]
fn factors_match_generic_pixels_depth_and_rank_for_native_grids() {
    for reverse in [false, true] {
        for linear in [false, true] {
            for near_clip in [false, true] {
                for constant_uv in [false, true] {
                    for region in [false, true] {
                        compare(
                            reverse,
                            linear,
                            near_clip,
                            constant_uv,
                            region,
                            32 * 1024 * 1024,
                            DepthFunc::Equal,
                            false,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn factor_reservation_failure_keeps_original_views_and_exact_coverage() {
    compare(false, true, true, false, true, 4, DepthFunc::Equal, false);
    compare(true, false, false, true, false, 4, DepthFunc::Equal, false);
}

#[test]
fn factor_multiply_forms_and_depth_functions_keep_generic_ties_and_clipping() {
    for reverse in [false, true] {
        for near_clip in [false, true] {
            for depth in [DepthFunc::Equal, DepthFunc::Lequal] {
                for alternate_multiply in [false, true] {
                    compare(
                        reverse,
                        true,
                        near_clip,
                        false,
                        true,
                        32 * 1024 * 1024,
                        depth,
                        alternate_multiply,
                    );
                }
            }
        }
    }
}

#[test]
fn invalid_prepared_region_preserves_each_generic_stage_and_depth() {
    use crate::assets::upload::{ExtentRound, UploadExtent};
    for reverse in [false, true] {
        let mut assets = Assets::load();
        let base = assets.register_image(1, 1, &[255; 4]).unwrap();
        let light = assets.register_image(8, 8, &[255; 8 * 8 * 4]).unwrap();
        let material = material(&mut assets, base, reverse, true, DepthFunc::Equal, false);
        let world = fixture_world(
            &mut assets,
            SurfaceMaterial {
                material,
                lightmap: light,
                texture_scale: [1.0; 2],
                lightmap_region: Some(AtlasRegion {
                    page: 0,
                    x: 4,
                    y: 0,
                    width: 4,
                    height: 4,
                }),
            },
            false,
            false,
        );
        let mut generic = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
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
        let mut fixed = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
        let packet = packet(&assets, &[world]);
        let generic_stats = generic.render(&packet, &assets);
        let fixed_stats = fixed.render(&packet, &assets);
        assert!(generic_stats.rejected > 0);
        assert_eq!(fixed_stats.rejected, generic_stats.rejected);
        assert_buffers(&fixed, &generic);
        assert_eq!(fixed.world_stats().factor_spans, 0);
        if !reverse {
            assert!(
                fixed
                    .pixels()
                    .iter()
                    .any(|&pixel| pixel != u32::from_le_bytes([17, 29, 43, 255]))
            );
            assert!(fixed.inverse_depth.iter().any(|&zi| zi > 0.0));
        }
    }
}

#[test]
fn numeric_factor_sources_share_native_mips_and_distinguish_regions() {
    use super::Factor;
    use crate::surface_cache::SurfaceCache;
    let mut assets = Assets::load();
    let bytes: Vec<_> = (0..16)
        .flat_map(|value| [value * 13, value * 7, value * 3, 255])
        .collect();
    let image = assets.register_image(4, 4, &bytes).unwrap();
    assets
        .prepare_image(
            image,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let mut factors = Vec::new();
    let mut sources = Vec::new();
    let full = Factor::register(image, None, &assets, &mut factors, &mut sources).unwrap();
    assert_eq!(
        Factor::register(image, None, &assets, &mut factors, &mut sources).unwrap(),
        full
    );
    let region = AtlasRegion {
        page: 0,
        x: 1,
        y: 1,
        width: 2,
        height: 2,
    };
    let local = Factor::register(image, Some(region), &assets, &mut factors, &mut sources).unwrap();
    assert_ne!(local, full);
    assert_eq!(factors.len(), 2);
    assert_eq!(sources.len(), 4);
    let mut cache = SurfaceCache::load(sources, 512).unwrap();
    assert!(cache.begin_batch());
    let block = factors[full].prepare(1, &assets, &mut cache).unwrap();
    let prepared = &assets
        .image(image)
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .levels[1];
    assert_eq!(
        cache.rgba_pixels(block).unwrap().as_flattened(),
        prepared.rgba.as_ref()
    );
    let hit = factors[full].prepare(1, &assets, &mut cache).unwrap();
    assert_eq!(cache.rgba_pixels(hit), cache.rgba_pixels(block));
    let roi = factors[local].prepare(0, &assets, &mut cache).unwrap();
    let expected: Vec<_> = [5, 6, 9, 10]
        .into_iter()
        .flat_map(|pixel| bytes[pixel * 4..pixel * 4 + 4].iter().copied())
        .collect();
    assert_eq!(
        cache.rgba_pixels(roi).unwrap().as_flattened(),
        expected.as_slice()
    );
    assert_eq!(cache.stats().fills, 2);
    assert_eq!(cache.stats().hits, 1);
    cache.end_batch();
}
