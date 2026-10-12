use super::*;
use qa_render::surface_cache::IndexedProjection;

const GRID: [usize; 2] = [17, 7];
const MINIMUM: [i32; 2] = [-16, 48];
const PROJECTION: [[f64; 3]; 2] = [[0.5, 0.0, 8.0], [0.0, 0.375, -18.0]];

fn projected_source(samples: &[u8]) -> SurfaceSource {
    SurfaceSource::load(
        MINIMUM,
        [32, 16],
        Some(LightGrid::gray(17, 7, [0, 1, 255, 255], samples).unwrap()),
        true,
        Some(IndexedProjection {
            lightmap: Some(PROJECTION),
            texture: None,
        }),
    )
    .unwrap()
}

fn samples() -> Vec<u8> {
    (0..2 * GRID[0] * GRID[1])
        .map(|index| ((index * 43 + index / GRID[0] * 29) % 170) as u8)
        .collect()
}

fn original_scalar(samples: &[u8], dynamic: &[i32], scales: [u16; 2]) -> Vec<i32> {
    let cells = GRID[0] * GRID[1];
    (0..cells)
        .map(|cell| {
            let value = (4_i64 << 8)
                + i64::from(samples[cell]) * i64::from(scales[0])
                + i64::from(samples[cells + cell]) * i64::from(scales[1])
                + i64::from(dynamic[cell]);
            ((255 * 256 - value.max(0)) >> 2).max(64) as i32
        })
        .collect()
}

fn expected_grade(lights: &[i32], x: usize, y: usize, mip: u8) -> u8 {
    let step = f64::from(1u32 << mip);
    let s = (0.5 * (f64::from(MINIMUM[0]) + (x as f64 + 0.5) * step) + 8.0)
        .clamp(0.0, (GRID[0] - 1) as f64);
    let t = (0.375 * (f64::from(MINIMUM[1]) + (y as f64 + 0.5) * step) - 18.0)
        .clamp(0.0, (GRID[1] - 1) as f64);
    let ix = s.floor() as usize;
    let iy = t.floor() as usize;
    let jx = (ix + 1).min(GRID[0] - 1);
    let jy = (iy + 1).min(GRID[1] - 1);
    let u = s - ix as f64;
    let v = t - iy as f64;
    // Independent four-corner weights, without the production fit or lerp.
    let light = f64::from(lights[iy * GRID[0] + ix]) * (1.0 - u) * (1.0 - v)
        + f64::from(lights[iy * GRID[0] + jx]) * u * (1.0 - v)
        + f64::from(lights[jy * GRID[0] + ix]) * (1.0 - u) * v
        + f64::from(lights[jy * GRID[0] + jx]) * u * v;
    ((light as usize & 0xff00) >> 8) as u8
}

#[test]
fn projected_cache_uses_actual_grid_at_each_mip_without_coarse_resampling() {
    let samples = samples();
    let cells = GRID[0] * GRID[1];
    let dynamic: Vec<_> = (0..cells).map(|i| (i as i32 % 11 - 5) * 256).collect();
    let mut mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
        let side = 32 >> mip;
        (0..side * side).map(|i| (i % 220 + 1) as u8).collect()
    });
    for (mip, pixels) in mips.iter_mut().enumerate() {
        let side = 32 >> mip;
        let x = (MINIMUM[0] >> mip).rem_euclid(side as i32) as usize;
        let y = (MINIMUM[1] >> mip).rem_euclid(side as i32) as usize;
        pixels[y * side + x] = 255;
    }
    let texture = IndexedTexture::load(32, 32, mips.each_ref().map(Vec::as_slice), true).unwrap();
    let palette = palette(true, 256);
    let mut cache = SurfaceCache::load(vec![projected_source(&samples)], 1024).unwrap();
    for scales in [[256, 128], [128, 256]] {
        let lights = original_scalar(&samples, &dynamic, scales);
        for mip in 0..4 {
            let block = cache
                .prepare(
                    0,
                    mip,
                    &texture,
                    &palette,
                    BuildState {
                        ambient: 4,
                        style_scales: [scales[0], scales[1], 0, 0],
                        dynamic: Some(&dynamic),
                        dynamic_generation: 1,
                        ..BuildState::default()
                    },
                )
                .unwrap();
            let pixels = cache.indexed_pixels(block).unwrap();
            assert_eq!(block.width, 32 >> mip);
            assert_eq!(block.height, 16 >> mip);
            let level = texture.mip(mip).unwrap();
            let tx = (MINIMUM[0] >> mip).rem_euclid(level.width as i32) as usize;
            let ty = (MINIMUM[1] >> mip).rem_euclid(level.height as i32) as usize;
            for y in 0..block.height as usize {
                for x in 0..block.width as usize {
                    let texel = level.indices()[((ty + y) % level.height as usize)
                        * level.width as usize
                        + (tx + x) % level.width as usize];
                    let expected = if texel == 255 {
                        255
                    } else {
                        expected_grade(&lights, x, y, mip)
                    };
                    assert_eq!(pixels[y * block.width as usize + x], expected);
                }
            }
            assert_eq!(pixels[0], 255);
        }
    }
    assert_eq!(cache.stats().fills, 8);
    // A classic 32x16 surface expects six cells; this actual projected grid
    // expects 119. The wrong dynamic shape is rejected, never truncated.
    assert!(
        cache
            .prepare(
                0,
                0,
                &texture,
                &palette,
                BuildState {
                    dynamic: Some(&[0; 6]),
                    ..BuildState::default()
                },
            )
            .is_none()
    );
}

#[test]
fn projected_single_cell_keeps_native_scalar_ties_and_style_invalidation() {
    let source = SurfaceSource::load(
        [0; 2],
        [16; 2],
        Some(LightGrid::rgb(1, 1, [0, 255, 255, 255], &[100, 100, 20]).unwrap()),
        true,
        Some(IndexedProjection {
            lightmap: Some([[17.0, -8.0, 100.0], [-23.0, 4.0, -200.0]]),
            texture: None,
        }),
    )
    .unwrap();
    let mut cache = SurfaceCache::load(vec![source], 512).unwrap();
    let texture = texture(17, false);
    let palette = palette(true, 256);
    for (lighting, scales, grade) in [
        (IndexedLighting::NativeRgb, 256, 58),
        (IndexedLighting::BrightestRgb, 256, 38),
        (IndexedLighting::BrightestRgb, 128, 51),
    ] {
        let state = BuildState {
            lighting,
            style_scales: [scales, 0, 0, 0],
            ..BuildState::default()
        };
        let block = cache.prepare(0, 0, &texture, &palette, state).unwrap();
        assert!(
            cache
                .indexed_pixels(block)
                .unwrap()
                .iter()
                .all(|&v| v == grade)
        );
        let block = cache.prepare(0, 0, &texture, &palette, state).unwrap();
        assert!(cache.indexed_pixels(block).is_some());
    }
    assert_eq!(cache.stats().fills, 3);
    assert_eq!(cache.stats().hits, 3);
}

#[test]
fn projected_load_validation_and_classic_absent_samples_keep_native_darkness() {
    let projection = Some(IndexedProjection {
        lightmap: Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        texture: None,
    });
    assert!(SurfaceSource::load([0; 2], [16; 2], None, true, projection).is_err());
    assert!(
        SurfaceSource::load(
            [0; 2],
            [16; 2],
            Some(LightGrid::gray(1, 1, [0, 255, 255, 255], &[64]).unwrap()),
            true,
            Some(IndexedProjection {
                lightmap: Some([[f64::NAN, 0.0, 0.0], [0.0; 3]]),
                texture: None
            }),
        )
        .is_err()
    );
    let source = SurfaceSource::load([0; 2], [16; 2], None, true, None).unwrap();
    let mut cache = SurfaceCache::load(vec![source], 256).unwrap();
    let texture = texture(17, false);
    let palette = palette(true, 256);
    let block = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(
        cache
            .indexed_pixels(block)
            .unwrap()
            .iter()
            .all(|&v| v == 63)
    );
}

#[test]
fn alternate_chart_samples_original_texture_mips_without_inventing_light_samples() {
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
        let side = 32 >> mip;
        (0..side * side)
            .map(|index| (index % side + index / side * 7 + 1) as u8)
            .collect()
    });
    let texture = IndexedTexture::load(32, 32, mips.each_ref().map(Vec::as_slice), false).unwrap();
    let mapping = IndexedProjection {
        lightmap: None,
        texture: Some([[0.0, 0.0, 16.0], [0.0, 1.0, 0.0]]),
    };
    for (world_has_lightdata, fullbright, expected_grade) in
        [(true, false, 63), (true, true, 0), (false, false, 0)]
    {
        for grade_rows in [false, true] {
            let source =
                SurfaceSource::load([0; 2], [32; 2], None, world_has_lightdata, Some(mapping))
                    .unwrap();
            let mut cache = SurfaceCache::load(vec![source], 2048).unwrap();
            let palette = palette(grade_rows, 256);
            for mip in 0..4 {
                let block = cache
                    .prepare(
                        0,
                        mip,
                        &texture,
                        &palette,
                        BuildState {
                            fullbright,
                            ..BuildState::default()
                        },
                    )
                    .unwrap();
                let pixels = cache.indexed_pixels(block).unwrap();
                let side = (32 >> mip) as usize;
                let source_x = (16 >> mip) as usize;
                for row in 0..side {
                    let expected = if grade_rows {
                        expected_grade
                    } else {
                        mips[mip as usize][row * side + source_x]
                    };
                    assert!(
                        pixels[row * side..(row + 1) * side]
                            .iter()
                            .all(|&p| p == expected)
                    );
                }
            }
            assert_eq!(cache.stats().fills, 4);
        }
    }
}
