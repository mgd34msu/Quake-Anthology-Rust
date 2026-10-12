use super::{Perspective, walk};
use crate::cpu::world::Affine;

#[test]
fn complete_and_partial_chunks_preserve_native_coordinates() {
    for chunk in [8_u32, 16, 32] {
        for count in 0..=chunk * 3 + 1 {
            let start = 17;
            let y = 31;
            let depth = Affine {
                x: -0.00013,
                y: 0.00017,
                origin: 0.83,
            };
            let texture = [
                Affine {
                    x: 0.3771,
                    y: -0.17,
                    origin: -0.73,
                },
                Affine {
                    x: -0.719,
                    y: 0.187,
                    origin: 117.7,
                },
            ];
            let adjust = [257, -113];
            let extents = [(256_i64 << 16) - 1; 2];
            let mut zi = depth.at(start as f32, y as f32);
            let mut divided = texture.map(|p| p.at(start as f32, y as f32));
            let endpoint = |divided: [f32; 2], zi: f32, minimum: i64| {
                let z = 65536.0 / zi;
                std::array::from_fn::<_, 2, _>(|i| {
                    ((divided[i] * z) as i64 + adjust[i]).clamp(minimum, extents[i])
                })
            };
            let mut current = endpoint(divided, zi, 0);
            let mut expected = Vec::new();
            let mut x = start;
            let end = start + count;
            while x < end {
                let n = chunk.min(end - x);
                let complete = x + n < end;
                let distance = if complete { chunk } else { n - 1 } as f32;
                divided[0] += texture[0].x * distance;
                divided[1] += texture[1].x * distance;
                zi += depth.x * distance;
                let next = endpoint(divided, zi, i64::from(chunk));
                let step = if complete {
                    std::array::from_fn(|i| (next[i] - current[i]) >> chunk.trailing_zeros())
                } else if n > 1 {
                    std::array::from_fn(|i| (next[i] - current[i]) / i64::from(n - 1))
                } else {
                    [0; 2]
                };
                for offset in 0..n {
                    expected.push((x + offset, current));
                    current[0] += step[0];
                    current[1] += step[1];
                }
                x += n;
                current = next;
            }
            let mut perspective = Perspective::new(depth, texture, adjust, [256; 2], start, y);
            let mut actual = Vec::new();
            walk(
                start,
                count,
                chunk,
                None,
                |_, distance, minimum| perspective.endpoint(distance, minimum),
                |x, st| actual.push((x, st)),
            );
            assert_eq!(actual, expected, "chunk={chunk} count={count}");
        }
    }
}

#[test]
fn tiled_chunks_mask_after_the_native_step_calculation() {
    // WinQuake/ref_soft Turbulent8 masks each chunk's initial coordinates only
    // after computing its steps from the unmasked perspective endpoints.
    for count in [1, 15, 16, 17, 31, 32, 33, 65] {
        for direction in [-1.0, 1.0] {
            let depth = Affine {
                x: -0.00013,
                y: 0.00017,
                origin: 0.83,
            };
            let texture = [
                Affine {
                    x: direction * 1.3771,
                    y: -0.17,
                    origin: -0.73,
                },
                Affine {
                    x: -direction * 0.719,
                    y: 0.187,
                    origin: 117.7,
                },
            ];
            let start = 17;
            let y = 31;
            let adjust = [(8192_i64 << 16) + 257, (8192_i64 << 16) - 113];
            let extent = (16384_i64 << 16) - 1;
            let mask = (128_i64 << 16) - 1;
            let mut divided = texture.map(|p| p.at(start as f32, y as f32));
            let mut zi = depth.at(start as f32, y as f32);
            let endpoint = |divided: [f32; 2], zi: f32, minimum: i64| {
                let z = 65536.0 / zi;
                std::array::from_fn::<_, 2, _>(|i| {
                    ((divided[i] * z) as i64 + adjust[i]).clamp(minimum, extent)
                })
            };
            let mut current = endpoint(divided, zi, 0);
            let mut remaining = count;
            let mut x = start;
            let mut expected = Vec::new();
            while remaining != 0 {
                let n = remaining.min(16);
                remaining -= n;
                let distance = if remaining != 0 { 16 } else { n - 1 } as f32;
                divided[0] += texture[0].x * distance;
                divided[1] += texture[1].x * distance;
                zi += depth.x * distance;
                let next = endpoint(divided, zi, 16);
                let step = if remaining != 0 {
                    std::array::from_fn(|i| (next[i] - current[i]) >> 4)
                } else if n > 1 {
                    std::array::from_fn(|i| (next[i] - current[i]) / i64::from(n - 1))
                } else {
                    [0; 2]
                };
                current = current.map(|value| value & mask);
                for offset in 0..n {
                    expected.push((x + offset, current));
                    current[0] += step[0];
                    current[1] += step[1];
                }
                x += n;
                current = next;
            }
            let mut perspective = Perspective::new(depth, texture, adjust, [16384; 2], start, y);
            let mut actual = Vec::new();
            walk(
                start,
                count,
                16,
                Some(mask),
                |_, distance, minimum| perspective.endpoint(distance, minimum),
                |x, current| actual.push((x, current)),
            );
            assert_eq!(actual, expected, "count={count}, direction={direction}");
        }
    }
}
