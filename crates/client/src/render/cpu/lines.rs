//! CPU aliased line rasterization.
//!
//! Donor provenance: `src/render/cpu/lines.ts` in full — a CPU
//! implementation of OpenGL 2.1 sections 3.4.1 and 3.4.2 with diamond-exit
//! coverage, half-open endpoints, and minor-axis width replication.

use qa_core::math::{Vec2, Vec3, Vec4};

use super::lighting::CpuVertex;

/// One rasterized line fragment in viewport-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineFragment {
    /// Viewport-local X.
    pub x: i32,
    /// Viewport-local Y, bottom-up like the donor.
    pub y: i32,
    /// Window depth.
    pub depth: f32,
    /// Eye depth for fog.
    pub eye_depth: f32,
    /// World-space position.
    pub world_position: Vec3,
    /// World-space normal.
    pub world_normal: Vec3,
    /// Interpolated color.
    pub color: Vec4,
    /// Primary texture coordinates.
    pub tex_coord: Vec2,
    /// Secondary texture coordinates.
    pub tex_coord2: Vec2,
    /// Primary per-pixel texture derivatives.
    pub tex_coord_derivative: LineTextureDerivative,
    /// Secondary per-pixel texture derivatives.
    pub tex_coord2_derivative: LineTextureDerivative,
}

/// Per-pixel texture-coordinate derivatives.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LineTextureDerivative {
    /// `ds` per pixel.
    pub ds_per_pixel: f32,
    /// `dt` per pixel.
    pub dt_per_pixel: f32,
}

/// Viewport-local scissor rectangle, top-down like the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineScissor {
    /// Minimum X.
    pub min_x: i32,
    /// Minimum Y.
    pub min_y: i32,
    /// Maximum X.
    pub max_x: i32,
    /// Maximum Y.
    pub max_y: i32,
}

fn interpolate(a: &CpuVertex, b: &CpuVertex, t: f32) -> CpuVertex {
    let mix = |x: f32, y: f32| x * (1.0 - t) + y * t;
    let coordinate = |left: f32, right: f32| left + (right - left) * t;
    CpuVertex {
        position: Vec4 {
            x: mix(a.position.x, b.position.x),
            y: mix(a.position.y, b.position.y),
            z: mix(a.position.z, b.position.z),
            w: mix(a.position.w, b.position.w),
        },
        world_position: Vec3 {
            x: mix(a.world_position.x, b.world_position.x),
            y: mix(a.world_position.y, b.world_position.y),
            z: mix(a.world_position.z, b.world_position.z),
        },
        world_normal: Vec3 {
            x: mix(a.world_normal.x, b.world_normal.x),
            y: mix(a.world_normal.y, b.world_normal.y),
            z: mix(a.world_normal.z, b.world_normal.z),
        },
        color: Vec4 {
            x: mix(a.color.x, b.color.x),
            y: mix(a.color.y, b.color.y),
            z: mix(a.color.z, b.color.z),
            w: mix(a.color.w, b.color.w),
        },
        tex_coord: Vec2 {
            x: coordinate(a.tex_coord.x, b.tex_coord.x),
            y: coordinate(a.tex_coord.y, b.tex_coord.y),
        },
        tex_coord2: Vec2 {
            x: coordinate(a.tex_coord2.x, b.tex_coord2.x),
            y: coordinate(a.tex_coord2.y, b.tex_coord2.y),
        },
    }
}

fn clip(first: &CpuVertex, second: &CpuVertex) -> Option<(CpuVertex, CpuVertex)> {
    let (mut a, mut b) = (*first, *second);
    let magnitude = a
        .position
        .x
        .abs()
        .max(a.position.y.abs())
        .max(a.position.z.abs())
        .max(a.position.w.abs())
        .max(b.position.x.abs())
        .max(b.position.y.abs())
        .max(b.position.z.abs())
        .max(b.position.w.abs());
    if magnitude > f32::MAX / 4.0 {
        let rescale = |vertex: &CpuVertex| {
            let mut scaled = *vertex;
            scaled.position.x /= magnitude;
            scaled.position.y /= magnitude;
            scaled.position.z /= magnitude;
            scaled.position.w /= magnitude;
            scaled
        };
        a = rescale(&a);
        b = rescale(&b);
    }
    let (mut begin, mut end) = (0.0f32, 1.0f32);
    let distance = |p: &Vec4| [p.w + p.x, p.w - p.x, p.w + p.y, p.w - p.y, p.w + p.z, p.w - p.z];
    let (dist_a, dist_b) = (distance(&a.position), distance(&b.position));
    for plane in 0..6 {
        let (x, y) = (dist_a[plane], dist_b[plane]);
        if x < 0.0 && y < 0.0 {
            return None;
        }
        if (x < 0.0) != (y < 0.0) {
            let scale = x.abs().max(y.abs());
            let crossing = (x / scale) / (x / scale - y / scale);
            if x < 0.0 {
                begin = begin.max(crossing);
            } else {
                end = end.min(crossing);
            }
        }
    }
    if begin >= end {
        return None;
    }
    let (start, finish) = (interpolate(&a, &b, begin), interpolate(&a, &b, end));
    if start.position.w <= 0.0 || finish.position.w <= 0.0 {
        return None;
    }
    Some((start, finish))
}

fn exits_diamond(ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) -> bool {
    let (mut enter, mut exit) = (0.0f64, 1.0f64);
    for sx in [-1.0f64, 1.0] {
        for sy in [-1.0f64, 1.0] {
            let a = 0.5 - sx * (ax - x) - sy * (ay - y);
            let b = 0.5 - sx * (bx - x) - sy * (by - y);
            if a <= 0.0 && b <= 0.0 {
                return false;
            }
            if (a <= 0.0) != (b <= 0.0) {
                let t = a / (a - b);
                if a <= 0.0 {
                    enter = enter.max(t);
                } else {
                    exit = exit.min(t);
                }
            }
        }
    }
    enter < exit && exit < 1.0
}

/// Rasterize one aliased line with diamond-exit coverage.
///
/// `width`/`height` are the viewport dimensions; the scissor is top-down
/// while emitted fragments are bottom-up, matching the donor.
pub fn rasterize_aliased_line(
    first: &CpuVertex,
    second: &CpuVertex,
    width: f32,
    height: f32,
    line_width: f32,
    scissor: &LineScissor,
    emit: &mut dyn FnMut(LineFragment),
) {
    if scissor.min_x > scissor.max_x || scissor.min_y > scissor.max_y {
        return;
    }
    let Some((a, b)) = clip(first, second) else {
        return;
    };
    // Window-space setup runs in binary64 like the donor: the endpoint
    // perturbation breaks ties far below binary32 resolution.
    let (width, height) = (f64::from(width), f64::from(height));
    let ax = (f64::from(a.position.x) / f64::from(a.position.w) + 1.0) * width / 2.0;
    let ay = (f64::from(a.position.y) / f64::from(a.position.w) + 1.0) * height / 2.0;
    let bx = (f64::from(b.position.x) / f64::from(b.position.w) + 1.0) * width / 2.0;
    let by = (f64::from(b.position.y) / f64::from(b.position.w) + 1.0) * height / 2.0;
    let (dx, dy) = (bx - ax, by - ay);
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return;
    }
    let length = length_squared.sqrt();
    let (inverse_wa, inverse_wb) = (1.0 / f64::from(a.position.w), 1.0 / f64::from(b.position.w));
    let delta = (
        f64::from(b.tex_coord.x) - f64::from(a.tex_coord.x),
        f64::from(b.tex_coord.y) - f64::from(a.tex_coord.y),
    );
    let delta2 = (
        f64::from(b.tex_coord2.x) - f64::from(a.tex_coord2.x),
        f64::from(b.tex_coord2.y) - f64::from(a.tex_coord2.y),
    );
    let x_major = dx.abs() >= dy.abs();
    let thickness = 1.max((f64::from(line_width) + 0.5).floor() as i32);
    let shift = f64::from(thickness - 1) / 2.0;
    // The spec's endpoint perturbation chooses boundary ownership consistently.
    let (pa_x, pa_y) = (
        ax - (if x_major { 0.0 } else { shift }) - 1e-5,
        ay - (if x_major { shift } else { 0.0 }) - 1e-10,
    );
    let (pb_x, pb_y) = (
        bx - (if x_major { 0.0 } else { shift }) - 1e-5,
        by - (if x_major { shift } else { 0.0 }) - 1e-10,
    );
    let (major_a, major_b) = if x_major { (pa_x, pb_x) } else { (pa_y, pb_y) };
    let (minor_a, minor_b) = if x_major { (pa_y, pb_y) } else { (pa_x, pb_x) };
    let height_i = height as i32;
    let (bottom, top) = (height_i - 1 - scissor.max_y, height_i - 1 - scissor.min_y);
    let (major_min, major_max) = if x_major {
        (scissor.min_x, scissor.max_x)
    } else {
        (bottom, top)
    };
    let (minor_min, minor_max) = if x_major {
        (bottom, top)
    } else {
        (scissor.min_x, scissor.max_x)
    };
    for major in major_min.max(major_a.min(major_b).floor() as i32)..=major_max.min(major_a.max(major_b).floor() as i32)
    {
        let fraction = (f64::from(major) + 0.5 - major_a) / (major_b - major_a);
        let minor_center = (minor_a + (minor_b - minor_a) * fraction).floor() as i32;
        for minor in minor_center - 1..=minor_center + 1 {
            let (x, y) = if x_major { (major, minor) } else { (minor, major) };
            if !exits_diamond(pa_x, pa_y, pb_x, pb_y, f64::from(x) + 0.5, f64::from(y) + 0.5) {
                continue;
            }
            // Wide-line fragments replicate one base fragment, including its attributes.
            let (base_x, base_y) = (
                f64::from(x) + (if x_major { 0.0 } else { shift }),
                f64::from(y) + (if x_major { shift } else { 0.0 }),
            );
            let t = (((base_x + 0.5 - ax) * dx + (base_y + 0.5 - ay) * dy) / length_squared).clamp(0.0, 1.0);
            let inverse_w = (1.0 - t) * inverse_wa + t * inverse_wb;
            let value = |left: f32, right: f32| {
                ((f64::from(left) * (1.0 - t) * inverse_wa + f64::from(right) * t * inverse_wb) / inverse_w) as f32
            };
            let wb = f64::from(b.position.w);
            let residual = (delta.0 * t / wb / inverse_w, delta.1 * t / wb / inverse_w);
            let residual2 = (delta2.0 * t / wb / inverse_w, delta2.1 * t / wb / inverse_w);
            let tex_coord = Vec2 {
                x: (f64::from(a.tex_coord.x) + residual.0) as f32,
                y: (f64::from(a.tex_coord.y) + residual.1) as f32,
            };
            let tex_coord2 = Vec2 {
                x: (f64::from(a.tex_coord2.x) + residual2.0) as f32,
                y: (f64::from(a.tex_coord2.y) + residual2.1) as f32,
            };
            // OpenGL 2.1 equation 3.22: differentiate the perspective quotient along
            // this clipped segment, then normalize by its window-space length.
            // Keep the first endpoint's common offset out of the quotient derivative.
            let derivative = |difference: f64, quotient: f64| {
                ((difference * inverse_wb - quotient * (inverse_wb - inverse_wa)) / inverse_w / length) as f32
            };
            let tex_coord_derivative = LineTextureDerivative {
                ds_per_pixel: derivative(delta.0, residual.0),
                dt_per_pixel: derivative(delta.1, residual.1),
            };
            let tex_coord2_derivative = LineTextureDerivative {
                ds_per_pixel: derivative(delta2.0, residual2.0),
                dt_per_pixel: derivative(delta2.1, residual2.1),
            };
            for actual_minor in minor_min.max(minor)..=minor_max.min(minor + thickness - 1) {
                let (actual_x, actual_y) = if x_major {
                    (major, actual_minor)
                } else {
                    (actual_minor, major)
                };
                emit(LineFragment {
                    x: actual_x,
                    y: height_i - 1 - actual_y,
                    eye_depth: (1.0 / inverse_w).abs() as f32,
                    world_position: Vec3 {
                        x: value(a.world_position.x, b.world_position.x),
                        y: value(a.world_position.y, b.world_position.y),
                        z: value(a.world_position.z, b.world_position.z),
                    },
                    world_normal: Vec3 {
                        x: value(a.world_normal.x, b.world_normal.x),
                        y: value(a.world_normal.y, b.world_normal.y),
                        z: value(a.world_normal.z, b.world_normal.z),
                    },
                    depth: (((1.0 - t) * f64::from(a.position.z) * inverse_wa
                        + t * f64::from(b.position.z) * inverse_wb)
                        * 0.5
                        + 0.5) as f32,
                    color: Vec4 {
                        x: value(a.color.x, b.color.x),
                        y: value(a.color.y, b.color.y),
                        z: value(a.color.z, b.color.z),
                        w: value(a.color.w, b.color.w),
                    },
                    tex_coord,
                    tex_coord2,
                    tex_coord_derivative,
                    tex_coord2_derivative,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec2, vec3, vec4};

    use super::*;

    fn vertex(x: f32, y: f32) -> CpuVertex {
        CpuVertex {
            position: vec4(x, y, 0.0, 1.0),
            world_position: vec3(0.0, 0.0, 0.0),
            world_normal: vec3(0.0, 0.0, 1.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            tex_coord: vec2(0.0, 0.0),
            tex_coord2: vec2(0.0, 0.0),
        }
    }

    #[test]
    fn horizontal_line_covers_one_row() {
        // NDC x -1..1 maps to window 0..4; y=0 sits on the window row boundary
        // and the perturbation assigns bottom-up row 1, emitted top-down as 2.
        let scissor = LineScissor {
            min_x: 0,
            min_y: 0,
            max_x: 3,
            max_y: 3,
        };
        let mut fragments = Vec::new();
        rasterize_aliased_line(
            &vertex(-1.0, 0.0),
            &vertex(1.0, 0.0),
            4.0,
            4.0,
            1.0,
            &scissor,
            &mut |fragment| {
                fragments.push((fragment.x, fragment.y, fragment.depth));
            },
        );
        fragments.sort_by_key(|(x, y, _)| (*x, *y));
        let coords: Vec<(i32, i32)> = fragments.iter().map(|(x, y, _)| (*x, *y)).collect();
        assert_eq!(coords, vec![(0, 2), (1, 2), (2, 2), (3, 2)]);
        for (_, _, depth) in &fragments {
            assert_eq!(*depth, 0.5);
        }
    }

    #[test]
    fn clipped_line_stops_at_frustum() {
        let scissor = LineScissor {
            min_x: 0,
            min_y: 0,
            max_x: 3,
            max_y: 3,
        };
        let mut fragments = Vec::new();
        rasterize_aliased_line(
            &vertex(-2.0, 0.0),
            &vertex(0.0, 0.0),
            4.0,
            4.0,
            1.0,
            &scissor,
            &mut |fragment| {
                fragments.push((fragment.x, fragment.y));
            },
        );
        fragments.sort_unstable();
        assert_eq!(fragments, vec![(0, 2), (1, 2)]);
    }

    #[test]
    fn degenerate_and_empty_lines_emit_nothing() {
        let scissor = LineScissor {
            min_x: 0,
            min_y: 0,
            max_x: 3,
            max_y: 3,
        };
        let mut count = 0;
        rasterize_aliased_line(
            &vertex(0.0, 0.0),
            &vertex(0.0, 0.0),
            4.0,
            4.0,
            1.0,
            &scissor,
            &mut |_| count += 1,
        );
        assert_eq!(count, 0);
        let empty = LineScissor {
            min_x: 2,
            min_y: 0,
            max_x: 1,
            max_y: 3,
        };
        rasterize_aliased_line(
            &vertex(-1.0, 0.0),
            &vertex(1.0, 0.0),
            4.0,
            4.0,
            1.0,
            &empty,
            &mut |_| count += 1,
        );
        assert_eq!(count, 0);
    }
}
