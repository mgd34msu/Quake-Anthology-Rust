use qa_render::surface_cache::{
    BuildState, IndexedLighting, IndexedTexture, LightGrid, PaletteLighting, SurfaceCache,
    SurfaceSource,
};

fn palette(grade_rows: bool, first_fullbright: u16) -> PaletteLighting {
    let rgb: Vec<_> = (0..256u16).flat_map(|index| [index as u8; 3]).collect();
    let colormap: Vec<_> = (0..64u8)
        .flat_map(|grade| {
            (0..256u16).map(move |index| if grade_rows { grade } else { index as u8 })
        })
        .collect();
    PaletteLighting::load(&rgb, &colormap, None, first_fullbright).unwrap()
}

fn texture(index: u8, cutout: bool) -> IndexedTexture {
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![index; (16 >> mip) * (16 >> mip)]);
    IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), cutout).unwrap()
}

fn gray_surface(samples: &[u8], styles: [u8; 4]) -> SurfaceSource {
    SurfaceSource::load(
        [0; 2],
        [16; 2],
        Some(LightGrid::gray(2, 2, styles, samples).unwrap()),
        true,
    )
    .unwrap()
}

#[test]
fn native_constant_light_style_and_dynamic_invalidation() {
    let palette = palette(true, 256);
    let texture = texture(17, false);
    let source = gray_surface(&[64; 4], [0, 255, 255, 255]);
    let mut cache = SurfaceCache::load(vec![source], 256).unwrap();
    let original = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(
        cache
            .pixels(original)
            .unwrap()
            .iter()
            .all(|&pixel| pixel == 47)
    );
    let hit = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(cache.pixels(hit).is_some());
    assert_eq!(cache.stats().hits, 1);
    let half = cache
        .prepare(
            0,
            0,
            &texture,
            &palette,
            BuildState {
                style_scales: [128, 256, 256, 256],
                ..BuildState::default()
            },
        )
        .unwrap();
    assert!(cache.pixels(half).unwrap().iter().all(|&pixel| pixel == 55));
    assert!(cache.pixels(original).is_none());
    let dynamic = [64 * 256; 4];
    let bright = cache
        .prepare(
            0,
            0,
            &texture,
            &palette,
            BuildState {
                dynamic: Some(&dynamic),
                dynamic_generation: 1,
                ..BuildState::default()
            },
        )
        .unwrap();
    assert!(
        cache
            .pixels(bright)
            .unwrap()
            .iter()
            .all(|&pixel| pixel == 31)
    );
    assert_eq!(cache.stats().fills, 3);
    assert_eq!(cache.stats().evictions, 0);
}

#[test]
fn native_right_to_left_fixed_interpolation_at_four_mips() {
    let palette = palette(true, 256);
    let texture = texture(17, false);
    let source = gray_surface(&[0, 64, 128, 255], [0, 255, 255, 255]);
    let mut cache = SurfaceCache::load(vec![source], 512).unwrap();
    // WinQuake R_DrawSurfaceBlock8_mip0..3 starts each row at the right
    // lightmap corner and walks backwards. These are its native first-row
    // colormap grades, including the one-texel horizontal sampling bias.
    for (mip, left_grade) in [62, 61, 59, 55].into_iter().enumerate() {
        let span = cache
            .prepare(0, mip as u8, &texture, &palette, BuildState::default())
            .unwrap();
        let pixels = cache.pixels(span).unwrap();
        assert_eq!(pixels[0], left_grade);
        assert_eq!(pixels[span.width as usize - 1], 47);
        if mip == 0 {
            assert_eq!(pixels[8 * 16], 46);
            assert_eq!(pixels[8 * 16 + 15], 24);
            assert_eq!(pixels[15 * 16], 31);
            assert_eq!(pixels[15 * 16 + 15], 3);
        }
    }
}

#[test]
fn multiple_style_planes_accumulate_before_six_bit_inversion() {
    let palette = palette(true, 256);
    let texture = texture(17, false);
    let source = gray_surface(&[64, 64, 64, 64, 32, 32, 32, 32], [0, 1, 255, 255]);
    let mut cache = SurfaceCache::load(vec![source], 256).unwrap();
    let span = cache
        .prepare(
            0,
            0,
            &texture,
            &palette,
            BuildState {
                style_scales: [256, 512, 0, 0],
                ..BuildState::default()
            },
        )
        .unwrap();
    assert!(cache.pixels(span).unwrap().iter().all(|&pixel| pixel == 31));
}

#[test]
fn owner_brightest_rgb_policy_preserves_red_green_ties() {
    let palette = palette(true, 256);
    let texture = texture(17, false);
    let grid = LightGrid::rgb(
        2,
        2,
        [0, 255, 255, 255],
        &[100, 100, 20, 100, 100, 20, 100, 100, 20, 100, 100, 20],
    )
    .unwrap();
    let source = SurfaceSource::load([0; 2], [16; 2], Some(grid), true).unwrap();
    let mut cache = SurfaceCache::load(vec![source], 256).unwrap();
    let span = cache
        .prepare(
            0,
            0,
            &texture,
            &palette,
            BuildState {
                lighting: IndexedLighting::BrightestRgb,
                ..BuildState::default()
            },
        )
        .unwrap();
    // Literal native Q2 strict comparisons would choose B=20 and grade58.
    // The owner's explicit true maximum uses100 and grade38 instead.
    assert!(cache.pixels(span).unwrap().iter().all(|&pixel| pixel == 38));
}

#[test]
fn page_subregion_does_not_include_adjacent_atlas_samples() {
    let page = [
        1, 2, 3, 10, 20, 30, 40, 50, 60, 4, 5, 6, 7, 8, 9, 70, 80, 90, 100, 110, 120, 11, 12, 13,
    ];
    let grid = LightGrid::rgb_page(4, 2, &page, [1, 0], [2, 2], 0).unwrap();
    assert_eq!(grid.dimensions(), [2, 2]);
    assert_eq!(grid.styles(), [0, 255, 255, 255]);
    assert_eq!(
        grid.samples(),
        &[[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]]
    );
}

#[test]
fn fence_cutouts_survive_colormap_lookup_in_every_original_mip() {
    let palette = palette(true, 256);
    let mut mips: [Vec<u8>; 4] = std::array::from_fn(|mip| vec![17; (16 >> mip) * (16 >> mip)]);
    for level in &mut mips {
        level[0] = 255;
    }
    let texture = IndexedTexture::load(16, 16, mips.each_ref().map(Vec::as_slice), true).unwrap();
    let source = gray_surface(&[0; 4], [0, 255, 255, 255]);
    let mut cache = SurfaceCache::load(vec![source], 512).unwrap();
    for mip in 0..4 {
        let span = cache
            .prepare(0, mip, &texture, &palette, BuildState::default())
            .unwrap();
        let pixels = cache.pixels(span).unwrap();
        assert!(span.cutout);
        assert_eq!(pixels[0], 255);
        assert!(pixels[1..].iter().all(|&pixel| pixel == 63));
    }
}

#[test]
fn supplied_colormap_and_negative_texture_minima_wrap_exactly() {
    let palette = palette(false, 224);
    let mips: [Vec<u8>; 4] = std::array::from_fn(|mip| {
        let side = 32 >> mip;
        (0..side * side).map(|pixel| (pixel % 256) as u8).collect()
    });
    let texture = IndexedTexture::load(32, 32, mips.each_ref().map(Vec::as_slice), false).unwrap();
    let source = SurfaceSource::load([-16, 48], [16; 2], None, false).unwrap();
    let mut cache = SurfaceCache::load(vec![source], 512).unwrap();
    for mip in 0..4 {
        let span = cache
            .prepare(0, mip, &texture, &palette, BuildState::default())
            .unwrap();
        let pixels = cache.pixels(span).unwrap();
        let width = 32 >> mip;
        let first = ((16 >> mip) * width + (16 >> mip)) % 256;
        assert_eq!(pixels[0], first as u8);
    }
    let bright_texture = self::texture(230, false);
    let dark_source = gray_surface(&[0; 4], [0, 255, 255, 255]);
    let mut cache = SurfaceCache::load(vec![dark_source], 256).unwrap();
    let nonidentity_palette = self::palette(true, 224);
    let span = cache
        .prepare(
            0,
            0,
            &bright_texture,
            &nonidentity_palette,
            BuildState::default(),
        )
        .unwrap();
    assert!(cache.pixels(span).unwrap().iter().all(|&pixel| pixel == 63));
}

#[test]
fn native_fullbright_identity_rows_preserve_indices_at_all_grades() {
    let rgb: Vec<_> = (0..256u16).flat_map(|index| [index as u8; 3]).collect();
    let colormap: Vec<_> = (0..64u8)
        .flat_map(|grade| {
            (0..256u16).map(move |index| if index >= 224 { index as u8 } else { grade })
        })
        .collect();
    let palette = PaletteLighting::load(&rgb, &colormap, None, 224).unwrap();
    let bright_texture = texture(230, false);
    let dark_source = gray_surface(&[0; 4], [0, 255, 255, 255]);
    let mut cache = SurfaceCache::load(vec![dark_source], 256).unwrap();
    for grade in 0..64u8 {
        let span = cache
            .prepare(
                0,
                0,
                &bright_texture,
                &palette,
                BuildState {
                    ambient: (63 - grade) * 4,
                    ..BuildState::default()
                },
            )
            .unwrap();
        assert!(
            cache
                .pixels(span)
                .unwrap()
                .iter()
                .all(|&pixel| pixel == 230)
        );
    }
}

#[test]
fn rover_coalesces_and_invalidates_each_evicted_owner() {
    let palette = palette(true, 256);
    let texture = texture(17, false);
    let sources = vec![
        SurfaceSource::load([0; 2], [16; 2], None, false).unwrap(),
        SurfaceSource::load([0; 2], [16; 2], None, false).unwrap(),
        SurfaceSource::load([0; 2], [32, 16], None, false).unwrap(),
    ];
    let mut cache = SurfaceCache::load(sources, 512).unwrap();
    let a = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let b = cache
        .prepare(1, 0, &texture, &palette, BuildState::default())
        .unwrap();
    let c = cache
        .prepare(2, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(cache.pixels(a).is_none());
    assert!(cache.pixels(b).is_none());
    assert_eq!(cache.pixels(c).unwrap().len(), 512);
    assert_eq!(cache.stats().evictions, 2);
    let replacement = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(cache.pixels(c).is_none());
    assert_eq!(cache.pixels(replacement).unwrap().len(), 256);
    assert_eq!(cache.stats().evictions, 3);
}

#[test]
fn pending_span_pins_block_eviction_and_changed_stamp_refill() {
    let palette = palette(true, 256);
    let texture = texture(17, false);
    let sources = vec![
        gray_surface(&[64; 4], [0, 255, 255, 255]),
        gray_surface(&[64; 4], [0, 255, 255, 255]),
    ];
    let mut cache = SurfaceCache::load(sources, 256).unwrap();
    assert!(cache.begin_batch());
    let held = cache
        .prepare(0, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(
        cache
            .prepare(1, 0, &texture, &palette, BuildState::default())
            .is_none()
    );
    assert!(
        cache
            .prepare(
                0,
                0,
                &texture,
                &palette,
                BuildState {
                    fullbright: true,
                    ..BuildState::default()
                }
            )
            .is_none()
    );
    assert!(cache.pixels(held).unwrap().iter().all(|&pixel| pixel == 47));
    cache.end_batch();
    let next = cache
        .prepare(1, 0, &texture, &palette, BuildState::default())
        .unwrap();
    assert!(cache.pixels(held).is_none());
    assert!(cache.pixels(next).is_some());
    assert_eq!(cache.stats().rejected, 2);
}

#[test]
fn numeric_resource_and_cache_owner_ids_prevent_cross_world_aliases() {
    let palette = palette(false, 256);
    let texture_a = texture(17, false);
    let texture_b = texture(18, false);
    let source = || SurfaceSource::load([0; 2], [16; 2], None, false).unwrap();
    let mut cache_a = SurfaceCache::load(vec![source()], 256).unwrap();
    let mut cache_b = SurfaceCache::load(vec![source()], 256).unwrap();
    let span_a = cache_a
        .prepare(0, 0, &texture_a, &palette, BuildState::default())
        .unwrap();
    let span_b = cache_b
        .prepare(0, 0, &texture_a, &palette, BuildState::default())
        .unwrap();
    assert!(cache_b.pixels(span_a).is_none());
    assert!(cache_a.pixels(span_b).is_none());
    let changed = cache_a
        .prepare(0, 0, &texture_b, &palette, BuildState::default())
        .unwrap();
    assert!(
        cache_a
            .pixels(changed)
            .unwrap()
            .iter()
            .all(|&pixel| pixel == 18)
    );
    assert_eq!(cache_a.stats().fills, 2);
}

#[test]
fn malformed_load_inputs_are_scoped_errors() {
    assert!(PaletteLighting::load(&[0; 768], &[0; 16384], Some(&[0; 1]), 256).is_err());
    assert!(IndexedTexture::load(16, 16, [&[]; 4], false).is_err());
    assert!(LightGrid::gray(2, 2, [0, 255, 1, 255], &[0; 8]).is_err());
    assert!(SurfaceSource::load([0; 2], [17, 16], None, false).is_err());
    assert!(
        SurfaceSource::load(
            [0; 2],
            [16; 2],
            Some(LightGrid::gray(3, 2, [0, 255, 255, 255], &[0; 6]).unwrap()),
            true,
        )
        .is_err()
    );
}
