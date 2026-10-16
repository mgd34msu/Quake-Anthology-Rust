use super::*;
use crate::assets::{Flow, SoftwareTiled, TcMod, UvWarp};
use crate::surface_cache::{IndexedLighting, IndexedTexture, PaletteLighting};

#[expect(
    clippy::approx_constant,
    reason = "Original software span oracle preserves the qsrc sine initialization literal."
)]
fn original_row(
    planes: Planes,
    adjust: [i64; 2],
    start: u32,
    count: u32,
    y: u32,
    tile: SoftwareTiled,
    time_ms: i64,
    pixels: &[u8],
) -> Vec<u8> {
    let time = if tile.time_f32 {
        f64::from((time_ms as f64 * 0.001) as f32)
    } else {
        time_ms as f64 / 1000.0
    };
    let phase = if tile.time_f32 {
        ((time as f32 * 20.0) as i32 & 127) as usize
    } else {
        ((time * 20.0) as i32 & 127) as usize
    };
    let table: [i32; 256] = std::array::from_fn(|i| {
        (8.0 * 65536.0 + (i as f64 * 3.14159 * 2.0 / 128.0).sin() * (8.0 * 65536.0)) as i32
    });
    let mut adjust = adjust;
    if let Some(speed) = tile.flow_speed {
        let cycle = time * speed;
        adjust[0] =
            (adjust[0] as f64 + 65536.0 * (-128.0 * (cycle - f64::from(cycle as i32)))) as i64;
    }
    let mut zi = planes.inverse_depth.at(start as f32, y as f32);
    let mut divided = planes.texture.map(|p| p.at(start as f32, y as f32));
    let endpoint = |divided: [f32; 2], zi: f32, minimum: i64| {
        let z = 65536.0 / zi;
        std::array::from_fn::<_, 2, _>(|i| {
            ((divided[i] * z) as i64 + adjust[i]).clamp(minimum, (16384_i64 << 16) - 1)
        })
    };
    let mut current = endpoint(divided, zi, 0);
    let mut count = count;
    let mut result = Vec::new();
    while count != 0 {
        let n = count.min(16);
        count -= n;
        let distance = if count != 0 { 16 } else { n - 1 } as f32;
        divided[0] += planes.texture[0].x * distance;
        divided[1] += planes.texture[1].x * distance;
        zi += planes.inverse_depth.x * distance;
        let next = endpoint(divided, zi, 16);
        let step = if count != 0 {
            std::array::from_fn(|i| (next[i] - current[i]) >> 4)
        } else if n > 1 {
            std::array::from_fn(|i| (next[i] - current[i]) / i64::from(n - 1))
        } else {
            [0; 2]
        };
        current = current.map(|v| v & ((128 << 16) - 1));
        for _ in 0..n {
            let [s, t] = if tile.turbulent {
                [
                    current[0] + i64::from(table[phase + ((current[1] >> 16) & 127) as usize]),
                    current[1] + i64::from(table[phase + ((current[0] >> 16) & 127) as usize]),
                ]
            } else {
                current
            };
            result.push(pixels[(((t >> 16) & 63) * 64 + ((s >> 16) & 63)) as usize]);
            current[0] += step[0];
            current[1] += step[1];
        }
        current = next;
    }
    result
}

#[test]
fn opaque_tiled_world_draws_raw_native_indices_with_depth_and_no_cache_fill() {
    for tile in [
        SoftwareTiled {
            turbulent: true,
            flow_speed: None,
            time_f32: false,
        },
        SoftwareTiled {
            turbulent: true,
            flow_speed: None,
            time_f32: true,
        },
        SoftwareTiled {
            turbulent: false,
            flow_speed: Some(0.77),
            time_f32: true,
        },
        SoftwareTiled {
            turbulent: true,
            flow_speed: Some(0.25),
            time_f32: true,
        },
    ] {
        let mut assets = Assets::load().unwrap();
        let colors: Vec<_> = (0..=255_u8).flat_map(|c| [c, c, c]).collect();
        // Any accidental cached lighting would erase the patterned raw texels.
        let colormap = vec![0; 64 * 256];
        let palette = assets
            .register_palette(PaletteLighting::load(&colors, &colormap, None, 224).unwrap())
            .unwrap();
        let pixels: Vec<_> = (0..64 * 64)
            .map(|i| ((i % 64) + (i / 64) * 17) as u8)
            .collect();
        let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
            if mip == 0 {
                pixels.clone()
            } else {
                vec![0; (64 >> mip) * (64 >> mip)]
            }
        });
        let image = assets
            .register_indexed_image(
                IndexedTexture::load(64, 64, mips.each_ref().map(Vec::as_slice), false).unwrap(),
                palette,
            )
            .unwrap();
        let mut base = Stage {
            texture: StageTexture::Image(image),
            ..Stage::default()
        };
        let mut slot = 0;
        if tile.turbulent {
            base.tcmods[slot] = Some(TcMod::Warp(UvWarp {
                texel_scale: [64.0; 2],
                amplitude: [0.125; 2],
                frequency: 0.125,
                time_scale: 1.0,
            }));
            slot += 1;
        }
        if tile.flow_speed.is_some() {
            base.tcmods[slot] = Some(TcMod::Flow(Flow {
                speed: 1.0 / 40.0,
                amplitude: [-64.0, 0.0],
                cycle_start: [-64.0, 0.0],
            }));
        }
        let material = {
            let material = assets
                .register_material(
                    "opaque tiled",
                    &[base],
                    MaterialSettings {
                        cull: Cull::None,
                        software_tiled: Some(tile),
                        ..MaterialSettings::default()
                    },
                )
                .unwrap();
            assets
                .register_shader_binding(crate::ShaderBinding::material(
                    material,
                    crate::world::geometry::LightSource::TwoDimensional,
                ))
                .unwrap()
        };
        let world = {
            let shader_input_0 = *assets.shader_binding(material).unwrap();
            fixture_world(
                &mut assets,
                4.0,
                1.0,
                SurfaceMaterial {
                    shader: shader_input_0,
                    ..SurfaceMaterial::default()
                },
                Some(0),
                GeometryPartition::SplitBsp,
                |geometry| {
                    // Native tiled bounds are independent of retained geometric/cache extents.
                    geometry.surfaces[0].texture_minima = [96, -112];
                    geometry.surfaces[0].texture_extents = [16384; 2];
                    geometry.surfaces[0].texture_projection[0][3] = 113.375;
                    geometry.surfaces[0].texture_projection[1][3] = -57.875;
                },
            )
        };
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
        let mut changed = false;
        let mut previous = None;
        for time_ms in [0, 1, 49, 50, 6350, 6400, 16777299] {
            let view = Refdef {
                time_ms,
                cpu_presentation: CpuPresentation::Indexed {
                    palette,
                    lighting: IndexedLighting::NativeRgb,
                    ambient: 0,
                    fullbright: false,
                },
                ..test_view()
            };
            let stats = cpu.render(&packet(&assets, &[world], view), &assets);
            assert_eq!(stats.rejected, 0, "tile={tile:?} time={time_ms}");
            assert!(cpu.world_stats().indexed_pixels > 0);
            assert_eq!(cpu.world_stats().cache.fills, 0);
            let camera = Camera::load(view, 29, 19).unwrap();
            let surface = &assets.world(world).unwrap().geometry().surfaces[0];
            let plane = surface.plane.unwrap();
            let eye = plane.distance - plane.normal.dot(view.origin);
            let (planes, adjust) = native_planes(
                &camera,
                surface,
                surface.texture_projection,
                [-8192; 2],
                0,
                eye,
            )
            .unwrap();
            for y in view.viewport.y..view.viewport.y + view.viewport.height {
                let first = (view.viewport.x..view.viewport.x + view.viewport.width)
                    .find(|&x| cpu.frame.inverse_depth[(y * 29 + x) as usize] > 0.0);
                let Some(first) = first else {
                    continue;
                };
                let last = (first..view.viewport.x + view.viewport.width)
                    .rfind(|&x| cpu.frame.inverse_depth[(y * 29 + x) as usize] > 0.0)
                    .unwrap();
                let expected = original_row(
                    planes,
                    adjust,
                    first,
                    last + 1 - first,
                    y,
                    tile,
                    time_ms,
                    &pixels,
                );
                for (offset, &color) in expected.iter().enumerate() {
                    let index = (y * 29 + first) as usize + offset;
                    assert_eq!(
                        cpu.frame.indices[index],
                        color,
                        "tile={tile:?} time={time_ms} x={} y={y}",
                        first as usize + offset
                    );
                    assert_eq!(cpu.frame.palettes[index], palette.0);
                    assert_eq!(
                        cpu.frame.pixels[index],
                        u32::from_le_bytes([color, color, color, 255])
                    );
                    assert_eq!(
                        cpu.frame.inverse_depth[index].to_bits(),
                        planes
                            .inverse_depth
                            .at((first as usize + offset) as f32, y as f32)
                            .to_bits()
                    );
                }
            }
            let indices = cpu.frame.indices.clone();
            if let Some(previous) = previous.as_ref() {
                changed |= previous != &indices;
            }
            previous = Some(indices);
        }
        assert!(changed, "tile={tile:?}");
    }
}

#[test]
fn malformed_tiled_image_rejects_only_its_surface() {
    let mut assets = Assets::load().unwrap();
    let palette = indexed_palette(&mut assets);
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![17; (16 >> mip) * (16 >> mip)]);
    let image = assets
        .register_indexed_image(
            IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), false).unwrap(),
            palette,
        )
        .unwrap();
    let good = {
        let material = assets
            .register_material(
                "good",
                &[Stage {
                    texture: StageTexture::Image(image),
                    ..Stage::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    ..MaterialSettings::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(crate::ShaderBinding::material(
                material,
                crate::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let bad = {
        let material = assets
            .register_material(
                "bad",
                &[Stage {
                    texture: StageTexture::Image(image),
                    ..Stage::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    software_tiled: Some(SoftwareTiled {
                        turbulent: true,
                        flow_speed: None,
                        time_f32: false,
                    }),
                    ..MaterialSettings::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(crate::ShaderBinding::material(
                material,
                crate::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let bad = {
        let shader_input_0 = *assets.shader_binding(bad).unwrap();
        fixture_world(
            &mut assets,
            4.0,
            1.0,
            SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::SplitBsp,
            |_| {},
        )
    };
    let good = {
        let shader_input_0 = *assets.shader_binding(good).unwrap();
        fixture_world(
            &mut assets,
            8.0,
            1.0,
            SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            },
            None,
            GeometryPartition::SplitBsp,
            |_| {},
        )
    };
    let view = Refdef {
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: true,
        },
        ..test_view()
    };
    let mut cpu = CpuBackend::load_with_assets(29, 19, &assets).unwrap();
    let stats = cpu.render(&packet(&assets, &[bad, good], view), &assets);
    assert!(stats.rejected > 0);
    assert!(cpu.world_stats().indexed_pixels > 0);
    assert!(cpu.frame.indices.iter().any(|&index| index == 17));
}
