use super::{Combination, Product, Recipe};
use crate::assets::{
    Assets, DepthFunc, MaterialSettings, Sampler, Stage, StageTexture, TextureIntensity, Vertex,
    Wrap,
    upload::{MipmapBuild, UploadParams},
};
use crate::lightmap::AtlasRegion;
use crate::scene::Refdef;
use crate::shader::{BlendFactor, RgbGen, StageBlend, TexCoordGen};
use crate::stage::StageEvaluator;
use crate::surface_cache::{CacheSpan, SurfaceCache};
use crate::world::{SurfaceBinding, geometry::*};
use qa_core::primitives::{Bounds, Vec3};
use qa_formats::bsp::IndexRange;

struct Fixture {
    assets: Assets,
    geometry: WorldGeometry,
    binding: SurfaceBinding,
    evaluator: StageEvaluator,
    vertex_color: bool,
}

fn fixture(reverse: bool, vertex_color: bool, equal: bool) -> Fixture {
    let mut assets = Assets::load();
    let base_pixels: Vec<_> = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                [
                    37 + 24 * x + 8 * y,
                    71 + 8 * x + 24 * y,
                    143 + 8 * x - 16 * y,
                    255,
                ]
            })
        })
        .collect();
    let base = assets.register_image(4, 4, &base_pixels).unwrap();
    assets
        .prepare_image(
            base,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                inverse_intensity: 0.5,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let light_pixels: Vec<_> = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                if (1..=2).contains(&x) && (1..=2).contains(&y) {
                    [
                        48 + 48 * (x - 1) + 24 * (y - 1),
                        144 - 32 * (x - 1),
                        80 + 40 * (y - 1),
                        255,
                    ]
                } else {
                    [255, 0, 255, 255]
                }
            })
        })
        .collect();
    let lightmap = assets.register_image(4, 4, &light_pixels).unwrap();
    assets
        .prepare_image(
            lightmap,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let rgb_gen = if vertex_color {
        RgbGen::Vertex
    } else {
        RgbGen::Identity
    };
    let base_stage = Stage {
        texture: StageTexture::Image(base),
        rgb_gen,
        texture_intensity: TextureIntensity::NeutralizeUpload,
        ..Stage::default()
    };
    let light_stage = Stage {
        texture: StageTexture::Lightmap,
        texgen: TexCoordGen::Lightmap,
        rgb_gen,
        // Admission must not reject native mip0 lightmaps merely because this
        // stage flag or image happens to retain additional levels.
        sampler: Sampler {
            mipmaps: true,
            ..Sampler::default()
        },
        ..Stage::default()
    };
    let mut stages = if reverse {
        [light_stage, base_stage]
    } else {
        [base_stage, light_stage]
    };
    stages[1].blend = Some(StageBlend {
        source: BlendFactor::DestinationColor,
        destination: BlendFactor::Zero,
    });
    stages[1].depth_write = false;
    stages[1].depth_func = if equal {
        DepthFunc::Equal
    } else {
        DepthFunc::Lequal
    };
    let material = assets
        .register_material(
            "static lattice oracle",
            &stages,
            MaterialSettings::default(),
        )
        .unwrap();
    let positions = [
        [0.0, 0.0, 0.0],
        [4.0, 0.0, 0.0],
        [4.0, 4.0, 0.0],
        [0.0, 4.0, 0.0],
    ];
    let vertices = positions
        .into_iter()
        .enumerate()
        .map(|(index, position)| {
            let mut color = [
                [64, 128, 192, 255],
                [96, 160, 160, 255],
                [128, 192, 128, 255],
                [96, 160, 160, 255],
            ][index];
            if !vertex_color && index == 3 {
                color = [19, 211, 37, 113];
            }
            WorldVertex {
                vertex: Vertex {
                    position: Vec3(position),
                    texcoord: [position[0] / 4.0, position[1] / 4.0],
                    lightmap_coord: [0.25 + position[0] / 8.0, 0.25 + position[1] / 8.0],
                    color,
                    ..Vertex::default()
                },
                normal: Vec3([0.0, 0.0, 1.0]),
            }
        })
        .collect();
    let bounds = Bounds {
        mins: Vec3([0.0; 3]),
        maxs: Vec3([4.0, 4.0, 0.0]),
    };
    let geometry = WorldGeometry {
        partition: GeometryPartition::Unpartitioned,
        world_has_lightdata: true,
        vertices,
        indices: vec![0, 1, 2, 3],
        boundaries: vec![IndexRange { first: 0, count: 4 }],
        surfaces: vec![WorldSurface {
            source_id: 0,
            kind: GeometryKind::Polygon,
            vertices: IndexRange { first: 0, count: 4 },
            indices: IndexRange { first: 0, count: 4 },
            boundaries: IndexRange { first: 0, count: 1 },
            plane: None,
            bounds,
            texture_coordinates: TextureCoordinates::Normalized,
            texture_projection: [[0.0; 4]; 2],
            texture_minima: [0; 2],
            texture_extents: [0; 2],
            lightmap_grid: [0; 2],
            styles: [0, 255, 255, 255],
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
    Fixture {
        assets,
        geometry,
        evaluator: StageEvaluator::load(),
        vertex_color,
        binding: SurfaceBinding {
            material,
            lightmap,
            lightmap_region: Some(AtlasRegion {
                page: 7,
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
            texture_scale: [1.0; 2],
            mesh_indices: crate::scene::Span { first: 0, count: 4 },
        },
    }
}

fn product(fixture: &Fixture, cache_bytes: usize) -> Option<Product> {
    Product::load(
        &fixture.geometry,
        0,
        fixture.binding,
        fixture.assets.material(fixture.binding.material).unwrap(),
        &fixture.assets,
        &fixture.evaluator,
        0,
        cache_bytes,
    )
}

// Independent mathematical oracle: fetch the original prepared mip texel and
// weight the four mip0 lightmap bytes in f64 inside the owned atlas rectangle.
// It does not call any production sampler, affine field or blend helper.
fn expected(
    fixture: &Fixture,
    recipe: &Product,
    mip: u8,
    chart: [f32; 2],
    identity: f32,
    tint: Option<[u8; 4]>,
) -> [u8; 4] {
    let base = fixture
        .assets
        .image(recipe.base)
        .unwrap()
        .prepared
        .as_ref()
        .unwrap();
    let level = &base.levels[mip as usize];
    let step = (1u64 << mip) as f64;
    let xy = chart.map(|value| (f64::from(value) / step).floor());
    let x = xy[0].rem_euclid(f64::from(level.width)) as usize;
    let y = xy[1].rem_euclid(f64::from(level.height)) as usize;
    let base_pixel = &level.rgba[(y * level.width as usize + x) * 4..][..4];
    let light = &fixture
        .assets
        .image(fixture.binding.lightmap)
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .levels[0];
    let p = chart.map(|value| (0.25 + f64::from(value) / 8.0) * 4.0 - 0.5);
    let floor = p.map(f64::floor);
    let fraction = [p[0] - floor[0], p[1] - floor[1]];
    let mut lighting = [0.0; 4];
    for row in 0..2 {
        for column in 0..2 {
            let x = ((floor[0] + column as f64) as i32).clamp(1, 2) as usize;
            let y = ((floor[1] + row as f64) as i32).clamp(1, 2) as usize;
            let weight = if column == 0 {
                1.0 - fraction[0]
            } else {
                fraction[0]
            } * if row == 0 {
                1.0 - fraction[1]
            } else {
                fraction[1]
            };
            for channel in 0..4 {
                lighting[channel] +=
                    f64::from(light.rgba[(y * 4 + x) * 4 + channel]) / 255.0 * weight;
            }
        }
    }
    let color = if let Some(tint) = tint {
        tint.map(|channel| f64::from(channel) / 255.0)
    } else if fixture.vertex_color {
        let origin = [64.0, 128.0, 192.0, 255.0];
        let delta = [8.0, 8.0, -8.0, 0.0];
        std::array::from_fn(|channel| {
            if channel == 3 {
                1.0
            } else {
                ((origin[channel] * f64::from(identity)).floor()
                    + delta[channel] * f64::from(identity) * f64::from(chart[0] + chart[1]))
                .clamp(0.0, 255.0)
                    / 255.0
            }
        })
    } else {
        [1.0; 4]
    };
    std::array::from_fn(|channel| {
        let base = f64::from(base_pixel[channel]) / 255.0
            * if channel < 3 {
                f64::from(base.inverse_intensity)
            } else {
                1.0
            };
        let factors = if recipe.base_stage == 0 {
            [base, lighting[channel]]
        } else {
            [lighting[channel], base]
        };
        let result = match recipe.combination {
            Combination::Modulate => factors[0] * factors[1] * color[channel],
            Combination::SeparatePasses => {
                let first = (factors[0] * color[channel] * 255.0)
                    .clamp(0.0, 255.0)
                    .round()
                    / 255.0;
                first * factors[1] * color[channel]
            }
        };
        (result * 255.0).clamp(0.0, 255.0).round() as u8
    })
}

fn build(
    cache: &mut SurfaceCache,
    fixture: &Fixture,
    recipe: &Product,
    mip: u8,
    refdef: Refdef,
) -> CacheSpan {
    assert!(cache.begin_batch());
    let prepared = recipe.prepare(refdef, &fixture.evaluator).unwrap();
    let block = cache
        .prepare_rgba(recipe.cache, mip, prepared.state, |out| {
            recipe.fill(prepared, mip, &fixture.assets, out);
        })
        .unwrap();
    cache.end_batch();
    block
}

#[test]
fn static_lattice_matches_native_mip_and_variable_bilinear_lightmap_oracle() {
    for reverse in [false, true] {
        for vertex_color in [false, true] {
            for equal in [false, true] {
                let fixture = fixture(reverse, vertex_color, equal);
                let recipe = product(&fixture, 65536).unwrap();
                assert_eq!(recipe.mip_count, 3);
                assert_eq!(recipe.minima, [-1; 2]);
                assert_eq!(recipe.extents, [6; 2]);
                let mut cache = SurfaceCache::load(vec![recipe.source().unwrap()], 65536).unwrap();
                for mip in 0..recipe.mip_count {
                    let refdef = Refdef {
                        identity_light: 0.5,
                        ..Refdef::default()
                    };
                    let block = build(&mut cache, &fixture, &recipe, mip, refdef);
                    let pixels = cache.rgba_pixels(block).unwrap();
                    let step = (1u64 << mip) as f32;
                    for y in 0..block.height {
                        for x in 0..block.width {
                            let chart = [
                                block.texture_mins[0] as f32 + (x as f32 + 0.5) * step,
                                block.texture_mins[1] as f32 + (y as f32 + 0.5) * step,
                            ];
                            let actual = pixels[(y * block.width + x) as usize];
                            let expected = expected(
                                &fixture,
                                &recipe,
                                mip,
                                chart,
                                refdef.identity_light,
                                None,
                            );
                            assert_eq!(
                                actual, expected,
                                "reverse={reverse} vertex={vertex_color} equal={equal} mip={mip} chart={chart:?}"
                            );
                            assert_eq!(
                                Product::cached_pixel(block, pixels, chart),
                                u32::from_le_bytes(actual)
                            );
                            assert_eq!(
                                Product::cached_pixel(
                                    block,
                                    pixels,
                                    [chart[0] - 0.49 * step, chart[1] + 0.49 * step]
                                ),
                                u32::from_le_bytes(actual)
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn collapse_applies_shared_constant_color_once_and_rounds_only_final_product() {
    let mut fixture = fixture(false, false, false);
    let material = fixture.assets.material(fixture.binding.material).unwrap();
    let mut stages = [material.stages[0], material.stages[1]];
    stages[0].rgb_gen = RgbGen::Const([0.5; 3]);
    stages[1].rgb_gen = RgbGen::Const([0.25; 3]);
    fixture.binding.material = fixture
        .assets
        .register_material(
            "native collapse color",
            &stages,
            MaterialSettings::default(),
        )
        .unwrap();
    let recipe = product(&fixture, 65536).unwrap();
    assert!(matches!(recipe.combination, Combination::Modulate));
    let mut cache = SurfaceCache::load(vec![recipe.source().unwrap()], 65536).unwrap();
    let block = build(&mut cache, &fixture, &recipe, 0, Refdef::default());
    let pixels = cache.rgba_pixels(block).unwrap();
    // Native constant_byte truncates .5 to127. The collapse retains stage0's
    // color once, despite stage1's different constant of the same generator.
    for (index, pixel) in pixels.iter().enumerate() {
        let chart = [
            block.texture_mins[0] as f32 + (index as u32 % block.width) as f32 + 0.5,
            block.texture_mins[1] as f32 + (index as u32 / block.width) as f32 + 0.5,
        ];
        let original = recipe
            .prepare(Refdef::default(), &fixture.evaluator)
            .unwrap();
        assert_eq!(original.state.stage_colors[0], [127, 127, 127, 255]);
        assert_eq!(
            *pixel,
            expected(&fixture, &recipe, 0, chart, 1.0, Some([127, 127, 127, 255]))
        );
    }
}

#[test]
fn static_cache_reuses_payload_and_explicit_input_changes_invalidate() {
    let mut fixture = fixture(false, false, false);
    let recipe = product(&fixture, 65536).unwrap();
    let mut cache = SurfaceCache::load(vec![recipe.source().unwrap()], 65536).unwrap();
    let refdef = Refdef::default();
    let first = build(&mut cache, &fixture, &recipe, 0, refdef);
    let original_pixels = cache.rgba_pixels(first).unwrap().to_vec();
    assert_eq!(cache.stats().fills, 1);
    let second = build(&mut cache, &fixture, &recipe, 0, refdef);
    assert_eq!(cache.rgba_pixels(second).unwrap(), original_pixels);
    assert_eq!(cache.stats().fills, 1);
    assert_eq!(cache.stats().hits, 1);
    let mut changed = refdef;
    changed.lightstyles[0].rgb = [0.5, 0.75, 1.0];
    let third = build(&mut cache, &fixture, &recipe, 0, changed);
    assert_eq!(cache.stats().fills, 2);
    // Atlas style animation is not supplied by this cache recipe. Its baked
    // bytes remain unchanged until the shared atlas resource is updated.
    assert_eq!(cache.rgba_pixels(third).unwrap(), original_pixels);
    let prepared = recipe.prepare(changed, &fixture.evaluator).unwrap();
    for state in [
        crate::surface_cache::RgbaBuildState {
            lighting_revision: 1,
            ..prepared.state
        },
        crate::surface_cache::RgbaBuildState {
            dynamic_revision: 1,
            ..prepared.state
        },
    ] {
        assert!(cache.begin_batch());
        cache
            .prepare_rgba(recipe.cache, 0, state, |out| {
                recipe.fill(prepared, 0, &fixture.assets, out)
            })
            .unwrap();
        cache.end_batch();
    }
    assert_eq!(cache.stats().fills, 4);
    assert!(recipe.current(&fixture.assets));
    fixture
        .assets
        .prepare_image(recipe.base, UploadParams::default())
        .unwrap();
    assert!(!recipe.current(&fixture.assets));
    let reloaded = product(&fixture, 65536).unwrap();
    assert!(reloaded.current(&fixture.assets));
    fixture
        .assets
        .set_image_native_sampler(
            recipe.base,
            Sampler {
                wrap: Wrap::Clamp,
                ..Sampler::default()
            },
        )
        .unwrap();
    assert!(!reloaded.current(&fixture.assets));
}

#[test]
fn native_vertex_byte_rounding_checks_all_polygon_vertices() {
    let mut fixture = fixture(false, true, false);
    for (vertex, red) in fixture.geometry.vertices.iter_mut().zip([0, 1, 2, 1]) {
        vertex.vertex.color = [red, 255, 255, 255];
    }
    let recipe = product(&fixture, 65536).unwrap();
    assert!(
        recipe
            .prepare(Refdef::default(), &fixture.evaluator)
            .is_some()
    );
    // Source colors form an affine field. Native CGEN_VERTEX truncation at
    // half identity light produces [0, 0, 1, 0], which needs separate triangles.
    assert!(
        recipe
            .prepare(
                Refdef {
                    identity_light: 0.5,
                    ..Refdef::default()
                },
                &fixture.evaluator,
            )
            .is_none()
    );
}

#[test]
fn invalid_roi_nonaffine_color_and_budget_have_scoped_fallbacks() {
    let mut fixture = fixture(false, true, false);
    fixture.geometry.vertices[3].vertex.color[0] = 19;
    assert!(product(&fixture, 65536).is_none());
    fixture.geometry.vertices[3].vertex.color[0] = 96;
    assert!(product(&fixture, 36).is_none());
    fixture.binding.lightmap_region = Some(AtlasRegion {
        page: 0,
        x: 3,
        y: 3,
        width: 2,
        height: 2,
    });
    assert!(product(&fixture, 65536).is_none());
    let mut sources = Vec::new();
    let mut factors = Vec::new();
    assert!(
        Recipe::load(
            &fixture.geometry,
            0,
            fixture.binding,
            fixture.assets.material(fixture.binding.material).unwrap(),
            &fixture.assets,
            &fixture.evaluator,
            &mut sources,
            &mut factors,
            65536
        )
        .unwrap()
        .is_none()
    );
    assert!(sources.is_empty());
    assert!(factors.is_empty());
}
