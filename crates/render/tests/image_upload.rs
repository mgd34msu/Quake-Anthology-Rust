use qa_render::assets::upload::{
    AlphaFringe, ColorOrder, ExtentRound, GammaCurve, MipmapBuild, ResizeFilter, RgbLut,
    UploadError, UploadExtent, UploadParams, expand_indexed, gamma_lut, light_scale, prepare_rgba,
};

fn gray(values: &[u8]) -> Vec<u8> {
    values.iter().flat_map(|&v| [v, v, v, 255]).collect()
}

fn channels(pixels: &[u8]) -> Vec<u8> {
    pixels.chunks_exact(4).map(|p| p[0]).collect()
}

#[test]
fn indexed_native_neighbor_order_and_row_boundary_quirks() {
    let palette = std::array::from_fn(|i| [i as u8, i as u8, i as u8, 255]);
    for (width, height, source, expected) in [
        (
            3,
            3,
            vec![2, 3, 4, 255, 255, 255, 5, 6, 7],
            vec![2, 3, 4, 5, 3, 4, 5, 6, 7],
        ),
        (3, 2, vec![255, 2, 3, 255, 255, 255], vec![2, 2, 3, 3, 2, 3]),
        (
            3,
            2,
            vec![255, 255, 255, 4, 255, 255],
            vec![4, 0, 4, 4, 4, 0],
        ),
        (1, 1, vec![255], vec![0]),
    ] {
        let original = source.clone();
        let pixels = expand_indexed(
            &source,
            width,
            height,
            &palette,
            Some(255),
            AlphaFringe::NativeNeighbors,
        )
        .unwrap();
        assert_eq!(channels(&pixels), expected);
        for (&index, pixel) in source.iter().zip(pixels.chunks_exact(4)) {
            assert_eq!(pixel[3], if index == 255 { 0 } else { 255 });
        }
        assert_eq!(source, original);
    }
    let keep = expand_indexed(&[255], 1, 1, &palette, Some(255), AlphaFringe::KeepPalette).unwrap();
    assert_eq!(&*keep, [255, 255, 255, 0]);
    let opaque =
        expand_indexed(&[255], 1, 1, &palette, None, AlphaFringe::NativeNeighbors).unwrap();
    assert_eq!(&*opaque, [255; 4]);
}

#[test]
fn native_rounding_keeps_original_pixels_and_independent_legacy_caps() {
    let pixels = gray(&(0..15).collect::<Vec<_>>());
    let original = pixels.clone();
    for (round, expected) in [(ExtentRound::Up, (4, 8)), (ExtentRound::Down, (2, 4))] {
        let prepared = prepare_rgba(
            3,
            5,
            &pixels,
            UploadParams {
                extent: UploadExtent::PowerOfTwo {
                    round,
                    drop: 0,
                    max_dimension: 256,
                },
                ..UploadParams::default()
            },
        )
        .unwrap();
        assert_eq!(
            (prepared.levels[0].width, prepared.levels[0].height),
            expected
        );
    }
    assert_eq!(pixels, original);
    let capped = prepare_rgba(
        8,
        2,
        &gray(&[17; 16]),
        UploadParams {
            extent: UploadExtent::PowerOfTwo {
                round: ExtentRound::Up,
                drop: 0,
                max_dimension: 4,
            },
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!((capped.levels[0].width, capped.levels[0].height), (4, 2));
}

#[test]
fn asset_preparation_preserves_raw_rgba_disk_mips_mask_and_prior_success() {
    use qa_render::{Assets, surface_cache::IndexedTexture};
    let mut assets = Assets::load();
    let indices = [0, 17, 255, 23];
    let disk_mips: [&[u8]; 4] = [&indices, &[5], &[6], &[7]];
    let indexed = IndexedTexture::load_masked(2, 2, disk_mips, Some(0)).unwrap();
    let raw = gray(&[7, 11, 19, 23, 31, 43, 59, 71]);
    let id = assets
        .register_rgba_with_indexed(4, 2, &raw, indexed)
        .unwrap();
    let scale = light_scale(RgbLut::default(), 2.0).unwrap();
    assets
        .prepare_image(
            id,
            UploadParams {
                mipmaps: MipmapBuild::Box,
                rgb_lut: scale.rgb_lut,
                inverse_intensity: scale.inverse_intensity,
                ..UploadParams::default()
            },
        )
        .unwrap();
    let image = assets.image(id).unwrap();
    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(image.rgba.as_ref(), raw);
    let indexed = image.indexed.as_ref().unwrap();
    assert_eq!(indexed.transparent_index(), Some(0));
    for (mip, bytes) in disk_mips.iter().enumerate() {
        assert_eq!(indexed.mip(mip as u8).unwrap().indices(), *bytes);
    }
    let prepared = image.prepared.as_ref().unwrap();
    assert_eq!(prepared.inverse_intensity, 0.5);
    assert_eq!(prepared.levels.len(), 3);
    assert_eq!(prepared.levels[0].rgba[0], 14);
    let prior: Vec<_> = prepared
        .levels
        .iter()
        .map(|level| (level.width, level.height, level.rgba.to_vec()))
        .collect();
    assert!(
        assets
            .prepare_image(
                id,
                UploadParams {
                    mipmaps: MipmapBuild::Weighted,
                    extent: UploadExtent::PowerOfTwoMip {
                        round: ExtentRound::Up,
                        drop: 32,
                        max_dimension: 256,
                        kernel: MipmapBuild::Weighted,
                    },
                    ..UploadParams::default()
                }
            )
            .is_err()
    );
    let image = assets.image(id).unwrap();
    let after: Vec<_> = image
        .prepared
        .as_ref()
        .unwrap()
        .levels
        .iter()
        .map(|level| (level.width, level.height, level.rgba.to_vec()))
        .collect();
    assert_eq!(after, prior);
    assert_eq!(image.rgba.as_ref(), raw);
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        indices
    );
}

#[test]
fn alternate_gl_expansion_keeps_raw_sources_and_prior_preparation_on_failure() {
    use qa_render::{Assets, surface_cache::IndexedTexture};
    let mut assets = Assets::load();
    let indices = [0, 17, 255, 23];
    let indexed = IndexedTexture::load_base(2, 2, &indices, Some(0)).unwrap();
    let raw = gray(&[7, 11, 19, 23]);
    let id = assets
        .register_rgba_with_indexed(2, 2, &raw, indexed)
        .unwrap();
    let gl_expansion = gray(&[31, 43, 59, 71]);
    assets
        .prepare_image_with_rgba(id, &gl_expansion, UploadParams::default())
        .unwrap();
    let image = assets.image(id).unwrap();
    assert_eq!(image.rgba.as_ref(), raw);
    assert_eq!(
        image.prepared.as_ref().unwrap().levels[0].rgba.as_ref(),
        gl_expansion
    );
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices(),
        indices
    );
    assert_eq!(image.indexed.as_ref().unwrap().transparent_index(), Some(0));
    assert!(
        assets
            .prepare_image_with_rgba(id, &[1, 2, 3], UploadParams::default())
            .is_err()
    );
    let image = assets.image(id).unwrap();
    assert_eq!(
        image.prepared.as_ref().unwrap().levels[0].rgba.as_ref(),
        gl_expansion
    );
    assert_eq!(image.rgba.as_ref(), raw);
}

#[test]
fn original_nearest_and_four_tap_sample_positions() {
    let source = gray(&[0, 10, 20, 40, 50, 60, 80, 90, 100]);
    for (filter, expected) in [
        (
            ResizeFilter::Nearest,
            [
                0, 10, 10, 20, 0, 10, 10, 20, 40, 50, 50, 60, 80, 90, 90, 100,
            ],
        ),
        (
            ResizeFilter::FourTap,
            [
                0, 5, 15, 20, 20, 25, 35, 40, 60, 65, 75, 80, 80, 85, 95, 100,
            ],
        ),
    ] {
        let prepared = prepare_rgba(
            3,
            3,
            &source,
            UploadParams {
                extent: UploadExtent::PowerOfTwo {
                    round: ExtentRound::Up,
                    drop: 0,
                    max_dimension: 256,
                },
                resize: filter,
                ..UploadParams::default()
            },
        )
        .unwrap();
        assert_eq!(channels(&prepared.levels[0].rgba), expected);
    }
}

#[test]
fn lookup_order_and_native_no_resize_gamma_bypass_preserve_alpha() {
    let source = [0, 0, 0, 19, 255, 255, 255, 73, 0, 0, 0, 127];
    let lookup = RgbLut(std::array::from_fn(|v| if v < 128 { 0 } else { 255 }));
    let params = UploadParams {
        extent: UploadExtent::PowerOfTwo {
            round: ExtentRound::Down,
            drop: 0,
            max_dimension: 256,
        },
        rgb_lut: lookup,
        ..UploadParams::default()
    };
    let before = prepare_rgba(
        3,
        1,
        &source,
        UploadParams {
            color_order: ColorOrder::BeforeResize,
            ..params
        },
    )
    .unwrap();
    let after = prepare_rgba(3, 1, &source, params).unwrap();
    assert_eq!(channels(&before.levels[0].rgba), [127, 127]);
    assert_eq!(channels(&after.levels[0].rgba), [0, 0]);
    assert_eq!(before.levels[0].rgba[3], 46);
    assert_eq!(before.levels[0].rgba[7], 100);
    assert_eq!(before.levels[0].rgba[3], after.levels[0].rgba[3]);
    let unchanged = prepare_rgba(
        2,
        1,
        &source[..8],
        UploadParams {
            color_order: ColorOrder::AfterResizeIfChanged,
            ..params
        },
    )
    .unwrap();
    assert_eq!(&*unchanged.levels[0].rgba, &source[..8]);
}

#[test]
fn native_gamma_curves_and_intensity_are_distinct_byte_operations() {
    let identity = gamma_lut(1.0, GammaCurve::HalfPixelPower).unwrap();
    assert_eq!(identity, RgbLut::default());
    assert_eq!(gamma_lut(1.0, GammaCurve::BytePower).unwrap(), identity);
    let palette = gamma_lut(1.0, GammaCurve::PalettePower).unwrap();
    assert_eq!(palette.0[0], 1);
    assert_eq!(palette.0[255], 255);
    let scale = light_scale(identity, 2.0).unwrap();
    assert_eq!(scale.rgb_lut.0[127], 254);
    assert_eq!(scale.rgb_lut.0[128], 255);
    assert_eq!(scale.inverse_intensity.to_bits(), 0.5_f32.to_bits());
    assert_eq!(light_scale(identity, 0.5).unwrap().rgb_lut, identity);
    let nonlinear = RgbLut(std::array::from_fn(|v| (v / 2) as u8));
    assert_eq!(light_scale(nonlinear, 2.0).unwrap().rgb_lut.0[200], 127);
}

#[test]
fn legacy_rectangular_tails_and_simple_one_dimensional_mips_differ() {
    for (width, height, legacy, simple) in [(2, 8, 84, 90), (8, 2, 54, 90)] {
        let pixels = gray(&(0..16).map(|v| v * 12).collect::<Vec<_>>());
        for (kernel, expected) in [(MipmapBuild::LegacyBox, legacy), (MipmapBuild::Box, simple)] {
            let prepared = prepare_rgba(
                width,
                height,
                &pixels,
                UploadParams {
                    mipmaps: kernel,
                    ..UploadParams::default()
                },
            )
            .unwrap();
            assert_eq!(prepared.levels.len(), 4);
            assert_eq!(channels(&prepared.levels[3].rgba), [expected]);
            assert!(
                prepared
                    .levels
                    .iter()
                    .all(|level| { level.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255) })
            );
        }
    }
    let thin = prepare_rgba(
        1,
        4,
        &gray(&[0, 20, 60, 100]),
        UploadParams {
            mipmaps: MipmapBuild::Box,
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!(channels(&thin.levels[1].rgba), [10, 80]);
    assert_eq!(channels(&thin.levels[2].rgba), [45]);
}

#[test]
fn picmip_kernel_reduction_precedes_color_and_scales_axes_together() {
    let pixels = gray(&(0..16).map(|v| v * 8).collect::<Vec<_>>());
    let prepared = prepare_rgba(
        4,
        4,
        &pixels,
        UploadParams {
            extent: UploadExtent::PowerOfTwoMip {
                round: ExtentRound::Up,
                drop: 2,
                max_dimension: 256,
                kernel: MipmapBuild::Box,
            },
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!(channels(&prepared.levels[0].rgba), [60]);
    let direct = prepare_rgba(
        4,
        4,
        &pixels,
        UploadParams {
            extent: UploadExtent::PowerOfTwo {
                round: ExtentRound::Up,
                drop: 2,
                max_dimension: 256,
            },
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!(channels(&direct.levels[0].rgba), [80]);
    let capped = prepare_rgba(
        8,
        2,
        &gray(&[17; 16]),
        UploadParams {
            extent: UploadExtent::PowerOfTwoMip {
                round: ExtentRound::Up,
                drop: 0,
                max_dimension: 4,
                kernel: MipmapBuild::Box,
            },
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!((capped.levels[0].width, capped.levels[0].height), (4, 1));
}

#[test]
fn unknown_and_undefined_native_inputs_return_scoped_errors() {
    assert_eq!(
        prepare_rgba(0, 1, &[], UploadParams::default()),
        Err(UploadError::Dimensions)
    );
    assert_eq!(
        prepare_rgba(1, 1, &[0; 3], UploadParams::default()),
        Err(UploadError::PixelCount)
    );
    assert_eq!(
        prepare_rgba(
            1,
            2,
            &[0; 8],
            UploadParams {
                mipmaps: MipmapBuild::LegacyBox,
                ..UploadParams::default()
            }
        ),
        Err(UploadError::LegacyMipBounds)
    );
    for mipmaps in [MipmapBuild::Box, MipmapBuild::Weighted] {
        assert_eq!(
            prepare_rgba(
                3,
                1,
                &[0; 12],
                UploadParams {
                    mipmaps,
                    ..UploadParams::default()
                }
            ),
            Err(UploadError::NonPowerOfTwoMip)
        );
    }
    assert_eq!(
        gamma_lut(f32::NAN, GammaCurve::PalettePower),
        Err(UploadError::InvalidColor)
    );
    assert_eq!(
        light_scale(RgbLut::default(), f32::INFINITY),
        Err(UploadError::InvalidColor)
    );
    assert_eq!(
        prepare_rgba(
            1,
            1,
            &[0; 4],
            UploadParams {
                extent: UploadExtent::PowerOfTwo {
                    round: ExtentRound::Up,
                    drop: 32,
                    max_dimension: 256
                },
                ..UploadParams::default()
            }
        ),
        Err(UploadError::Extent)
    );
}

#[test]
fn weighted_mips_wrap_all_channels_and_read_original_pixels_until_copyback() {
    let mut pixels = vec![0; 4 * 4 * 4];
    pixels[15 * 4..16 * 4].copy_from_slice(&[255, 128, 64, 32]);
    let original = pixels.clone();
    let prepared = prepare_rgba(
        4,
        4,
        &pixels,
        UploadParams {
            mipmaps: MipmapBuild::Weighted,
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!(
        prepared.levels[1].rgba.as_ref(),
        [7, 3, 1, 0, 14, 7, 3, 1, 14, 7, 3, 1, 28, 14, 7, 3]
    );
    assert_eq!(prepared.levels[2].rgba.as_ref(), [15, 7, 3, 1]);
    assert_eq!(pixels, original);
}

#[test]
fn weighted_one_dimensional_tails_keep_native_prefix_without_pair_averaging() {
    for (width, height) in [(4, 1), (1, 4)] {
        let pixels = gray(&[0, 30, 200, 255]);
        let prepared = prepare_rgba(
            width,
            height,
            &pixels,
            UploadParams {
                mipmaps: MipmapBuild::Weighted,
                ..UploadParams::default()
            },
        )
        .unwrap();
        assert_eq!(prepared.levels.len(), 3);
        assert_eq!(channels(&prepared.levels[1].rgba), [0, 30]);
        assert_eq!(channels(&prepared.levels[2].rgba), [0]);
        assert_eq!(
            (prepared.levels[1].width, prepared.levels[1].height),
            if width == 4 { (2, 1) } else { (1, 2) }
        );
    }
    let pixels = gray(&[
        0, 30, 90, 150, 210, 240, 255, 255, 20, 40, 80, 140, 200, 230, 250, 255,
    ]);
    let prepared = prepare_rgba(
        8,
        2,
        &pixels,
        UploadParams {
            mipmaps: MipmapBuild::Weighted,
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!(channels(&prepared.levels[1].rgba), [71, 116, 212, 210]);
    assert_eq!(channels(&prepared.levels[2].rgba), [71, 116]);
    assert_eq!(channels(&prepared.levels[3].rgba), [71]);
}

#[test]
fn weighted_picmip_reduces_before_lookup_and_keeps_initialized_rectangular_tail() {
    let pixels = gray(&[
        0, 30, 90, 150, 210, 240, 255, 255, 20, 40, 80, 140, 200, 230, 250, 255,
    ]);
    let prepared = prepare_rgba(
        8,
        2,
        &pixels,
        UploadParams {
            extent: UploadExtent::PowerOfTwoMip {
                round: ExtentRound::Up,
                drop: 2,
                max_dimension: 256,
                kernel: MipmapBuild::Weighted,
            },
            mipmaps: MipmapBuild::Weighted,
            rgb_lut: RgbLut(std::array::from_fn(|value| (value as u8).saturating_add(5))),
            ..UploadParams::default()
        },
    )
    .unwrap();
    assert_eq!(
        (prepared.levels[0].width, prepared.levels[0].height),
        (2, 1)
    );
    assert_eq!(channels(&prepared.levels[0].rgba), [76, 121]);
    assert_eq!(channels(&prepared.levels[1].rgba), [76]);
}

/// Optional export for tools/check_image_upload.py; ordinary tests execute no C.
#[test]
fn original_image_upload_fixture_export() {
    let Ok(directory) = std::env::var("QA_IMAGE_UPLOAD_EVIDENCE") else {
        return;
    };
    let mut input = Vec::new();
    let mut output = Vec::new();
    let mut state = 0x494d4755_u32;
    for case in 0..96_u32 {
        let lookup = case % 3;
        let width = if lookup == 2 {
            1 << (1 + case % 4)
        } else if lookup == 0 {
            4 + case % 11
        } else {
            3 + case % 11
        };
        let height = if lookup == 2 {
            1 << (1 + (case / 4) % 4)
        } else {
            3 + (case / 11) % 7
        };
        let mipmaps = if case % 2 == 0 {
            MipmapBuild::None
        } else if lookup == 2 {
            MipmapBuild::Box
        } else {
            MipmapBuild::LegacyBox
        };
        let filter = if lookup == 0 {
            ResizeFilter::Nearest
        } else {
            ResizeFilter::FourTap
        };
        let order = if lookup == 0 {
            ColorOrder::BeforeResize
        } else if mipmaps == MipmapBuild::None {
            ColorOrder::AfterResizeIfChanged
        } else {
            ColorOrder::AfterResize
        };
        let gamma = [0.7_f32, 1.0, 1.3][(case / 3) as usize % 3];
        let intensity = [2.0_f32, 1.75, 0.5][(case / 9) as usize % 3];
        let lut = if lookup == 0 {
            gamma_lut(gamma, GammaCurve::PalettePower).unwrap()
        } else if lookup == 1 {
            gamma_lut(gamma, GammaCurve::HalfPixelPower).unwrap()
        } else {
            RgbLut::default()
        };
        let scale = light_scale(
            lut,
            if lookup == 1 && mipmaps != MipmapBuild::None {
                intensity
            } else {
                1.0
            },
        )
        .unwrap();
        let params = UploadParams {
            extent: UploadExtent::PowerOfTwo {
                round: if case % 4 == 0 {
                    ExtentRound::Down
                } else {
                    ExtentRound::Up
                },
                drop: 0,
                max_dimension: 256,
            },
            resize: filter,
            mipmaps,
            color_order: order,
            rgb_lut: scale.rgb_lut,
            inverse_intensity: scale.inverse_intensity,
        };
        let pixels: Vec<_> = (0..width * height * 4)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let prepared = prepare_rgba(width, height, &pixels, params).unwrap();
        let base = &prepared.levels[0];
        for value in [
            u32::from(filter == ResizeFilter::FourTap),
            match mipmaps {
                MipmapBuild::None => 0,
                MipmapBuild::LegacyBox => 1,
                _ => 2,
            },
            match order {
                ColorOrder::BeforeResize => 0,
                ColorOrder::AfterResize => 1,
                _ => 2,
            },
            lookup,
            width,
            height,
            base.width,
            base.height,
        ] {
            input.extend_from_slice(&value.to_le_bytes());
        }
        input.extend_from_slice(&gamma.to_le_bytes());
        input.extend_from_slice(&intensity.to_le_bytes());
        input.extend_from_slice(&pixels);
        output.extend_from_slice(&(prepared.levels.len() as u32).to_le_bytes());
        output.extend_from_slice(&prepared.inverse_intensity.to_le_bytes());
        for level in &prepared.levels {
            output.extend_from_slice(&level.width.to_le_bytes());
            output.extend_from_slice(&level.height.to_le_bytes());
            output.extend_from_slice(&level.rgba);
        }
    }
    let directory = std::path::Path::new(&directory);
    std::fs::write(directory.join("input.bin"), input).unwrap();
    std::fs::write(directory.join("rust.bin"), output).unwrap();
    let mut input = Vec::new();
    let mut output = Vec::new();
    let palette: [[u8; 4]; 256] = std::array::from_fn(|i| {
        [
            i as u8,
            (i as u8).wrapping_mul(17),
            (i as u8).wrapping_mul(31),
            255,
        ]
    });
    for height in 1..=6_u32 {
        for width in 1..=6_u32 {
            for pattern in 0..4_u32 {
                let indices: Vec<_> = (0..width * height)
                    .map(|i| {
                        if pattern == 0 || (i + pattern) % 3 == 0 {
                            255
                        } else {
                            ((i * 13 + pattern * 11) % 255) as u8
                        }
                    })
                    .collect();
                input.extend_from_slice(&width.to_le_bytes());
                input.extend_from_slice(&height.to_le_bytes());
                for color in palette {
                    input.extend_from_slice(&color);
                }
                input.extend_from_slice(&indices);
                let pixels = expand_indexed(
                    &indices,
                    width,
                    height,
                    &palette,
                    Some(255),
                    AlphaFringe::NativeNeighbors,
                )
                .unwrap();
                output.extend_from_slice(&pixels);
            }
        }
    }
    std::fs::write(directory.join("indexed-input.bin"), input).unwrap();
    std::fs::write(directory.join("indexed-rust.bin"), output).unwrap();
    let mut input = Vec::new();
    let mut output = Vec::new();
    let mut state = 0x4d495032_u32;
    // 49 shapes include every rectangular/1D pair of powers from 1 to 64.
    for height_power in 0..=6 {
        for width_power in 0..=6 {
            let width = 1u32 << width_power;
            let height = 1u32 << height_power;
            let pixels: Vec<_> = (0..width * height * 4)
                .map(|_| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    (state >> 24) as u8
                })
                .collect();
            let prepared = prepare_rgba(
                width,
                height,
                &pixels,
                UploadParams {
                    mipmaps: MipmapBuild::Weighted,
                    ..UploadParams::default()
                },
            )
            .unwrap();
            // Existing native-driver Fixture header: filter, kernel, order,
            // lookup, source width/height, final base width/height, gamma/intensity.
            for value in [1u32, 3, 1, 2, width, height, width, height] {
                input.extend_from_slice(&value.to_le_bytes());
            }
            input.extend_from_slice(&1.0f32.to_le_bytes());
            input.extend_from_slice(&1.0f32.to_le_bytes());
            input.extend_from_slice(&pixels);
            output.extend_from_slice(&(prepared.levels.len() as u32).to_le_bytes());
            output.extend_from_slice(&prepared.inverse_intensity.to_le_bytes());
            for level in &prepared.levels {
                output.extend_from_slice(&level.width.to_le_bytes());
                output.extend_from_slice(&level.height.to_le_bytes());
                output.extend_from_slice(&level.rgba);
            }
        }
    }
    std::fs::write(directory.join("weighted-input.bin"), input).unwrap();
    std::fs::write(directory.join("weighted-rust.bin"), output).unwrap();
}
