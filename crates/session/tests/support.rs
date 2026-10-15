use qa_core::primitives::{Bounds, Vec3};
use qa_session::physics::{SupportQueries, check_bottom};
use qa_world::collision::Trace;

struct Queries {
    contents: Vec<Vec3>,
    traces: Vec<(Vec3, Vec3)>,
    solid: bool,
    heights: [Option<f32>; 5],
}
impl SupportQueries for Queries {
    type Error = ();
    fn solid(&mut self, point: Vec3) -> Result<bool, ()> {
        self.contents.push(point);
        Ok(self.solid)
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<Trace, ()> {
        let index = self.traces.len();
        self.traces.push((start, end));
        let mut trace = Trace::clear(end);
        if let Some(height) = self.heights[index] {
            trace.fraction = 0.5;
            trace.end.0[2] = height;
        }
        Ok(trace)
    }
}
fn queries(solid: bool, heights: [Option<f32>; 5]) -> Queries {
    Queries {
        contents: Vec::new(),
        traces: Vec::new(),
        solid,
        heights,
    }
}
const ORIGIN: Vec3 = Vec3([10.0, 20.0, 30.0]);
const BOUNDS: Bounds = Bounds {
    mins: Vec3([-2.0, -3.0, -4.0]),
    maxs: Vec3([5.0, 6.0, 7.0]),
};

#[test]
fn solid_corners_end_before_any_trace_and_keep_native_order() {
    let mut query = queries(true, [None; 5]);
    assert_eq!(check_bottom(&mut query, ORIGIN, BOUNDS, 18.0), Ok(true));
    assert_eq!(
        query.contents,
        [
            Vec3([8.0, 17.0, 25.0]),
            Vec3([8.0, 26.0, 25.0]),
            Vec3([15.0, 17.0, 25.0]),
            Vec3([15.0, 26.0, 25.0]),
        ]
    );
    assert!(query.traces.is_empty());
}

#[test]
fn support_traces_use_midpoint_then_corners_and_allow_exactly_one_step() {
    let mut query = queries(
        false,
        [Some(20.0), Some(2.0), Some(22.0), Some(20.0), Some(20.0)],
    );
    assert_eq!(check_bottom(&mut query, ORIGIN, BOUNDS, 18.0), Ok(true));
    assert_eq!(query.contents, [Vec3([8.0, 17.0, 25.0])]);
    assert_eq!(
        query.traces,
        [
            (Vec3([11.5, 21.5, 26.0]), Vec3([11.5, 21.5, -10.0])),
            (Vec3([8.0, 17.0, 26.0]), Vec3([8.0, 17.0, -10.0])),
            (Vec3([8.0, 26.0, 26.0]), Vec3([8.0, 26.0, -10.0])),
            (Vec3([15.0, 17.0, 26.0]), Vec3([15.0, 17.0, -10.0])),
            (Vec3([15.0, 26.0, 26.0]), Vec3([15.0, 26.0, -10.0])),
        ]
    );
}

#[test]
fn a_midpoint_miss_or_first_unsupported_corner_stops_the_remaining_queries() {
    let mut query = queries(false, [None; 5]);
    assert_eq!(check_bottom(&mut query, ORIGIN, BOUNDS, 18.0), Ok(false));
    assert_eq!(query.traces.len(), 1);
    for corner in [None, Some(1.999)] {
        let mut query = queries(false, [Some(20.0), corner, None, None, None]);
        assert_eq!(check_bottom(&mut query, ORIGIN, BOUNDS, 18.0), Ok(false));
        assert_eq!(query.traces.len(), 2);
    }
}
