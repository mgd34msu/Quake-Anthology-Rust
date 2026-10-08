//! Packed surface-cache consumption. Cache payloads are validated and pinned
//! by SurfaceCache::rgba_pixels before this module receives their borrowed view.
use super::{Buffers, Planes};
use crate::edges::Span;
use crate::surface_cache::CacheSpan;

#[derive(Clone, Copy)]
struct Grid<'a> {
    texels: &'a [[u8; 4]],
    dimensions: [u32; 2],
    minimum: [f32; 2],
    step: f32,
}

impl<'a> Grid<'a> {
    fn from_cache(block: CacheSpan, texels: &'a [[u8; 4]]) -> Self {
        Self {
            texels,
            dimensions: [block.width, block.height],
            minimum: block.texture_mins.map(|value| value as f32),
            step: (1u64 << block.mip) as f32,
        }
    }

    fn pixel(self, chart: [f32; 2]) -> u32 {
        let coordinate: [usize; 2] = std::array::from_fn(|axis| {
            (((chart[axis] - self.minimum[axis]) / self.step) as usize)
                .min(self.dimensions[axis] as usize - 1)
        });
        u32::from_le_bytes(self.texels[coordinate[1] * self.dimensions[0] as usize + coordinate[0]])
    }
}

/// Consume a validated, row-local span over a pinned RGBA cache payload.
/// Returns the number of pixels written; the caller merges its counters.
pub(super) fn cached_rgba_span(
    width: u32,
    span: Span,
    planes: &Planes,
    draw_rank: u32,
    block: CacheSpan,
    texels: &[[u8; 4]],
    buffers: &mut Buffers<'_>,
) -> u32 {
    if span.count == 0 {
        return 0;
    }
    let grid = Grid::from_cache(block, texels);
    #[cfg(target_arch = "x86_64")]
    if grid.dimensions[0] <= i16::MAX as u32
        && grid.dimensions[1] <= i16::MAX as u32
        && span.x <= i32::MAX as u32
        && span.count <= i32::MAX as u32 - span.x
    {
        // x86_64 provides SSE2. The cache boundary validated nonzero dimensions
        // and exact payload length; the edge scanner owns span/band bounds.
        return unsafe { sse2_span(width, span, planes, draw_rank, grid, buffers) };
    }
    scalar_span(width, span, planes, draw_rank, grid, buffers)
}

fn scalar_span(
    width: u32,
    span: Span,
    planes: &Planes,
    draw_rank: u32,
    grid: Grid<'_>,
    buffers: &mut Buffers<'_>,
) -> u32 {
    let mut written = 0;
    for x in span.x..span.x + span.count {
        let zi = planes.inverse_depth.at(x as f32, span.y as f32);
        let index = buffers.offset(width, x, span.y);
        if zi <= 0.0
            || !zi.is_finite()
            || !(zi > buffers.inverse_depth[index]
                || (zi == buffers.inverse_depth[index] && draw_rank >= buffers.depth_ranks[index]))
        {
            continue;
        }
        let chart = planes
            .texture
            .map(|plane| plane.at(x as f32, span.y as f32) / zi);
        buffers.pixels[index] = grid.pixel(chart);
        buffers.palettes[index] = u32::MAX;
        buffers.inverse_depth[index] = zi;
        buffers.depth_ranks[index] = draw_rank;
        written += 1;
    }
    written
}

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Safety: each complete four-lane block lies inside the validated row span
/// in every destination slice. Grid dimensions are nonzero and <= i16::MAX,
/// and the pinned texel slice contains their exact row-major product. Signed
/// 16-bit row/column operands and madd products therefore remain representable.
#[cfg(target_arch = "x86_64")]
unsafe fn sse2_span(
    width: u32,
    span: Span,
    planes: &Planes,
    draw_rank: u32,
    grid: Grid<'_>,
    buffers: &mut Buffers<'_>,
) -> u32 {
    unsafe {
        let zero = _mm_setzero_ps();
        let zero_i = _mm_setzero_si128();
        let one = _mm_set1_ps(1.0);
        let infinity = _mm_set1_ps(f32::INFINITY);
        let magnitude_bits = _mm_castsi128_ps(_mm_set1_epi32(i32::MAX));
        let sign_bits = _mm_set1_epi32(i32::MIN);
        let incoming_rank = _mm_set1_epi32(draw_rank as i32);
        let unsigned_rank = _mm_xor_si128(incoming_rank, sign_bits);
        let palette = _mm_set1_epi32(-1);
        let step = _mm_set1_ps(grid.step);
        let minimum = grid.minimum.map(|value| _mm_set1_ps(value));
        let upper = grid
            .dimensions
            .map(|dimension| _mm_set1_ps((dimension - 1) as f32));
        let stride = _mm_set1_epi32(((1u32 << 16) | grid.dimensions[0]) as i32);
        let row = span.y as f32;
        // Preserve Affine::at's two additions and their order. No plane values
        // advance by repeated floating-point addition across pixels.
        let depth_row = _mm_set1_ps(planes.inverse_depth.origin + planes.inverse_depth.y * row);
        let depth_x = _mm_set1_ps(planes.inverse_depth.x);
        let texture_row = planes
            .texture
            .map(|plane| _mm_set1_ps(plane.origin + plane.y * row));
        let texture_x = planes.texture.map(|plane| _mm_set1_ps(plane.x));
        let first = buffers.offset(width, span.x, span.y);
        let vector_count = span.count / 4 * 4;
        let mut cursor = 0;
        let mut written = 0;
        while cursor < vector_count {
            let x = span.x + cursor;
            let xs = _mm_cvtepi32_ps(_mm_setr_epi32(
                x as i32,
                (x + 1) as i32,
                (x + 2) as i32,
                (x + 3) as i32,
            ));
            let zi = _mm_add_ps(depth_row, _mm_mul_ps(depth_x, xs));
            let finite_positive = _mm_and_ps(
                _mm_cmpgt_ps(zi, zero),
                _mm_cmplt_ps(_mm_and_ps(zi, magnitude_bits), infinity),
            );
            if _mm_movemask_ps(finite_positive) == 0 {
                cursor += 4;
                continue;
            }
            let index = first + cursor as usize;
            let old_depth = _mm_loadu_ps(buffers.inverse_depth.as_ptr().add(index));
            let old_rank = _mm_loadu_si128(buffers.depth_ranks.as_ptr().add(index).cast());
            let rank_ge = _mm_or_si128(
                _mm_cmpgt_epi32(unsigned_rank, _mm_xor_si128(old_rank, sign_bits)),
                _mm_cmpeq_epi32(incoming_rank, old_rank),
            );
            let mask = _mm_and_ps(
                finite_positive,
                _mm_or_ps(
                    _mm_cmpgt_ps(zi, old_depth),
                    _mm_and_ps(_mm_cmpeq_ps(zi, old_depth), _mm_castsi128_ps(rank_ge)),
                ),
            );
            let mask_bits = _mm_movemask_ps(mask) as u32;
            if mask_bits == 0 {
                cursor += 4;
                continue;
            }
            // Rejected lanes use benign operands and still resolve to an
            // owned cache texel. Their framebuffer bits are preserved below.
            let denominator = _mm_or_ps(_mm_and_ps(mask, zi), _mm_andnot_ps(mask, one));
            let coordinates: [__m128i; 2] = std::array::from_fn(|axis| {
                let numerator = _mm_add_ps(texture_row[axis], _mm_mul_ps(texture_x[axis], xs));
                let chart = _mm_div_ps(_mm_and_ps(mask, numerator), denominator);
                let lattice = _mm_div_ps(_mm_sub_ps(chart, minimum[axis]), step);
                // Operand order matters: MAX(NaN, zero) yields zero. This
                // matches Rust's saturating cast before the integer upper cap.
                let bounded = _mm_min_ps(_mm_max_ps(lattice, zero), upper[axis]);
                _mm_cvttps_epi32(bounded)
            });
            let columns = _mm_packs_epi32(coordinates[0], zero_i);
            let rows = _mm_packs_epi32(coordinates[1], zero_i);
            let offsets = _mm_madd_epi16(_mm_unpacklo_epi16(rows, columns), stride);
            let mut addresses = [0i32; 4];
            _mm_storeu_si128(addresses.as_mut_ptr().cast(), offsets);
            let packed = addresses.map(|address| {
                u32::from_le_bytes(*grid.texels.get_unchecked(address as usize)) as i32
            });
            let pixels = _mm_setr_epi32(packed[0], packed[1], packed[2], packed[3]);
            let mask_i = _mm_castps_si128(mask);
            if mask_bits == 15 {
                _mm_storeu_si128(buffers.pixels.as_mut_ptr().add(index).cast(), pixels);
                _mm_storeu_ps(buffers.inverse_depth.as_mut_ptr().add(index), zi);
                _mm_storeu_si128(
                    buffers.depth_ranks.as_mut_ptr().add(index).cast(),
                    incoming_rank,
                );
                _mm_storeu_si128(buffers.palettes.as_mut_ptr().add(index).cast(), palette);
            } else {
                let old_pixels = _mm_loadu_si128(buffers.pixels.as_ptr().add(index).cast());
                let old_palette = _mm_loadu_si128(buffers.palettes.as_ptr().add(index).cast());
                _mm_storeu_si128(
                    buffers.pixels.as_mut_ptr().add(index).cast(),
                    _mm_or_si128(
                        _mm_and_si128(mask_i, pixels),
                        _mm_andnot_si128(mask_i, old_pixels),
                    ),
                );
                _mm_storeu_ps(
                    buffers.inverse_depth.as_mut_ptr().add(index),
                    _mm_or_ps(_mm_and_ps(mask, zi), _mm_andnot_ps(mask, old_depth)),
                );
                _mm_storeu_si128(
                    buffers.depth_ranks.as_mut_ptr().add(index).cast(),
                    _mm_or_si128(
                        _mm_and_si128(mask_i, incoming_rank),
                        _mm_andnot_si128(mask_i, old_rank),
                    ),
                );
                _mm_storeu_si128(
                    buffers.palettes.as_mut_ptr().add(index).cast(),
                    _mm_or_si128(
                        _mm_and_si128(mask_i, palette),
                        _mm_andnot_si128(mask_i, old_palette),
                    ),
                );
            }
            written += mask_bits.count_ones();
            cursor += 4;
        }
        written
            + scalar_span(
                width,
                Span {
                    x: span.x + vector_count,
                    count: span.count - vector_count,
                    ..span
                },
                planes,
                draw_rank,
                grid,
                buffers,
            )
    }
}

#[cfg(test)]
#[path = "../../../tests/cpu_packed/oracle.rs"]
mod tests;
