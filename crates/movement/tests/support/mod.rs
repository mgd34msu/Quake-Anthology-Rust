use qa_core::primitives::{Plane, Vec3};
use qa_movement::TraceServices;
use qa_world::collision::{Contents, Trace, TraceQuery};

/// Analytic fixture shared by function checks and timing; no retail-map claim.
#[derive(Default)]
pub struct FixtureWorld {
    pub step: bool,
    pub water: bool,
}
impl TraceServices for FixtureWorld {
    fn trace(&mut self, query: TraceQuery) -> Trace {
        // The extracted original movement fixture and this analytic oracle
        // both have zero contact bias. Production hull/brush services apply
        // the explicit caller policy received through this same entry.
        let TraceQuery {
            start,
            end,
            mins,
            maxs,
            ..
        } = query;
        let mut result = Trace::clear(end);
        for (normal, distance) in [
            (Vec3([0.0, 0.0, 1.0]), 0.0),
            (Vec3([-1.0, 0.0, 0.0]), -240.0),
            (Vec3([0.0, 1.0, 0.0]), -120.0),
            (Vec3([0.0, -1.0, 0.0]), -120.0),
        ] {
            let offset: f32 = (0..3)
                .map(|i| {
                    normal.0[i]
                        * if normal.0[i] < 0.0 {
                            maxs.0[i]
                        } else {
                            mins.0[i]
                        }
                })
                .sum();
            let a = start.dot(normal) + offset - distance;
            let b = end.dot(normal) + offset - distance;
            if a < 0.0 {
                result.start_solid = true;
                result.all_solid = b < 0.0;
            }
            if a >= 0.0 && b < 0.0 {
                let fraction = a / (a - b);
                if fraction < result.fraction {
                    result.fraction = fraction;
                    result.plane = Plane {
                        normal,
                        distance,
                        axis: None,
                    };
                }
            }
        }
        if self.step {
            let low = Vec3([
                96.0 - maxs.0[0],
                -1000000.0 - maxs.0[1],
                -1000000.0 - maxs.0[2],
            ]);
            let high = Vec3([160.0 - mins.0[0], 1000000.0 - mins.0[1], 16.0 - mins.0[2]]);
            let inside = |p: Vec3| (0..3).all(|i| p.0[i] > low.0[i] && p.0[i] < high.0[i]);
            if inside(start) {
                result.start_solid = true;
                result.all_solid = inside(end);
            } else {
                let delta = end - start;
                let mut enter = 0.0f32;
                let mut exit = 1.0f32;
                let mut normal = Vec3::default();
                let mut miss = false;
                for i in 0..3 {
                    if delta.0[i] == 0.0 {
                        if start.0[i] <= low.0[i] || start.0[i] >= high.0[i] {
                            miss = true;
                            break;
                        }
                        continue;
                    }
                    let (near, far, sign) = if delta.0[i] > 0.0 {
                        (
                            (low.0[i] - start.0[i]) / delta.0[i],
                            (high.0[i] - start.0[i]) / delta.0[i],
                            -1.0,
                        )
                    } else {
                        (
                            (high.0[i] - start.0[i]) / delta.0[i],
                            (low.0[i] - start.0[i]) / delta.0[i],
                            1.0,
                        )
                    };
                    if near >= enter {
                        enter = near;
                        normal = Vec3::default();
                        normal.0[i] = sign;
                    }
                    exit = exit.min(far);
                    if enter > exit {
                        miss = true;
                        break;
                    }
                }
                if !miss && normal != Vec3::default() && enter < result.fraction && exit > 0.0 {
                    result.fraction = enter;
                    result.plane.normal = normal;
                }
            }
        }
        result.end = start + (end - start) * result.fraction;
        if result.fraction < 1.0 || result.start_solid {
            result.contents = Contents::SOLID;
        }
        result
    }
    fn point_contents(&self, point: Vec3) -> Contents {
        if self.water && point.0[2] < 64.0 {
            Contents::WATER
        } else {
            Contents::EMPTY
        }
    }
}
