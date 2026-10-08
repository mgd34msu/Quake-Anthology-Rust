use qa_render::edges::{Edges, ProjectedVertex, Span, Stats};
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
    // R_EmitEdge inserts the later equal-U leader first in the new-edge bucket.
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
