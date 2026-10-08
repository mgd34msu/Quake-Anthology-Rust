use qa_render::lightmap::{
    AtlasBuilder, LightmapError, PAGE_SIZE, Q1GlScale, build_quake_rgb, build_quake2_rgb,
    shift_quake3_rgb,
};

fn pixels(width: u32, height: u32, value: u8) -> Vec<u8> {
    vec![value; width as usize * height as usize * 3]
}

#[test]
fn native_columns_choose_leftmost_lowest_fit_then_stack() {
    let mut builder = AtlasBuilder::load(PAGE_SIZE, 2).unwrap();
    let grid = pixels(40, 40, 17);
    let first = builder.insert(40, 40, &grid).unwrap();
    let second = builder.insert(40, 40, &grid).unwrap();
    let third = builder.insert(40, 40, &grid).unwrap();
    let fourth = builder.insert(40, 40, &grid).unwrap();
    assert_eq!((first.page, first.x, first.y), (0, 0, 0));
    assert_eq!((second.page, second.x, second.y), (0, 40, 0));
    assert_eq!((third.page, third.x, third.y), (0, 80, 0));
    assert_eq!((fourth.page, fourth.x, fourth.y), (0, 0, 40));
    let atlas = builder.finish();
    assert_eq!(atlas.page_count(), 1);
    assert_eq!(atlas.page_size(), 128);
    assert_eq!(atlas.page(0).unwrap()[0], 17);
    assert_eq!(atlas.page(0).unwrap()[127 * 3], 0);
}

#[test]
fn native_rightmost_column_is_excluded_and_page_cap_is_scoped() {
    let mut builder = AtlasBuilder::load(128, 1).unwrap();
    builder.insert(127, 128, &pixels(127, 128, 1)).unwrap();
    assert_eq!(
        builder.insert(1, 128, &pixels(1, 128, 2)),
        Err(LightmapError::PageLimit)
    );
    assert_eq!(
        builder.insert(128, 1, &pixels(128, 1, 3)),
        Err(LightmapError::Dimensions)
    );
    assert_eq!(builder.finish().page_count(), 1);

    let mut builder = AtlasBuilder::load(128, 2).unwrap();
    builder.insert(127, 128, &pixels(127, 128, 1)).unwrap();
    let region = builder.insert(1, 128, &pixels(1, 128, 2)).unwrap();
    assert_eq!((region.page, region.x, region.y), (1, 0, 0));
}

#[test]
fn imported_q3_pages_stay_intact_and_grids_share_the_collection() {
    let mut builder = AtlasBuilder::load(128, 3).unwrap();
    let source: Vec<u8> = (0..128 * 128 * 3).map(|n| (n % 251) as u8).collect();
    let page = builder.insert_page(&source).unwrap();
    assert_eq!(
        (page.page, page.x, page.y, page.width, page.height),
        (0, 0, 0, 128, 128)
    );
    let grid = builder
        .insert(
            3,
            2,
            &[
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
            ],
        )
        .unwrap();
    assert_eq!((grid.page, grid.x, grid.y), (1, 0, 0));
    let atlas = builder.finish();
    assert_eq!(atlas.page(0).unwrap(), source);
    assert_eq!(&atlas.page(1).unwrap()[..9], &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(
        &atlas.page(1).unwrap()[128 * 3..128 * 3 + 9],
        &[10, 11, 12, 13, 14, 15, 16, 17, 18]
    );
    assert_eq!(grid.uv_at_sample([0.0, 0.0]), [0.5 / 128.0; 2]);
    assert_eq!(
        grid.uv_from_texture([48.0, 16.0], [48, 16]),
        [0.5 / 128.0; 2]
    );
    assert_eq!(
        grid.uv_from_texture([64.0, 32.0], [48, 16]),
        [1.5 / 128.0; 2]
    );
    assert_eq!(grid.uv_at_sample([2.0, 1.0]), [2.5 / 128.0, 1.5 / 128.0]);
}

#[test]
fn original_glquake_shift_is_explicit_and_styles_accumulate_before_clamping() {
    assert_eq!(
        build_quake_rgb(1, 1, &[64; 3], &[256], Q1GlScale::OriginalOverbright).unwrap(),
        [128; 3]
    );
    assert_eq!(
        build_quake_rgb(1, 1, &[64; 3], &[256], Q1GlScale::Standard).unwrap(),
        [64; 3]
    );
    assert_eq!(
        build_quake_rgb(
            1,
            1,
            &[32, 16, 8, 32, 16, 8],
            &[128, 256],
            Q1GlScale::OriginalOverbright
        )
        .unwrap(),
        [96, 48, 24]
    );
    assert_eq!(
        build_quake_rgb(1, 1, &[255; 3], &[256], Q1GlScale::OriginalOverbright).unwrap(),
        [255; 3]
    );
}

#[test]
fn ref_gl_truncates_then_proportionally_normalizes_rgb_after_styles() {
    assert_eq!(
        build_quake2_rgb(1, 1, &[200, 100, 50], &[[1.0; 3]], 3.0).unwrap(),
        [255, 127, 63]
    );
    assert_eq!(
        build_quake2_rgb(1, 1, &[101, 3, 1], &[[0.5; 3]], 1.0).unwrap(),
        [50, 1, 0]
    );
    assert_eq!(
        build_quake2_rgb(
            1,
            1,
            &[20, 40, 80, 10, 20, 40],
            &[[1.0, 0.5, 0.25], [0.5, 1.0, 2.0]],
            2.0
        )
        .unwrap(),
        [50, 80, 200]
    );
    assert_eq!(
        build_quake2_rgb(1, 1, &[20; 3], &[[-1.0, 1.0, 2.0]], 1.0).unwrap(),
        [0, 20, 40]
    );
}

#[test]
fn q3_overbright_shifts_and_preserves_color_ratios() {
    assert_eq!(
        shift_quake3_rgb(&[200, 100, 50], 2, 1).unwrap(),
        [255, 127, 63]
    );
    assert_eq!(
        shift_quake3_rgb(&[10, 20, 30], 2, 0).unwrap(),
        [40, 80, 120]
    );
    assert_eq!(
        shift_quake3_rgb(&[200, 100, 50], 2, 2).unwrap(),
        [200, 100, 50]
    );
    assert_eq!(shift_quake3_rgb(&[0; 3], 2, 0).unwrap(), [0; 3]);
}

#[test]
fn dimensions_style_lengths_and_nonfinite_scalars_fail_before_page_mutation() {
    assert!(matches!(
        AtlasBuilder::load(256, 1),
        Err(LightmapError::PageSize)
    ));
    assert!(matches!(
        AtlasBuilder::load(128, 0),
        Err(LightmapError::PageLimit)
    ));
    let mut builder = AtlasBuilder::load(128, 1).unwrap();
    assert_eq!(
        builder.insert(1, 1, &[1, 2]),
        Err(LightmapError::SampleLength {
            expected: 3,
            actual: 2
        })
    );
    assert!(builder.insert_page(&[1, 2, 3]).is_err());
    assert!(builder.insert(0, 1, &[]).is_err());
    assert_eq!(builder.finish().page_count(), 0);
    assert!(build_quake_rgb(1, 1, &[1, 2, 3], &[256, 256], Q1GlScale::OriginalOverbright).is_err());
    assert!(build_quake2_rgb(1, 1, &[1, 2], &[[1.0; 3]], 1.0).is_err());
    assert_eq!(
        build_quake2_rgb(1, 1, &[1, 2, 3], &[[f32::NAN; 3]], 1.0),
        Err(LightmapError::Scalar)
    );
    assert_eq!(
        build_quake2_rgb(1, 1, &[1, 2, 3], &[[1.0; 3]], f32::INFINITY),
        Err(LightmapError::Scalar)
    );
    assert!(shift_quake3_rgb(&[1, 2], 2, 0).is_err());
    assert_eq!(
        shift_quake3_rgb(&[1, 2, 3], 0, 1),
        Err(LightmapError::Overbright)
    );
    assert_eq!(
        shift_quake3_rgb(&[1, 2, 3], 9, 0),
        Err(LightmapError::Overbright)
    );
}
