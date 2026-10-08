use super::{AllSolid, BoundsOrigin, Contents, FractionClamp, OutsideBrush, Trace, TraceQuery};
use qa_core::primitives::{Plane, SurfaceFlags, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub first_plane: u32,
    pub plane_count: u32,
    pub contents: Contents,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GeometryError {
    Plane,
    BrushRange,
}

pub struct BrushMap {
    planes: Box<[Plane]>,
    brushes: Box<[Brush]>,
    surfaces: Box<[SurfaceFlags]>,
}

impl BrushMap {
    pub fn load(planes: Vec<Plane>, brushes: Vec<Brush>) -> Result<Self, GeometryError> {
        let surfaces = vec![SurfaceFlags::default(); planes.len()];
        Self::load_surfaces(planes, brushes, surfaces)
    }
    /// BSP side metadata converts once at load, independent of movement rules.
    pub fn load_surfaces(
        planes: Vec<Plane>,
        brushes: Vec<Brush>,
        surfaces: Vec<SurfaceFlags>,
    ) -> Result<Self, GeometryError> {
        if surfaces.len() != planes.len() {
            return Err(GeometryError::Plane);
        }
        if planes.iter().any(|plane| {
            !plane.distance.is_finite()
                || plane.normal.0.iter().any(|value| !value.is_finite())
                || plane.normal == Vec3::default()
        }) {
            return Err(GeometryError::Plane);
        }
        if brushes.iter().any(|brush| {
            (brush.first_plane as usize)
                .checked_add(brush.plane_count as usize)
                .is_none_or(|end| end > planes.len())
        }) {
            return Err(GeometryError::BrushRange);
        }
        Ok(Self {
            planes: planes.into_boxed_slice(),
            brushes: brushes.into_boxed_slice(),
            surfaces: surfaces.into_boxed_slice(),
        })
    }

    pub fn point_contents(&self, point: Vec3) -> Contents {
        let mut contents = Contents::EMPTY;
        for brush in &self.brushes {
            let planes = &self.planes[brush.first_plane as usize
                ..brush.first_plane as usize + brush.plane_count as usize];
            if !planes.is_empty()
                && planes
                    .iter()
                    .all(|plane| plane.normal.dot(point) <= plane.distance)
            {
                contents |= brush.contents;
            }
        }
        contents
    }

    pub(crate) fn trace(&self, query: TraceQuery) -> Trace {
        trace_brushes(query, &self.planes, &self.brushes, &self.surfaces)
    }
}

/// Loaded world brushes and stack-built temporary bodies share this kernel.
pub(crate) fn trace_brushes(
    query: TraceQuery,
    planes: &[Plane],
    brushes: &[Brush],
    surfaces: &[SurfaceFlags],
) -> Trace {
    let TraceQuery {
        mut start,
        mut end,
        mut mins,
        mut maxs,
        mask,
        rules,
        ..
    } = query;
    if rules.bounds_origin == BoundsOrigin::Centered {
        // CM_Trace centers the box before its brush support-point math.
        // Keep its f32 addition/subtraction order for asymmetric bounds.
        for axis in 0..3 {
            let center = (mins.0[axis] + maxs.0[axis]) * 0.5;
            mins.0[axis] -= center;
            maxs.0[axis] -= center;
            start.0[axis] += center;
            end.0[axis] += center;
        }
    }
    let point = mins == Vec3::default() && maxs == Vec3::default();
    let mut trace = Trace::clear(query.end);
    for brush in brushes {
        if !brush.contents.intersects(mask) || brush.plane_count == 0 {
            continue;
        }
        let planes = &planes
            [brush.first_plane as usize..brush.first_plane as usize + brush.plane_count as usize];
        let mut enter = -1.0f32;
        let mut leave = 1.0f32;
        let mut contact = Plane::default();
        let mut surface = SurfaceFlags::default();
        let mut start_out = false;
        let mut end_out = false;
        let mut missed = false;
        for (index, &plane) in planes.iter().enumerate() {
            let offset = Vec3(std::array::from_fn(|axis| {
                if plane.normal.0[axis] < 0.0 {
                    maxs.0[axis]
                } else {
                    mins.0[axis]
                }
            }));
            let distance = if point {
                plane.distance
            } else {
                plane.distance - offset.dot(plane.normal)
            };
            let d1 = start.dot(plane.normal) - distance;
            let d2 = end.dot(plane.normal) - distance;
            start_out |= d1 > 0.0;
            end_out |= d2 > 0.0;
            if d1 > 0.0
                && (d2 >= d1
                    || rules.outside_brush == OutsideBrush::EndBeyondEpsilon
                        && f64::from(d2) >= rules.contact_epsilon)
            {
                missed = true;
                break;
            }
            if d1 <= 0.0 && d2 <= 0.0 {
                continue;
            }
            if d1 > d2 {
                let mut fraction =
                    ((f64::from(d1) - rules.contact_epsilon) / f64::from(d1 - d2)) as f32;
                // Q3 clamps before comparing, including equal near-contact
                // planes. Q2 clamps only the selected brush intersection.
                if rules.fraction_clamp == FractionClamp::PerPlane && fraction < 0.0 {
                    fraction = 0.0;
                }
                if fraction > enter {
                    enter = fraction;
                    contact = plane;
                    surface = surfaces[brush.first_plane as usize + index];
                }
            } else {
                let mut fraction =
                    ((f64::from(d1) + rules.contact_epsilon) / f64::from(d1 - d2)) as f32;
                if rules.fraction_clamp == FractionClamp::PerPlane && fraction > 1.0 {
                    fraction = 1.0;
                }
                if fraction < leave {
                    leave = fraction;
                }
            }
        }
        if missed {
            continue;
        }
        if !start_out {
            trace.start_solid = true;
            if !end_out {
                trace.all_solid = true;
                // Stationary Q2/Q3 position tests block immediately. A
                // moving Q2 embedded trace preserves fraction and contents.
                if rules.all_solid == AllSolid::BlockAtStart || query.start == query.end {
                    trace.fraction = 0.0;
                    trace.contents = brush.contents;
                }
            }
            if trace.fraction == 0.0 {
                break;
            }
            continue;
        }
        if enter < leave && enter > -1.0 && enter < trace.fraction {
            trace.fraction = enter.max(0.0);
            trace.plane = contact;
            trace.surface = surface;
            trace.contents = brush.contents;
        }
        // Native leaf traversal stops at the first zero-fraction brush;
        // later overlapping brushes must not replace that contact.
        if trace.fraction == 0.0 {
            break;
        }
    }
    trace.end = if trace.fraction == 1.0 {
        query.end
    } else {
        query.start.lerp(query.end, trace.fraction)
    };
    trace
}
