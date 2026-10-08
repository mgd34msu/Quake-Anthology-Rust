use qa_render::edges::{DepthPolicy, Edges, ProjectedVertex, Span};
use qa_render::scene::Viewport;

const WIDTH: u32 = 48;
const HEIGHT: u32 = 40;

struct Polygon {
    surface: u32,
    key: u32,
    rank: u32,
    vertices: Vec<ProjectedVertex>,
}

fn polygon(surface: u32, key: u32, rank: u32, points: &[[f32; 2]], plane: [f32; 3]) -> Polygon {
    Polygon {
        surface,
        key,
        rank,
        vertices: points
            .iter()
            .map(|&xy| ProjectedVertex {
                xy,
                inverse_depth: plane[0] * xy[0] + plane[1] * xy[1] + plane[2],
                texcoord_over_depth: [0.0; 2],
            })
            .collect(),
    }
}

fn rect(surface: u32, key: u32, rank: u32, bounds: [f32; 4], plane: [f32; 3]) -> Polygon {
    let [left, top, right, bottom] = bounds;
    polygon(
        surface,
        key,
        rank,
        &[[left, top], [right, top], [right, bottom], [left, bottom]],
        plane,
    )
}

fn view(x: u32, y: u32, width: u32, height: u32) -> Viewport {
    Viewport {
        x,
        y,
        width,
        height,
    }
}

fn record(
    pixels: &mut [Option<u32>],
    spans: &[Span],
    viewport: Viewport,
    rows: std::ops::Range<u32>,
) {
    for span in spans {
        assert!(span.count != 0);
        assert!(rows.contains(&span.y));
        assert!(span.x >= viewport.x);
        assert!(span.x + span.count <= viewport.x + viewport.width);
        for x in span.x..span.x + span.count {
            let pixel = &mut pixels[(span.y * WIDTH + x) as usize];
            assert!(
                pixel.is_none(),
                "a pixel was written by multiple spans or bands"
            );
            *pixel = Some(span.surface);
        }
    }
}

fn render(
    viewport: Viewport,
    polygons: &[Polygon],
    policy: DepthPolicy,
    band_height: Option<u32>,
    span_capacity: usize,
) -> Vec<Option<u32>> {
    let mut edges = Edges::load(WIDTH, HEIGHT, 128, 32, span_capacity).unwrap();
    let mut pixels = vec![None; (WIDTH * HEIGHT) as usize];
    let bottom = viewport.y + viewport.height;
    let mut top = viewport.y;
    let mut measured_pixels = 0;
    while top < bottom {
        let end = (top + band_height.unwrap_or(viewport.height)).min(bottom);
        if band_height.is_some() {
            assert!(edges.begin_band(viewport, top..end, policy));
        } else {
            assert!(edges.begin_with_policy(viewport, policy));
        }
        for polygon in polygons {
            assert!(edges.add_polygon(
                polygon.surface,
                polygon.key,
                polygon.rank,
                &polygon.vertices
            ));
        }
        let stats = edges.scan(|spans| record(&mut pixels, spans, viewport, top..end));
        assert_eq!(stats.rejected, 0);
        measured_pixels += stats.pixels;
        top = end;
    }
    assert_eq!(
        measured_pixels,
        pixels.iter().filter(|pixel| pixel.is_some()).count() as u64
    );
    pixels
}

fn compare_bands(
    viewport: Viewport,
    polygons: &[Polygon],
    policy: DepthPolicy,
) -> Vec<Option<u32>> {
    let serial = render(viewport, polygons, policy, None, 1);
    for capacity in [1, 3, 128] {
        for band_height in [1, 2, 4, 8] {
            assert_eq!(
                render(viewport, polygons, policy, Some(band_height), capacity),
                serial
            );
        }
    }
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let inside = x >= viewport.x
                && x < viewport.x + viewport.width
                && y >= viewport.y
                && y < viewport.y + viewport.height;
            assert_eq!(serial[(y * WIDTH + x) as usize].is_some(), inside);
        }
    }
    serial
}

#[test]
fn clipped_slopes_keep_global_rows_and_full_offset_viewport_trajectory() {
    let viewport = view(5, 3, 32, 29);
    let polygons = [
        rect(90, 100, 0, [-30.0, -20.0, 70.0, 60.0], [0.0, 0.0, 0.125]),
        polygon(
            10,
            5,
            1,
            &[[-9.25, -2.75], [42.25, 9.5], [6.125, 36.75]],
            [0.0, 0.0, 0.25],
        ),
        polygon(
            20,
            2,
            2,
            &[[18.25, -8.5], [47.75, 24.25], [3.5, 36.5]],
            [0.0, 0.0, 0.5],
        ),
    ];
    for policy in [DepthPolicy::BspKeys, DepthPolicy::PlaneDepth] {
        compare_bands(viewport, &polygons, policy);
    }
}

#[test]
fn negative_fixed_step_advances_without_recomputing_a_float_intercept() {
    let viewport = view(0, 0, 16, 30);
    let polygons = [
        rect(90, 10, 0, [0.0, 0.0, 16.0, 30.0], [0.0, 0.0, 0.125]),
        polygon(
            10,
            1,
            1,
            &[[3.0, 0.0], [15.0, 0.0], [15.0, 30.0], [0.0, 30.0]],
            [0.0, 0.0, 0.25],
        ),
    ];
    for policy in [DepthPolicy::BspKeys, DepthPolicy::PlaneDepth] {
        let pixels = compare_bands(viewport, &polygons, policy);
        // Native -0.1 * 2^20 truncates toward zero. Ten integer steps put the
        // biased boundary just above x=2; a fresh float intercept would use 2.
        assert_eq!(pixels[(10 * WIDTH + 2) as usize], Some(90));
        assert_eq!(pixels[(10 * WIDTH + 3) as usize], Some(10));
    }
}

#[test]
fn crossing_planes_and_rank_ties_sample_full_global_depth_coordinates() {
    let viewport = view(3, 4, 32, 28);
    let bounds = [3.0, 4.0, 35.0, 32.0];
    let recipes = [
        (10, 0, 1, [-0.015625, -0.0078125, 1.5]),
        (20, 9000, 2, [0.0, 0.0, 1.125]),
        (30, 1, 3, [0.015625, 0.0078125, 0.75]),
        (40, u32::MAX, 4, [0.0, 0.0, 1.125]),
    ];
    let polygons: Vec<_> = recipes
        .iter()
        .map(|&(surface, key, rank, plane)| rect(surface, key, rank, bounds, plane))
        .collect();
    let pixels = compare_bands(viewport, &polygons, DepthPolicy::PlaneDepth);
    for y in viewport.y..viewport.y + viewport.height {
        for x in viewport.x..viewport.x + viewport.width {
            let winner = recipes
                .iter()
                .max_by(|a, b| {
                    let depth = |recipe: &(u32, u32, u32, [f32; 3])| {
                        f64::from(recipe.3[0]) * f64::from(x)
                            + f64::from(recipe.3[1]) * f64::from(y)
                            + f64::from(recipe.3[2])
                    };
                    depth(a).total_cmp(&depth(b)).then(a.2.cmp(&b.2))
                })
                .unwrap();
            assert_eq!(pixels[(y * WIDTH + x) as usize], Some(winner.0));
        }
    }
}

#[test]
fn shared_diagonal_and_vertical_edges_cover_each_pixel_once() {
    let viewport = view(4, 2, 32, 32);
    for diagonal in [false, true] {
        let polygons = if diagonal {
            [
                polygon(
                    1,
                    3,
                    0,
                    &[[4.0, 2.0], [36.0, 2.0], [4.0, 34.0]],
                    [0.0, 0.0, 0.25],
                ),
                polygon(
                    2,
                    3,
                    0,
                    &[[36.0, 2.0], [36.0, 34.0], [4.0, 34.0]],
                    [0.0, 0.0, 0.25],
                ),
            ]
        } else {
            [
                rect(1, 3, 0, [4.0, 2.0, 20.0, 34.0], [0.0, 0.0, 0.25]),
                rect(2, 3, 0, [20.0, 2.0, 36.0, 34.0], [0.0, 0.0, 0.25]),
            ]
        };
        for policy in [DepthPolicy::BspKeys, DepthPolicy::PlaneDepth] {
            let pixels = compare_bands(viewport, &polygons, policy);
            for y in viewport.y..viewport.y + viewport.height {
                let border = if diagonal { 36 - (y - 2) } else { 20 };
                for x in viewport.x..viewport.x + viewport.width {
                    assert_eq!(
                        pixels[(y * WIDTH + x) as usize],
                        Some(if x < border { 1 } else { 2 })
                    );
                }
            }
        }
    }
}

#[test]
fn carried_aet_ties_keep_crossing_and_native_insertion_history() {
    let viewport = view(0, 0, 8, 32);
    let scenarios = [
        // Left edges meet at y=8. The existing AET retains the rising edge
        // before the vertical edge at equality, then reorders after crossing.
        vec![
            rect(90, 100, 0, [0.0, 0.0, 8.0, 32.0], [0.0, 0.0, 0.125]),
            polygon(
                10,
                4,
                0,
                &[[1.0, 0.0], [7.0, 0.0], [7.0, 32.0], [5.0, 32.0]],
                [0.0, 0.0, 0.25],
            ),
            rect(20, 4, 0, [2.0, 0.0, 6.0, 32.0], [0.0, 0.0, 0.25]),
        ],
        // Coincident trajectories from different rows put the newer edge first.
        vec![
            rect(90, 100, 0, [0.0, 0.0, 8.0, 32.0], [0.0, 0.0, 0.125]),
            rect(10, 4, 0, [1.0, 0.0, 7.0, 32.0], [0.0, 0.0, 0.25]),
            rect(20, 4, 0, [1.0, 8.0, 6.0, 32.0], [0.0, 0.0, 0.25]),
        ],
        // At the same birth row, later equal-U leaders precede earlier ones.
        vec![
            rect(90, 100, 0, [0.0, 0.0, 8.0, 32.0], [0.0, 0.0, 0.125]),
            rect(10, 4, 0, [1.0, 0.0, 7.0, 32.0], [0.0, 0.0, 0.25]),
            rect(20, 4, 0, [1.0, 0.0, 6.0, 32.0], [0.0, 0.0, 0.25]),
        ],
    ];
    for policy in [DepthPolicy::BspKeys, DepthPolicy::PlaneDepth] {
        for (scenario, polygons) in scenarios.iter().enumerate() {
            let pixels = compare_bands(viewport, polygons, policy);
            for y in 0..32 {
                let expected = match scenario {
                    0 if y <= 8 => 10,
                    0 => 20,
                    1 if y < 8 => 10,
                    _ => 20,
                };
                assert_eq!(pixels[(y * WIDTH + 2) as usize], Some(expected));
            }
        }
    }
}

#[test]
fn band_bounds_and_polygon_capacity_failures_preserve_queued_full_view_work() {
    let viewport = view(0, 0, 8, 6);
    let background = rect(7, 9, 0, [0.0, 0.0, 8.0, 6.0], [0.0, 0.0, 0.125]);
    let mut edges = Edges::load(WIDTH, HEIGHT, 2, 2, 1).unwrap();
    assert!(edges.begin(viewport));
    assert!(edges.add_polygon(
        background.surface,
        background.key,
        background.rank,
        &background.vertices
    ));
    for (full_view, rows) in [
        (viewport, 3..3),
        (viewport, 4..2),
        (viewport, 0..7),
        (view(0, 2, 8, 4), 1..4),
        (view(0, 0, 49, 6), 0..6),
        (view(0, u32::MAX, 8, 2), 0..1),
    ] {
        assert!(!edges.begin_band(full_view, rows, DepthPolicy::PlaneDepth));
    }
    let extra = rect(9, 0, 1, [1.0, 1.0, 7.0, 5.0], [0.0, 0.0, 0.25]);
    assert!(!edges.add_polygon(extra.surface, extra.key, extra.rank, &extra.vertices));
    let mut pixels = vec![None; (WIDTH * HEIGHT) as usize];
    let stats = edges.scan(|spans| record(&mut pixels, spans, viewport, 0..6));
    assert_eq!(stats.rejected, 7);
    assert_eq!(stats.polygons, 1);
    assert_eq!(stats.edges, 2);
    assert_eq!(stats.pixels, 48);
    assert_eq!(stats.flushes, 6);
    for y in 0..6 {
        for x in 0..8 {
            assert_eq!(pixels[(y * WIDTH + x) as usize], Some(7));
        }
    }
    assert!(edges.begin_band(viewport, 2..4, DepthPolicy::BspKeys));
    assert!(edges.add_polygon(
        background.surface,
        background.key,
        background.rank,
        &background.vertices
    ));
    let mut band = vec![None; (WIDTH * HEIGHT) as usize];
    let stats = edges.scan(|spans| record(&mut band, spans, viewport, 2..4));
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.pixels, 16);
    assert_eq!(stats.flushes, 2);
}
