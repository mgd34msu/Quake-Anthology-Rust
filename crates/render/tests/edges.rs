use qa_render::edges::{DepthPolicy, Edges, ProjectedVertex, Span, Stats};
use qa_render::scene::Viewport;

fn polygon(points: &[[f32; 2]]) -> Vec<ProjectedVertex> {
    points
        .iter()
        .map(|&xy| ProjectedVertex {
            xy,
            inverse_depth: 0.125,
            texcoord_over_depth: [0.0; 2],
        })
        .collect()
}

fn rect(x: f32, y: f32, right: f32, bottom: f32) -> Vec<ProjectedVertex> {
    polygon(&[[x, y], [right, y], [right, bottom], [x, bottom]])
}

fn view(width: u32, height: u32) -> Viewport {
    Viewport {
        width,
        height,
        ..Viewport::default()
    }
}

fn coverage(
    edges: &mut Edges,
    width: usize,
    height: usize,
) -> (Vec<Option<u32>>, Stats, Vec<Span>) {
    let mut pixels = vec![None; width * height];
    let mut spans = Vec::new();
    let stats = edges.scan(|batch| {
        spans.extend_from_slice(batch);
        for span in batch {
            assert!(span.count > 0);
            assert!(span.y < height as u32);
            assert!(span.x + span.count <= width as u32);
            for x in span.x..span.x + span.count {
                let pixel = &mut pixels[span.y as usize * width + x as usize];
                assert!(pixel.is_none(), "opaque spans overlap");
                *pixel = Some(span.surface);
            }
        }
    });
    (pixels, stats, spans)
}

#[test]
fn native_ceil_rows_and_exclusive_bottom_accept_either_winding() {
    let vertices = rect(0.2, 0.2, 5.2, 3.2);
    let mut reverse = vertices.clone();
    reverse.reverse();
    for vertices in [&vertices, &reverse] {
        let mut edges = Edges::load(8, 6, 8, 2, 16).unwrap();
        assert!(edges.begin(view(8, 6)));
        assert!(edges.add_polygon(7, 1, vertices));
        let (pixels, stats, _) = coverage(&mut edges, 8, 6);
        for y in 0..6 {
            for x in 0..8 {
                assert_eq!(
                    pixels[y * 8 + x],
                    (y >= 1 && y < 4 && x >= 1 && x < 6).then_some(7)
                );
            }
        }
        assert_eq!(stats.pixels, 15);
        assert_eq!(stats.spans, 3);
        assert_eq!(stats.rejected, 0);
    }
}

#[test]
fn near_depth_keys_occlude_far_surfaces_without_overdraw() {
    for near_first in [false, true] {
        let mut edges = Edges::load(8, 6, 8, 4, 32).unwrap();
        assert!(edges.begin(view(8, 6)));
        let far = rect(0.0, 0.0, 8.0, 6.0);
        let near = rect(2.0, 1.0, 6.0, 5.0);
        if near_first {
            assert!(edges.add_polygon(10, 2, &near));
            assert!(edges.add_polygon(30, 20, &far));
        } else {
            assert!(edges.add_polygon(30, 20, &far));
            assert!(edges.add_polygon(10, 2, &near));
        }
        let (pixels, stats, _) = coverage(&mut edges, 8, 6);
        for y in 0..6 {
            for x in 0..8 {
                assert_eq!(
                    pixels[y * 8 + x],
                    Some(if y >= 1 && y < 5 && x >= 2 && x < 6 {
                        10
                    } else {
                        30
                    })
                );
            }
        }
        assert_eq!(stats.pixels, 48);
        assert_eq!(stats.spans, 14);
    }
}

#[test]
fn shared_vertical_and_diagonal_edges_leave_no_holes_or_double_spans() {
    for diagonal in [false, true] {
        let mut edges = Edges::load(6, 6, 8, 4, 32).unwrap();
        assert!(edges.begin(view(6, 6)));
        if diagonal {
            assert!(edges.add_polygon(1, 1, &polygon(&[[0.0, 0.0], [6.0, 0.0], [0.0, 6.0]])));
            assert!(edges.add_polygon(2, 1, &polygon(&[[6.0, 0.0], [6.0, 6.0], [0.0, 6.0]])));
        } else {
            assert!(edges.add_polygon(1, 1, &rect(0.0, 0.0, 3.0, 6.0)));
            assert!(edges.add_polygon(2, 1, &rect(3.0, 0.0, 6.0, 6.0)));
        }
        let (pixels, stats, _) = coverage(&mut edges, 6, 6);
        assert!(pixels.iter().all(Option::is_some));
        assert_eq!(stats.pixels, 36);
        for y in 0..6 {
            for x in 0..6 {
                let border = if diagonal { 6 - y } else { 3 };
                assert_eq!(pixels[y * 6 + x], Some(if x < border { 1 } else { 2 }));
            }
        }
    }
}

#[test]
fn sloped_aet_edges_reorder_when_they_cross() {
    let mut edges = Edges::load(6, 6, 8, 4, 32).unwrap();
    assert!(edges.begin(view(6, 6)));
    assert!(edges.add_polygon(1, 1, &polygon(&[[0.0, 0.0], [6.0, 0.0], [0.0, 6.0]])));
    assert!(edges.add_polygon(2, 2, &polygon(&[[0.0, 0.0], [6.0, 6.0], [0.0, 6.0]])));
    let (pixels, _, _) = coverage(&mut edges, 6, 6);
    for y in 0..6 {
        for x in 0..6 {
            let expected = if x < 6 - y {
                Some(1)
            } else if x < y {
                Some(2)
            } else {
                None
            };
            assert_eq!(pixels[y * 6 + x], expected);
        }
    }
}

#[test]
fn viewport_clips_top_bottom_and_right_without_distorting_slopes() {
    let mut edges = Edges::load(8, 6, 8, 4, 32).unwrap();
    assert!(edges.begin(Viewport {
        x: 2,
        y: 1,
        width: 4,
        height: 4
    }));
    assert!(edges.add_polygon(9, 20, &rect(-20.0, -20.0, 30.0, 30.0)));
    assert!(edges.add_polygon(
        1,
        2,
        &polygon(&[[4.0, 0.0], [8.0, 3.0], [4.0, 6.0], [0.0, 3.0]])
    ));
    let (pixels, stats, _) = coverage(&mut edges, 8, 6);
    for y in 0..6 {
        for x in 0..8 {
            let expected = if y < 1 || y >= 5 || x < 2 || x >= 6 {
                None
            } else if y == 1 && x == 2 {
                Some(9)
            } else {
                Some(1)
            };
            assert_eq!(pixels[y * 8 + x], expected);
        }
    }
    assert_eq!(stats.pixels, 16);
}

#[test]
fn coplanar_ties_keep_active_surface_and_later_initial_leader_wins() {
    let mut edges = Edges::load(8, 2, 8, 4, 32).unwrap();
    assert!(edges.begin(view(8, 2)));
    // Submitted first, but starts later: the existing native stack wins.
    assert!(edges.add_polygon(80, 4, &rect(2.0, 0.0, 6.0, 2.0)));
    assert!(edges.add_polygon(90, 4, &rect(1.0, 0.0, 7.0, 2.0)));
    let (pixels, _, _) = coverage(&mut edges, 8, 2);
    for row in pixels.chunks_exact(8) {
        assert_eq!(
            row,
            &[
                None,
                Some(90),
                Some(90),
                Some(90),
                Some(90),
                Some(90),
                Some(90),
                None
            ]
        );
    }
    assert!(edges.begin(view(8, 2)));
    // R_EmitEdge inserts the later equal-U leader first in the new-edge bucket.
    assert!(edges.add_polygon(10, 4, &rect(1.0, 0.0, 6.0, 2.0)));
    assert!(edges.add_polygon(20, 4, &rect(1.0, 0.0, 7.0, 2.0)));
    let (pixels, _, _) = coverage(&mut edges, 8, 2);
    for row in pixels.chunks_exact(8) {
        assert_eq!(
            row,
            &[
                None,
                Some(20),
                Some(20),
                Some(20),
                Some(20),
                Some(20),
                Some(20),
                None
            ]
        );
    }
}

#[test]
fn new_row_leader_precedes_equal_u_existing_active_edge() {
    let mut edges = Edges::load(8, 3, 8, 4, 32).unwrap();
    assert!(edges.begin(view(8, 3)));
    assert!(edges.add_polygon(10, 4, &rect(1.0, 0.0, 7.0, 3.0)));
    assert!(edges.add_polygon(20, 4, &rect(1.0, 1.0, 6.0, 3.0)));
    let (pixels, _, _) = coverage(&mut edges, 8, 3);
    for y in 0..3 {
        for x in 0..8 {
            let expected = if x < 1 || x >= 7 {
                None
            } else if y != 0 && x < 6 {
                Some(20)
            } else {
                Some(10)
            };
            assert_eq!(pixels[y * 8 + x], expected);
        }
    }
}

#[test]
fn stepped_aet_equal_u_preserves_existing_order_then_crossing_repairs_it() {
    let mut edges = Edges::load(8, 3, 8, 4, 32).unwrap();
    assert!(edges.begin(view(8, 3)));
    assert!(edges.add_polygon(
        10,
        4,
        &polygon(&[[1.0, 0.0], [7.0, 0.0], [7.0, 3.0], [4.0, 3.0],])
    ));
    assert!(edges.add_polygon(20, 4, &rect(2.0, 0.0, 6.0, 3.0)));
    let (pixels, _, _) = coverage(&mut edges, 8, 3);
    for y in 0..3 {
        for x in 0..8 {
            let expected = if y < 2 {
                (x >= 1 + y && x < 7).then_some(10)
            } else if x >= 2 && x < 6 {
                Some(20)
            } else {
                (x == 6).then_some(10)
            };
            assert_eq!(pixels[y * 8 + x], expected);
        }
    }
}

#[test]
fn capacity_failure_is_atomic_and_span_arena_flushes_then_reuses() {
    let mut edges = Edges::load(8, 6, 2, 4, 1).unwrap();
    assert!(edges.begin(view(8, 6)));
    assert!(edges.add_polygon(3, 4, &rect(0.0, 0.0, 8.0, 6.0)));
    assert!(!edges.add_polygon(1, 1, &rect(2.0, 1.0, 6.0, 5.0)));
    let (pixels, stats, spans) = coverage(&mut edges, 8, 6);
    assert!(pixels.iter().all(|&p| p == Some(3)));
    assert_eq!(stats.polygons, 1);
    assert_eq!(stats.edges, 2);
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.flushes, 6);
    assert_eq!(spans.len(), 6);
    assert!(edges.begin(view(8, 6)));
    assert!(edges.add_polygon(5, 2, &rect(1.0, 1.0, 2.0, 2.0)));
    let (pixels, stats, _) = coverage(&mut edges, 8, 6);
    assert_eq!(pixels.iter().filter(|p| p.is_some()).count(), 1);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.flushes, 1);
}

#[test]
fn polygon_budget_and_invalid_geometry_leave_existing_edges_intact() {
    let mut edges = Edges::load(8, 2, 8, 1, 4).unwrap();
    assert!(edges.begin(view(8, 2)));
    assert!(edges.add_polygon(7, 2, &rect(0.0, 0.0, 4.0, 2.0)));
    let mut invalid = rect(4.0, 0.0, 8.0, 2.0);
    invalid[3].xy[0] = f32::NAN;
    assert!(!edges.add_polygon(9, 1, &invalid));
    assert!(!edges.add_polygon(9, 1, &rect(4.0, 0.0, 8.0, 2.0)));
    let (pixels, stats, _) = coverage(&mut edges, 8, 2);
    assert_eq!(stats.rejected, 2);
    for row in pixels.chunks_exact(8) {
        assert_eq!(
            row,
            &[Some(7), Some(7), Some(7), Some(7), None, None, None, None]
        );
    }
    assert!(!edges.begin(Viewport {
        x: u32::MAX,
        width: 2,
        height: 1,
        ..Viewport::default()
    }));
    let mut flushed = false;
    assert_eq!(edges.scan(|_| flushed = true).rejected, 1);
    assert!(!flushed);
}

#[test]
fn widened_native_bias_preserves_integer_boundaries_at_large_resolutions() {
    let mut edges = Edges::load(8192, 1, 4, 2, 2).unwrap();
    assert!(edges.begin(view(8192, 1)));
    assert!(edges.add_polygon(4, 1, &rect(7000.0, 0.0, 7016.0, 1.0)));
    let (_, stats, spans) = coverage(&mut edges, 8192, 1);
    assert_eq!(stats.pixels, 16);
    assert_eq!(
        spans,
        [Span {
            surface: 4,
            x: 7000,
            y: 0,
            count: 16
        }]
    );
}

fn plane_polygon(points: &[[f32; 2]], plane: [f32; 3]) -> Vec<ProjectedVertex> {
    let mut vertices = polygon(points);
    for vertex in &mut vertices {
        vertex.inverse_depth = plane[0] * vertex.xy[0] + plane[1] * vertex.xy[1] + plane[2];
    }
    vertices
}

fn plane_rect(x: f32, y: f32, right: f32, bottom: f32, plane: [f32; 3]) -> Vec<ProjectedVertex> {
    plane_polygon(&[[x, y], [right, y], [right, bottom], [x, bottom]], plane)
}

#[test]
fn plane_depth_ignores_misleading_partition_keys() {
    for policy in [DepthPolicy::BspKeys, DepthPolicy::PlaneDepth] {
        let mut edges = Edges::load(8, 2, 8, 4, 16).unwrap();
        assert!(edges.begin_with_policy(view(8, 2), policy));
        assert!(edges.add_polygon(1, 0, &plane_rect(0.0, 0.0, 8.0, 2.0, [0.0, 0.0, 0.125])));
        assert!(edges.add_polygon(
            2,
            u32::MAX,
            &plane_rect(2.0, 0.0, 6.0, 2.0, [0.0, 0.0, 0.25])
        ));
        let (pixels, stats, _) = coverage(&mut edges, 8, 2);
        for y in 0..2 {
            for x in 0..8 {
                let near = policy == DepthPolicy::PlaneDepth && x >= 2 && x < 6;
                assert_eq!(pixels[y * 8 + x], Some(if near { 2 } else { 1 }));
            }
        }
        assert_eq!(stats.pixels, 16);
    }
}

#[test]
fn depth_plane_crossing_splits_inside_one_edge_interval_with_exact_ties() {
    for other_key in [5, 20] {
        let mut edges = Edges::load(8, 2, 8, 4, 16).unwrap();
        assert!(edges.begin_with_policy(view(8, 2), DepthPolicy::PlaneDepth));
        assert!(edges.add_polygon(
            1,
            10,
            &plane_rect(0.0, 0.0, 8.0, 2.0, [-0.0625, 0.0, 0.625])
        ));
        assert!(edges.add_polygon(
            2,
            other_key,
            &plane_rect(0.0, 0.0, 8.0, 2.0, [0.0625, 0.0, 0.125])
        ));
        let (pixels, stats, spans) = coverage(&mut edges, 8, 2);
        for y in 0..2 {
            for x in 0..8 {
                let first = x < 4 || (x == 4 && 10 < other_key);
                assert_eq!(pixels[y * 8 + x], Some(if first { 1 } else { 2 }));
            }
        }
        assert_eq!(stats.pixels, 16);
        assert_eq!(spans.len(), 4);
    }
}

#[test]
fn depth_envelope_handles_three_visible_planes_and_hidden_intersections() {
    let mut edges = Edges::load(12, 2, 12, 6, 16).unwrap();
    assert!(edges.begin_with_policy(view(12, 2), DepthPolicy::PlaneDepth));
    for (surface, key, plane) in [
        (1, 10, [-0.0625, 0.0, 0.875]),
        (2, 20, [0.0, 0.0, 0.625]),
        (3, 30, [0.0625, 0.0, 0.125]),
        (4, 0, [0.0, 0.0, 0.25]),
    ] {
        assert!(edges.add_polygon(surface, key, &plane_rect(0.0, 0.0, 12.0, 2.0, plane)));
    }
    let (pixels, stats, spans) = coverage(&mut edges, 12, 2);
    for y in 0..2 {
        for x in 0..12 {
            assert_eq!(
                pixels[y * 12 + x],
                Some(if x <= 4 {
                    1
                } else if x <= 8 {
                    2
                } else {
                    3
                })
            );
        }
    }
    assert_eq!(stats.pixels, 24);
    assert_eq!(spans.len(), 6);
}

#[test]
fn curved_patch_triangles_share_edge_machinery_and_actual_depth_occlusion() {
    let mut edges = Edges::load(6, 6, 12, 6, 32).unwrap();
    assert!(edges.begin_with_policy(view(6, 6), DepthPolicy::PlaneDepth));
    assert!(edges.add_polygon(1, 0, &plane_rect(0.0, 0.0, 6.0, 6.0, [0.0, 0.0, 0.125])));
    let mut first = polygon(&[[0.0, 0.0], [6.0, 0.0], [0.0, 6.0]]);
    for (vertex, depth) in first.iter_mut().zip([0.25, 0.5, 0.25]) {
        vertex.inverse_depth = depth;
    }
    let mut second = polygon(&[[6.0, 0.0], [6.0, 6.0], [0.0, 6.0]]);
    for (vertex, depth) in second.iter_mut().zip([0.5, 0.75, 0.25]) {
        vertex.inverse_depth = depth;
    }
    assert!(edges.add_polygon(10, 100, &first));
    assert!(edges.add_polygon(11, 101, &second));
    assert!(edges.add_polygon(
        20,
        u32::MAX,
        &plane_rect(3.0, 1.0, 5.0, 5.0, [0.0, 0.0, 0.4375])
    ));
    let (pixels, stats, _) = coverage(&mut edges, 6, 6);
    for y in 0..6 {
        for x in 0..6 {
            let (patch, depth) = if x < 6 - y {
                (10, 0.25 + x as f64 / 24.0)
            } else {
                (11, x as f64 / 12.0 + y as f64 / 24.0)
            };
            let overlay = x >= 3 && x < 5 && y >= 1 && y < 5 && 0.4375 > depth;
            assert_eq!(pixels[y * 6 + x], Some(if overlay { 20 } else { patch }));
        }
    }
    assert_eq!(stats.pixels, 36);
}

#[test]
fn equal_depth_planes_retain_native_initial_and_later_row_leader_ties() {
    let mut edges = Edges::load(8, 3, 8, 4, 16).unwrap();
    for start_y in [0.0, 1.0] {
        assert!(edges.begin_with_policy(view(8, 3), DepthPolicy::PlaneDepth));
        assert!(edges.add_polygon(10, 4, &plane_rect(1.0, 0.0, 7.0, 3.0, [0.0, 0.0, 0.25])));
        assert!(edges.add_polygon(20, 4, &plane_rect(1.0, start_y, 6.0, 3.0, [0.0, 0.0, 0.25])));
        let (pixels, _, _) = coverage(&mut edges, 8, 3);
        for y in 0..3 {
            for x in 0..8 {
                let expected = if x < 1 || x >= 7 {
                    None
                } else if y as f32 >= start_y && x < 6 {
                    Some(20)
                } else {
                    Some(10)
                };
                assert_eq!(pixels[y * 8 + x], expected);
            }
        }
    }
}

#[test]
fn overlapping_worlds_with_independent_keys_resolve_one_visible_depth_envelope() {
    let mut edges = Edges::load(8, 3, 10, 5, 1).unwrap();
    assert!(edges.begin_with_policy(view(8, 3), DepthPolicy::PlaneDepth));
    assert!(edges.add_polygon(10, 0, &plane_rect(0.0, 0.0, 8.0, 3.0, [0.0, 0.0, 0.25])));
    assert!(edges.add_polygon(20, 9000, &plane_rect(2.0, 0.0, 6.0, 3.0, [0.0, 0.0, 0.5])));
    assert!(edges.add_polygon(30, 0, &plane_rect(4.0, 1.0, 8.0, 3.0, [0.0, 0.0, 0.375])));
    let (pixels, stats, _) = coverage(&mut edges, 8, 3);
    for y in 0..3 {
        for x in 0..8 {
            let expected = if x >= 2 && x < 6 {
                20
            } else if y >= 1 && x >= 4 {
                30
            } else {
                10
            };
            assert_eq!(pixels[y * 8 + x], Some(expected));
        }
    }
    assert_eq!(stats.pixels, 24);
    assert_eq!(stats.rejected, 0);
    assert!(stats.flushes > 1);
}

#[test]
fn depth_plane_uses_a_noncollinear_triple_after_collinear_polygon_vertices() {
    let mut edges = Edges::load(4, 4, 8, 4, 8).unwrap();
    assert!(edges.begin_with_policy(view(4, 4), DepthPolicy::PlaneDepth));
    assert!(edges.add_polygon(1, 0, &plane_rect(0.0, 0.0, 4.0, 4.0, [0.0, 0.0, 0.125])));
    assert!(edges.add_polygon(
        2,
        100,
        &plane_polygon(
            &[[0.0, 0.0], [2.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0],],
            [0.03125, 0.0, 0.25]
        )
    ));
    let (pixels, stats, _) = coverage(&mut edges, 4, 4);
    assert!(pixels.iter().all(|&pixel| pixel == Some(2)));
    assert_eq!(stats.pixels, 16);
    assert_eq!(stats.rejected, 0);
}

#[test]
fn widest_depth_basis_preserves_slope_lost_in_rounded_nearly_collinear_vertices() {
    let mut edges = Edges::load(4, 4, 8, 4, 8).unwrap();
    assert!(edges.begin_with_policy(view(4, 4), DepthPolicy::PlaneDepth));
    let vertices = plane_polygon(
        &[
            [0.0, 0.0],
            [1.0, 0.0],
            [2.0, 2.0_f32.powi(-24)],
            [4.0, 4.0],
            [0.0, 4.0],
        ],
        [0.125, 0.25, 0.25],
    );
    // Rounding loses the first tiny triangle's y slope, while wider vertices
    // still exactly describe the actual .125*x + .25*y + .25 depth plane.
    assert_eq!(vertices[2].inverse_depth, 0.5);
    assert_eq!(vertices[3].inverse_depth, 1.75);
    assert_eq!(vertices[4].inverse_depth, 1.25);
    assert!(edges.add_polygon(1, 100, &vertices));
    assert!(edges.add_polygon(2, 0, &plane_rect(0.0, 0.0, 4.0, 4.0, [0.0, 0.0, 0.6])));
    let (pixels, stats, _) = coverage(&mut edges, 4, 4);
    assert_eq!(pixels[1 * 4 + 1], Some(1)); // Actual .625 is nearer than .6.
    assert_eq!(pixels[1 * 4], Some(2)); // Actual .5 is farther than .6.
    assert_eq!(pixels[2 * 4 + 2], Some(1));
    assert_eq!(stats.pixels, 16);
    assert_eq!(stats.rejected, 0);
}
