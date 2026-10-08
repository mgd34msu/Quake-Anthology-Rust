use super::{Grid, cached_rgba_span, scalar_span};
use crate::cpu::world::{Affine, Buffers, Planes};
use crate::edges::Span;
use crate::surface_cache::{CacheSpan, RgbaBuildState, SurfaceCache, SurfaceSource};

#[derive(Clone)]
struct Frame {
    first_row: u32,
    pixels: Vec<u32>,
    depth: Vec<f32>,
    ranks: Vec<u32>,
    indices: Vec<u8>,
    palettes: Vec<u32>,
}

impl Frame {
    fn load(width: u32, rows: u32, first_row: u32) -> Self {
        let depth = [
            0.0,
            -0.0,
            0.125,
            0.5,
            1.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(0x7fc1_2345),
            f32::from_bits(0xff81_2345),
        ];
        let ranks = [0, 0x7fff_ffff, 0x8000_0000, u32::MAX];
        let count = width as usize * rows as usize;
        Self {
            first_row,
            pixels: (0..count).map(|i| 0x1357_9bdf ^ i as u32).collect(),
            depth: (0..count).map(|i| depth[i % depth.len()]).collect(),
            ranks: (0..count).map(|i| ranks[i % ranks.len()]).collect(),
            indices: (0..count).map(|i| (i * 37 + 19) as u8).collect(),
            palettes: (0..count).map(|i| 0x2468_ace0 ^ i as u32).collect(),
        }
    }

    fn buffers(&mut self, width: u32) -> Buffers<'_> {
        Buffers {
            first_row: self.first_row,
            frame_height: self.first_row + self.pixels.len() as u32 / width,
            pixels: &mut self.pixels,
            inverse_depth: &mut self.depth,
            depth_ranks: &mut self.ranks,
            indices: &mut self.indices,
            palettes: &mut self.palettes,
        }
    }
}

fn descriptor() -> CacheSpan {
    let source = SurfaceSource::load_rgba([0; 2], [1; 2], 1).unwrap();
    let mut cache = SurfaceCache::load(vec![source], 4096).unwrap();
    assert!(cache.begin_batch());
    let block = cache
        .prepare_rgba(0, 0, RgbaBuildState::default(), |bytes| bytes.fill(0))
        .unwrap();
    assert!(cache.rgba_pixels(block).is_some());
    block
}

fn grid(dimensions: [u32; 2], minimum: [i32; 2], mip: u8) -> (CacheSpan, Vec<[u8; 4]>) {
    // The kernel consumes only numeric layout fields. These independently
    // owned payloads also exercise a hypothetical wider future cache catalog.
    let mut block = descriptor();
    block.width = dimensions[0];
    block.height = dimensions[1];
    block.texture_mins = minimum;
    block.mip = mip;
    let pixels = (0..dimensions[0] as usize * dimensions[1] as usize)
        .map(|i| {
            let word = (i as u32)
                .wrapping_mul(0x0123_4567)
                .wrapping_add(0x89ab_cdef);
            word.to_le_bytes()
        })
        .collect();
    (block, pixels)
}

// Frozen pre-SIMD rgba_span pixel loop and Product::cached_pixel arithmetic.
// It deliberately retains floor(), scalar indexing and independent per-pixel
// plane evaluation instead of sharing the new kernel's setup or masks.
fn reference(
    width: u32,
    span: Span,
    planes: &Planes,
    rank: u32,
    block: CacheSpan,
    texels: &[[u8; 4]],
    frame: &mut Frame,
) -> u32 {
    let mut written = 0;
    for x in span.x..span.x + span.count {
        let y = span.y as f32;
        let xf = x as f32;
        let plane = planes.inverse_depth;
        let zi = plane.origin + plane.y * y + plane.x * xf;
        let index = ((span.y - frame.first_row) * width + x) as usize;
        if zi <= 0.0
            || !zi.is_finite()
            || !(zi > frame.depth[index]
                || (zi == frame.depth[index] && rank >= frame.ranks[index]))
        {
            continue;
        }
        let chart: [f32; 2] = std::array::from_fn(|axis| {
            let plane = planes.texture[axis];
            (plane.origin + plane.y * y + plane.x * xf) / zi
        });
        let step = (1u64 << block.mip) as f32;
        let coordinate: [usize; 2] = std::array::from_fn(|axis| {
            (((chart[axis] - block.texture_mins[axis] as f32) / step).floor() as usize)
                .min([block.width, block.height][axis] as usize - 1)
        });
        frame.pixels[index] =
            u32::from_le_bytes(texels[coordinate[1] * block.width as usize + coordinate[0]]);
        frame.palettes[index] = u32::MAX;
        frame.depth[index] = zi;
        frame.ranks[index] = rank;
        written += 1;
    }
    written
}

fn equal(actual: &Frame, expected: &Frame) {
    assert_eq!(actual.pixels, expected.pixels);
    assert_eq!(actual.ranks, expected.ranks);
    assert_eq!(actual.indices, expected.indices);
    assert_eq!(actual.palettes, expected.palettes);
    assert_eq!(
        actual
            .depth
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>(),
        expected
            .depth
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
}

fn compare(
    width: u32,
    span: Span,
    planes: &Planes,
    rank: u32,
    block: CacheSpan,
    texels: &[[u8; 4]],
    initial: &Frame,
) -> u32 {
    let mut expected = initial.clone();
    let count = reference(width, span, planes, rank, block, texels, &mut expected);
    let mut scalar = initial.clone();
    let scalar_count = scalar_span(
        width,
        span,
        planes,
        rank,
        Grid::from_cache(block, texels),
        &mut scalar.buffers(width),
    );
    assert_eq!(scalar_count, count);
    equal(&scalar, &expected);
    let mut actual = initial.clone();
    let actual_count = cached_rgba_span(
        width,
        span,
        planes,
        rank,
        block,
        texels,
        &mut actual.buffers(width),
    );
    assert_eq!(actual_count, count);
    equal(&actual, &expected);
    count
}

fn affine(x: f32, y: f32, origin: f32) -> Affine {
    Affine { x, y, origin }
}

#[test]
fn all_lane_masks_and_scalar_tails_preserve_rejected_bits() {
    let (block, texels) = grid([7, 5], [-32, 16], 3);
    let planes = Planes {
        inverse_depth: affine(0.0, 0.0, 1.0),
        texture: [affine(1.25, 0.5, -29.0), affine(-0.375, 0.25, 19.0)],
        ..Planes::default()
    };
    for mask in 0..16 {
        for tail in 0..4 {
            let span = Span {
                x: 3,
                y: 14,
                count: 4 + tail,
                ..Span::default()
            };
            let mut frame = Frame::load(19, 3, 13);
            for lane in 0..4 {
                let index = (span.y - frame.first_row) as usize * 19 + span.x as usize + lane;
                frame.depth[index] = if mask & (1 << lane) != 0 { 0.0 } else { 2.0 };
            }
            let count = compare(19, span, &planes, 0x8000_0000, block, &texels, &frame);
            assert!(count >= (mask as u32).count_ones());
            assert!(count <= (mask as u32).count_ones() + tail);
        }
    }
}

#[test]
fn unsigned_depth_ties_and_exceptional_stored_depths_match() {
    let (block, texels) = grid([5, 3], [-2, -3], 0);
    let planes = Planes {
        inverse_depth: affine(0.0, 0.0, 0.5),
        texture: [affine(0.25, 0.0, -1.0), affine(-0.125, 0.0, -2.0)],
        ..Planes::default()
    };
    for rank in [0, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
        let mut frame = Frame::load(23, 2, 37);
        frame.depth[..4].fill(0.5);
        frame.ranks[..4].copy_from_slice(&[0, 0x7fff_ffff, 0x8000_0000, u32::MAX]);
        let count = compare(
            23,
            Span {
                x: 0,
                y: 37,
                count: 23,
                ..Span::default()
            },
            &planes,
            rank,
            block,
            &texels,
            &frame,
        );
        assert!(
            count
                >= [0, 0x7fff_ffff, 0x8000_0000, u32::MAX]
                    .into_iter()
                    .filter(|&existing| rank >= existing)
                    .count() as u32
        );
    }
}

#[test]
fn nonfinite_incoming_depth_and_far_plane_rounding_match() {
    let (block, texels) = grid([13, 7], [0, 0], 0);
    let far = 1.0f32 / 8192.0;
    let values = [
        0.0,
        -0.0,
        f32::from_bits(1),
        -f32::from_bits(1),
        f32::from_bits(far.to_bits() - 1),
        far,
        f32::from_bits(far.to_bits() + 1),
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7fc1_2345),
        f32::from_bits(0xff81_2345),
    ];
    for origin in values {
        for slope in [0.0, f32::from_bits(1), -f32::from_bits(1), f32::MAX] {
            let planes = Planes {
                inverse_depth: affine(slope, -slope, origin),
                texture: [affine(0.5, -0.25, 3.0), affine(-0.25, 0.5, 1.0)],
                ..Planes::default()
            };
            let mut frame = Frame::load(29, 2, 11);
            frame.depth.fill(far);
            frame.ranks.fill(0);
            compare(
                29,
                Span {
                    x: 0,
                    y: 11,
                    count: 29,
                    ..Span::default()
                },
                &planes,
                0,
                block,
                &texels,
                &frame,
            );
        }
    }
}

#[test]
fn lattice_casts_keep_exceptional_values_mips_and_extreme_minima() {
    let values = [
        0.0,
        -0.0,
        f32::from_bits(1),
        -f32::from_bits(1),
        -0.99999994,
        -1.0,
        0.99999994,
        1.0,
        1.0000001,
        3.9999998,
        4.0,
        f32::MAX,
        -f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7fc1_2345),
        f32::from_bits(0xff81_2345),
    ];
    for minimum in [i32::MIN, -8388609, -17, 0, 17, 8388609, i32::MAX] {
        for mip in [0, 1, 3, 15, 31] {
            let (block, texels) = grid([7, 13], [minimum, -17], mip);
            let step = (1u64 << mip) as f32;
            for value in values {
                let chart = [minimum as f32 + value * step, -17.0 + value * step];
                let planes = Planes {
                    inverse_depth: affine(0.0, 0.0, 1.0),
                    texture: [affine(0.0, 0.0, chart[0]), affine(0.0, 0.0, chart[1])],
                    ..Planes::default()
                };
                let mut frame = Frame::load(13, 1, 17);
                frame.depth.fill(0.0);
                assert_eq!(
                    compare(
                        13,
                        Span {
                            x: 0,
                            y: 17,
                            count: 13,
                            ..Span::default()
                        },
                        &planes,
                        0,
                        block,
                        &texels,
                        &frame,
                    ),
                    13
                );
            }
        }
    }
}

#[test]
fn one_dimensional_grids_and_wider_catalog_fallback_match() {
    for dimensions in [
        [1, 1],
        [1, 8192],
        [8192, 1],
        [8192, 3],
        [32767, 1],
        [32767, 3],
        [3, 32767],
        [32768, 1],
        [1, 32768],
    ] {
        let (block, texels) = grid(dimensions, [-41, 29], 0);
        let planes = Planes {
            inverse_depth: affine(0.0078125, -0.00390625, 1.0),
            texture: [
                affine(4096.0, -1024.0, -10000.0),
                affine(-2048.0, 512.0, 20000.0),
            ],
            ..Planes::default()
        };
        let mut frame = Frame::load(41, 2, 3);
        frame.depth.fill(0.0);
        for count in 0..=37 {
            assert_eq!(
                compare(
                    41,
                    Span {
                        x: 4,
                        y: 4,
                        count,
                        ..Span::default()
                    },
                    &planes,
                    u32::MAX,
                    block,
                    &texels,
                    &frame,
                ),
                count
            );
        }
    }
}

fn next(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}

#[test]
fn seeded_planes_keep_output_bits_across_offset_spans() {
    let (mut block, texels) = grid([17, 13], [-5, 9], 0);
    let mut seed = 0x636f_7079;
    for case in 0..4096 {
        block.mip = (case % 32) as u8;
        let minimum = [next(&mut seed) as i32, next(&mut seed) as i32];
        block.texture_mins = if case & 4 == 0 { [0, 0] } else { minimum };
        let mut coefficient = || {
            let word = next(&mut seed);
            match case % 4 {
                0 => f32::from_bits(word),
                1 => (word as i32 % 65) as f32 / 64.0,
                2 => (word as i32 % 17) as f32 / 8192.0,
                _ => f32::from_bits((word & 0x8000_0000) | (word & 0x007f_ffff)),
            }
        };
        let planes = Planes {
            inverse_depth: affine(coefficient(), coefficient(), coefficient() + 1.0),
            texture: [
                affine(coefficient(), coefficient(), coefficient()),
                affine(coefficient(), coefficient(), coefficient()),
            ],
            ..Planes::default()
        };
        let rank = next(&mut seed);
        let span = Span {
            x: case % 11,
            y: 13 + case % 3,
            count: case % 38,
            ..Span::default()
        };
        let frame = Frame::load(53, 3, 13);
        compare(53, span, &planes, rank, block, &texels, &frame);
    }
}
