use super::{TexelView, Wrap, image_repeat_mask, sample, sampler_function, stage_sampler};
use crate::assets::upload::{MipLevel, PreparedImage};
use crate::assets::{Filter, Image, Sampler, TextureIntensity};
use crate::lightmap::AtlasRegion;

// Frozen four-corner sampler from e9e6211:crates/render/src/cpu.rs::sample.
// This reference always borrows the complete native image, never a copied ROI.
fn original_texel(coordinate: f32, size: u32, wrap: Wrap) -> usize {
    let coordinate = match wrap {
        Wrap::Repeat => coordinate.rem_euclid(1.0),
        Wrap::Clamp => coordinate.clamp(0.0, 1.0),
    };
    ((coordinate * size as f32) as usize).min(size as usize - 1)
}

fn original<const LINEAR: bool, const REPEAT: bool>(
    image: TexelView<'_>,
    coordinates: [f32; 2],
    color: [f32; 4],
) -> [f32; 4] {
    let texture: [f32; 4] = if LINEAR {
        let size = [image.width, image.height];
        let p = std::array::from_fn::<_, 2, _>(|i| coordinates[i] * size[i] as f32 - 0.5);
        let base = p.map(f32::floor);
        let fraction = [p[0] - base[0], p[1] - base[1]];
        let samples: [[f32; 4]; 4] = std::array::from_fn(|corner| {
            let xy = std::array::from_fn::<_, 2, _>(|i| {
                let value = base[i] + ((corner >> i) & 1) as f32;
                if REPEAT {
                    value.rem_euclid(size[i] as f32) as usize
                } else {
                    value.clamp(0.0, size[i] as f32 - 1.0) as usize
                }
            });
            let xy: [usize; 2] = std::array::from_fn(|axis| {
                xy[axis].clamp(
                    image.bounds[axis] as usize,
                    (image.bounds[axis] + image.bounds[axis + 2] - 1) as usize,
                )
            });
            let offset = (xy[1] * image.width as usize + xy[0]) * 4;
            std::array::from_fn(|i| image.rgba[offset + i] as f32 / 255.0)
        });
        std::array::from_fn(|i| {
            let a = samples[0][i] + fraction[0] * (samples[1][i] - samples[0][i]);
            let b = samples[2][i] + fraction[0] * (samples[3][i] - samples[2][i]);
            a + fraction[1] * (b - a)
        })
    } else {
        let wrap = if REPEAT { Wrap::Repeat } else { Wrap::Clamp };
        let x = original_texel(coordinates[0], image.width, wrap).clamp(
            image.bounds[0] as usize,
            (image.bounds[0] + image.bounds[2] - 1) as usize,
        );
        let y = original_texel(coordinates[1], image.height, wrap).clamp(
            image.bounds[1] as usize,
            (image.bounds[1] + image.bounds[3] - 1) as usize,
        );
        let offset = (y * image.width as usize + x) * 4;
        std::array::from_fn(|i| image.rgba[offset + i] as f32 / 255.0)
    };
    std::array::from_fn(|i| texture[i] * color[i] * if i < 3 { image.intensity } else { 1.0 })
}

fn next(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn pixels(width: u32, height: u32) -> Vec<[u8; 4]> {
    let mut state = 0x193c_20e7;
    (0..width * height)
        .map(|_| std::array::from_fn(|_| next(&mut state) as u8))
        .collect()
}

fn coordinate(state: &mut u32, index: usize, size: u32) -> f32 {
    const SPECIAL: [f32; 24] = [
        0.0,
        -0.0,
        f32::from_bits(1),
        f32::from_bits(0x8000_0001),
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1.0_f32.to_bits() - 1),
        1.0,
        f32::from_bits(1.0_f32.to_bits() + 1),
        -1.0,
        0.5,
        -0.5,
        1.5,
        -1.5,
        65_536.0,
        -65_536.0,
        f32::MAX,
        -f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::from_bits(0xffc0_0123),
        f32::from_bits(0x7f80_0123),
        f32::from_bits(0xff80_0123),
    ];
    let bits = next(state);
    match index % 6 {
        0 => SPECIAL[(index / 6) % SPECIAL.len()],
        1 => f32::from_bits(bits),
        2 => (bits as i32 % 1_000_001) as f32 / 4096.0,
        3 => {
            const CUTOVER: [f32; 8] = [
                -16_777_218.0,
                -16_777_216.0,
                -16_777_215.0,
                -16_777_214.0,
                16_777_214.0,
                16_777_215.0,
                16_777_216.0,
                16_777_218.0,
            ];
            (CUTOVER[(index / 6) % CUTOVER.len()] + 0.5) / size as f32
        }
        4 => {
            let edge = ((bits as i32 % (size as i32 * 4)) as f32 + 0.5) / size as f32;
            match (bits >> 29) % 3 {
                0 => edge,
                1 => edge.next_up(),
                _ => edge.next_down(),
            }
        }
        _ => (bits as i32 % 1_000_001) as f32 * 1.0e30,
    }
}

fn compare<const LINEAR: bool, const REPEAT: bool>(
    native: TexelView<'_>,
    candidate: TexelView<'_>,
    coordinates: [f32; 2],
    color: [f32; 4],
) {
    assert_eq!(
        sample::<LINEAR, REPEAT, false>(candidate, coordinates, color).map(f32::to_bits),
        original::<LINEAR, REPEAT>(native, coordinates, color).map(f32::to_bits),
        "linear={LINEAR} repeat={REPEAT} size={}x{} bounds={:?} coordinates={coordinates:?} color={color:?}",
        native.width,
        native.height,
        native.bounds,
    );
}

fn compare_all(native: TexelView<'_>, candidate: TexelView<'_>, seed: u32) {
    let mut state = seed;
    for index in 0..2048 {
        let coordinates = [
            coordinate(&mut state, index, native.width),
            coordinate(&mut state, index + 997, native.height),
        ];
        let color = std::array::from_fn(|channel| match (index + channel) % 7 {
            0 => 0.0,
            1 => 1.0,
            2 => -0.0,
            3 => f32::from_bits(1),
            _ => (next(&mut state) as i32 % 4097) as f32 / 255.0,
        });
        compare::<true, true>(native, candidate, coordinates, color);
        compare::<true, false>(native, candidate, coordinates, color);
        compare::<false, true>(native, candidate, coordinates, color);
        compare::<false, false>(native, candidate, coordinates, color);
        if native.width.is_power_of_two() && native.height.is_power_of_two() {
            compare_mask(native, candidate, coordinates, color);
        }
    }
}

fn compare_mask(
    native: TexelView<'_>,
    candidate: TexelView<'_>,
    coordinates: [f32; 2],
    color: [f32; 4],
) {
    assert_eq!(
        sample::<true, true, true>(candidate, coordinates, color).map(f32::to_bits),
        original::<true, true>(native, coordinates, color).map(f32::to_bits),
        "mask size={}x{} bounds={:?} stride={} coordinates={coordinates:?}",
        native.width,
        native.height,
        native.bounds,
        candidate.row_stride,
    );
}

#[test]
#[expect(
    clippy::assertions_on_constants,
    reason = "Unexpected fixture setup failure deliberately fails before testing the native output oracle"
)]
fn stratified_native_sampling_matches_frozen_output_bits() {
    for [width, height] in [
        [1, 1],
        [1, 31],
        [31, 1],
        [2, 2],
        [3, 5],
        [7, 11],
        [16, 32],
        [63, 64],
        [128, 128],
        [256, 17],
        [513, 257],
        [8192, 1],
        [1, 8192],
        [8192, 3],
    ] {
        let rgba = pixels(width, height);
        let full = TexelView::cache(width, height, &rgba);
        for [x, y, region_width, region_height] in [
            [0, 0, width, height],
            [
                width / 3,
                height / 3,
                width - width / 3,
                height - height / 3,
            ],
            [width - 1, height - 1, 1, 1],
        ] {
            let Some(mut native) = full.region(Some(AtlasRegion {
                page: 4,
                x,
                y,
                width: region_width,
                height: region_height,
            })) else {
                assert!(false, "valid native region");
                return;
            };
            native.intensity = match (x + y) % 3 {
                0 => 1.0,
                1 => 0.5,
                _ => 1.0 / 3.0,
            };
            compare_all(native, native, 0x84cd_1713);
        }
    }
}

#[test]
#[expect(
    clippy::assertions_on_constants,
    reason = "Unexpected fixture setup failure deliberately fails before testing the native output oracle"
)]
fn copied_roi_preserves_native_taps_and_output_bits() {
    for [width, height, x, y, region_width, region_height] in [
        [128, 128, 13, 21, 7, 11],
        [64, 32, 21, 13, 7, 5],
        [32, 1, 11, 0, 7, 1],
        [1, 32, 0, 11, 1, 7],
        [63, 17, 59, 14, 4, 3],
        [7, 11, 3, 4, 1, 1],
        [1, 31, 0, 13, 1, 5],
        [31, 1, 13, 0, 5, 1],
        [8192, 3, 8190, 1, 2, 2],
    ] {
        let rgba = pixels(width, height);
        let Some(mut native) = TexelView::cache(width, height, &rgba).region(Some(AtlasRegion {
            page: 0,
            x,
            y,
            width: region_width,
            height: region_height,
        })) else {
            assert!(false, "valid native region");
            return;
        };
        native.intensity = 1.0 / 3.0;
        let copied_pixels: Vec<[u8; 4]> = (y..y + region_height)
            .flat_map(|row| {
                let start = (row * width + x) as usize;
                rgba[start..start + region_width as usize].iter().copied()
            })
            .collect();
        let Some(copied) = native.copied(&copied_pixels) else {
            assert!(false, "valid copied region");
            return;
        };
        assert_eq!([copied.width, copied.height], [width, height]);
        assert_eq!(copied.bounds, native.bounds);
        assert_eq!(copied.row_stride, region_width);
        assert_eq!(copied.storage_origin, [x, y]);
        assert_eq!(copied.intensity.to_bits(), native.intensity.to_bits());
        compare_all(native, copied, 0x7219_fc27);
    }
}

fn image_fixture(width: u32, height: u32, levels: &[[u32; 2]]) -> Image {
    let prepared = (!levels.is_empty()).then(|| PreparedImage {
        levels: levels
            .iter()
            .map(|&[width, height]| MipLevel {
                width,
                height,
                rgba: pixels(width, height).as_flattened().into(),
            })
            .collect(),
        inverse_intensity: 1.0 / 3.0,
    });
    Image {
        width,
        height,
        rgba: pixels(width, height).as_flattened().into(),
        indexed: None,
        prepared,
        preparation_revision: 0,
        native_sampler: None,
    }
}

#[test]
fn image_selection_uses_every_selectable_logical_mip() {
    let repeat = Sampler::default();
    for levels in [
        &[[32, 8], [16, 4], [8, 2], [4, 1], [2, 1], [1, 1]][..],
        &[[8, 32], [4, 16], [2, 8], [1, 4], [1, 2], [1, 1]][..],
    ] {
        let image = image_fixture(31, 13, levels);
        assert!(image_repeat_mask(&image, true));
        assert!(image_repeat_mask(&image, false));
        let selected = stage_sampler(&image, repeat);
        let mut state = 0x7408_4acb;
        for mip in 0..levels.len() {
            let native = TexelView::image(&image, mip as u8, TextureIntensity::NeutralizeUpload);
            for index in 0..2048 {
                let coordinates = [
                    coordinate(&mut state, index, native.width),
                    coordinate(&mut state, index + 997, native.height),
                ];
                let color = [0.7, 1.0, -0.3, 0.5];
                assert_eq!(
                    selected(native, coordinates, color).map(f32::to_bits),
                    original::<true, true>(native, coordinates, color).map(f32::to_bits),
                );
            }
        }
    }
    let mut image = image_fixture(13, 7, &[[16, 8], [8, 4], [3, 2], [1, 1]]);
    assert!(image_repeat_mask(&image, false));
    assert!(!image_repeat_mask(&image, true));
    let general = sampler_function(repeat);
    assert!(std::ptr::fn_addr_eq(stage_sampler(&image, repeat), general));
    let base_only = Sampler {
        mipmaps: false,
        ..repeat
    };
    assert!(std::ptr::fn_addr_eq(
        stage_sampler(&image, base_only),
        sample::<true, true, true> as super::ShadeFn,
    ));
    image.prepared = None;
    assert!(!image_repeat_mask(&image, true));
    assert!(std::ptr::fn_addr_eq(stage_sampler(&image, repeat), general));
    image = image_fixture(16, 8, &[]);
    assert!(image_repeat_mask(&image, true));
    assert!(std::ptr::fn_addr_eq(
        stage_sampler(&image, repeat),
        sample::<true, true, true> as super::ShadeFn,
    ));
}

#[test]
fn native_sampler_filter_and_wrap_override_mask_selection() {
    let repeat = Sampler::default();
    let mut image = image_fixture(16, 8, &[]);
    for sampler in [
        Sampler {
            filter: Filter::Nearest,
            ..repeat
        },
        Sampler {
            wrap: Wrap::Clamp,
            ..repeat
        },
        Sampler {
            filter: Filter::Nearest,
            wrap: Wrap::Clamp,
            ..repeat
        },
    ] {
        assert!(std::ptr::fn_addr_eq(
            stage_sampler(&image, sampler),
            sampler_function(sampler),
        ));
        image.native_sampler = Some(sampler);
        assert!(std::ptr::fn_addr_eq(
            stage_sampler(&image, repeat),
            sampler_function(sampler),
        ));
        image.native_sampler = None;
    }
    image.native_sampler = Some(repeat);
    assert!(std::ptr::fn_addr_eq(
        stage_sampler(
            &image,
            Sampler {
                wrap: Wrap::Clamp,
                ..repeat
            }
        ),
        sample::<true, true, true> as super::ShadeFn,
    ));
}

#[test]
#[expect(
    clippy::assertions_on_constants,
    reason = "Unexpected fixture setup failure deliberately fails before testing the native output oracle"
)]
fn copied_roi_rejects_malformed_storage_and_region_extension() {
    let rgba = pixels(8, 6);
    let view = TexelView::cache(8, 6, &rgba);
    let region = AtlasRegion {
        page: 0,
        x: 2,
        y: 1,
        width: 3,
        height: 2,
    };
    let Some(native) = view.region(Some(region)) else {
        assert!(false, "valid native region");
        return;
    };
    assert!(native.copied(&rgba[..5]).is_none());
    assert!(native.copied(&rgba[..7]).is_none());
    let Some(copied) = native.copied(&rgba[..6]) else {
        assert!(false, "valid copied payload");
        return;
    };
    for invalid in [
        AtlasRegion { x: 1, ..region },
        AtlasRegion { y: 0, ..region },
        AtlasRegion { width: 4, ..region },
        AtlasRegion {
            height: 3,
            ..region
        },
        AtlasRegion { width: 0, ..region },
        AtlasRegion {
            x: u32::MAX,
            width: 2,
            ..region
        },
        AtlasRegion {
            y: u32::MAX,
            height: 2,
            ..region
        },
    ] {
        assert!(copied.region(Some(invalid)).is_none());
    }
    assert!(
        copied
            .region(Some(AtlasRegion {
                x: 3,
                y: 2,
                width: 1,
                height: 1,
                ..region
            }))
            .is_some()
    );
    for malformed in [
        TexelView {
            bounds: [0, 0, 0, 1],
            ..view
        },
        TexelView {
            bounds: [u32::MAX, 0, 2, 1],
            ..view
        },
        TexelView {
            row_stride: 0,
            ..view
        },
        TexelView {
            rgba: &view.rgba[..view.rgba.len() - 1],
            ..view
        },
        TexelView {
            storage_origin: [1, 0],
            ..view
        },
        TexelView { width: 0, ..view },
        TexelView { height: 0, ..view },
    ] {
        assert!(malformed.region(None).is_none());
        assert!(malformed.copied(&rgba).is_none());
    }
}
