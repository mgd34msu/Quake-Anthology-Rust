use super::{Contents, Trace};
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

pub(crate) enum BrushRules {
    Classic,
    Arena,
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

    pub(crate) fn trace(
        &self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
        mask: Contents,
        rules: BrushRules,
    ) -> Trace {
        let (epsilon, arena_rules) = match rules {
            BrushRules::Classic => (0.03125, false),
            BrushRules::Arena => (0.125, true),
        };
        let mut trace = Trace::clear(end);
        for brush in &self.brushes {
            if !brush.contents.intersects(mask) || brush.plane_count == 0 {
                continue;
            }
            let planes = &self.planes[brush.first_plane as usize
                ..brush.first_plane as usize + brush.plane_count as usize];
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
                let distance = plane.distance - offset.dot(plane.normal);
                let d1 = start.dot(plane.normal) - distance;
                let d2 = end.dot(plane.normal) - distance;
                start_out |= d1 > 0.0;
                end_out |= d2 > 0.0;
                if d1 > 0.0 && (d2 >= d1 || d2 > 0.0 && (!arena_rules || d2 >= epsilon)) {
                    missed = true;
                    break;
                }
                if d1 <= 0.0 && d2 <= 0.0 {
                    continue;
                }
                if d1 > d2 {
                    let fraction = (d1 - epsilon) / (d1 - d2);
                    if fraction > enter {
                        enter = fraction;
                        contact = plane;
                        surface = self.surfaces[brush.first_plane as usize + index];
                    }
                } else {
                    leave = leave.min((d1 + epsilon) / (d1 - d2));
                }
            }
            if missed {
                continue;
            }
            if !start_out {
                trace.start_solid = true;
                if !end_out {
                    trace.all_solid = true;
                    trace.contents = brush.contents;
                    if arena_rules || start == end {
                        trace.fraction = 0.0;
                    }
                }
                continue;
            }
            if enter < leave && enter > -1.0 && enter < trace.fraction {
                trace.fraction = enter.max(0.0);
                trace.plane = contact;
                trace.surface = surface;
                trace.contents = brush.contents;
            }
        }
        trace.end = start.lerp(end, trace.fraction);
        trace
    }
}
