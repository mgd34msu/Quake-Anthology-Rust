use super::tree::BrushScratch;
use super::tree::Topology;
pub use super::tree::{BrushTree, CollisionLeaf, ModelRoot};
use super::{
    AllSolid, BoundsOrigin, Contents, EntityTraceRules, FractionClamp, OutsideBrush,
    PositionEndpoint, PositionRules, Trace, TraceQuery,
};
use qa_core::primitives::{Bounds, Plane, SurfaceFlags, Vec3};

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
    TreePlane,
    TreeRange,
    TreeCycle,
    Capacity,
}

pub(super) struct BrushMap {
    pub(super) planes: Box<[Plane]>,
    pub(super) brushes: Box<[Brush]>,
    pub(super) surfaces: Box<[SurfaceFlags]>,
    pub(super) topology: Topology,
    pub(super) axial_bounds: Box<[Option<Bounds>]>,
}

impl BrushMap {
    /// Side geometry and BSP/model membership retain their original numeric IDs.
    pub(crate) fn load_tree(
        planes: Vec<Plane>,
        brushes: Vec<Brush>,
        surfaces: Vec<SurfaceFlags>,
        tree: BrushTree,
    ) -> Result<(Self, Vec<ModelRoot>), GeometryError> {
        if surfaces.len() != planes.len() || planes.iter().any(|plane| !valid_plane(*plane)) {
            return Err(GeometryError::Plane);
        }
        if brushes.iter().any(|brush| {
            (brush.first_plane as usize)
                .checked_add(brush.plane_count as usize)
                .is_none_or(|end| end > planes.len())
        }) {
            return Err(GeometryError::BrushRange);
        }
        let (topology, models) = Topology::load(tree, &brushes)?;
        let axial_bounds = brushes
            .iter()
            .map(|brush| {
                axial_prefix(
                    &planes[brush.first_plane as usize
                        ..brush.first_plane as usize + brush.plane_count as usize],
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok((
            Self {
                planes: planes.into_boxed_slice(),
                brushes: brushes.into_boxed_slice(),
                surfaces: surfaces.into_boxed_slice(),
                topology,
                axial_bounds,
            },
            models,
        ))
    }

    pub(crate) fn scratch_capacity(&self) -> (usize, usize) {
        (self.topology.scratch_capacity(), self.brushes.len())
    }

    pub(crate) fn trace_root(
        &self,
        root: ModelRoot,
        query: TraceQuery,
        scratch: &mut BrushScratch,
    ) -> Trace {
        self.topology.trace(self, root, query, scratch)
    }

    pub(crate) fn point_contents_root(
        &self,
        root: ModelRoot,
        point: Vec3,
        rules: EntityTraceRules,
    ) -> Contents {
        let leaf = self.topology.point_leaf(root, point);
        if !matches!(rules, EntityTraceRules::Quake3)
            && let Some(contents) = leaf.stored_contents
        {
            return contents;
        }
        let mut contents = Contents::EMPTY;
        for &id in self.topology.members(leaf) {
            let brush = self.brushes[id as usize];
            let planes = &self.planes[brush.first_plane as usize
                ..brush.first_plane as usize + brush.plane_count as usize];
            if !planes
                .iter()
                .any(|plane| point.dot(plane.normal) > plane.distance)
            {
                contents |= brush.contents;
            }
        }
        contents
    }
}

pub(super) fn valid_plane(plane: Plane) -> bool {
    plane.distance.is_finite()
        && plane.normal.0.iter().all(|value| value.is_finite())
        && plane.normal != Vec3::default()
}

fn axial_prefix(planes: &[Plane]) -> Option<Bounds> {
    if planes.len() < 6 {
        return None;
    }
    let mut bounds = Bounds::default();
    for axis in 0..3 {
        let mut negative = [0.0; 3];
        negative[axis] = -1.0;
        let mut positive = [0.0; 3];
        positive[axis] = 1.0;
        let pair = &planes[axis * 2..axis * 2 + 2];
        let (min, max) = if pair[0].normal == Vec3(negative) && pair[1].normal == Vec3(positive) {
            (-pair[0].distance, pair[1].distance)
        } else if pair[0].normal == Vec3(positive) && pair[1].normal == Vec3(negative) {
            (-pair[1].distance, pair[0].distance)
        } else {
            return None;
        };
        bounds.mins.0[axis] = min;
        bounds.maxs.0[axis] = max;
    }
    bounds
        .mins
        .0
        .iter()
        .zip(bounds.maxs.0)
        .all(|(min, max)| *min <= max)
        .then_some(bounds)
}

pub(super) struct BrushWork<'a> {
    pub query: TraceQuery<'a>,
    pub start: Vec3,
    pub end: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub extents: Vec3,
    pub tree_point: bool,
    pub bounds: Bounds,
    support_point: bool,
}

impl<'a> BrushWork<'a> {
    pub fn new(query: TraceQuery<'a>) -> Self {
        let mut start = query.start;
        let mut end = query.end;
        let mut mins = query.mins;
        let mut maxs = query.maxs;
        if query.rules.bounds_origin == BoundsOrigin::Centered {
            for axis in 0..3 {
                let center = (mins.0[axis] + maxs.0[axis]) * 0.5;
                mins.0[axis] -= center;
                maxs.0[axis] -= center;
                start.0[axis] += center;
                end.0[axis] += center;
            }
        }
        let centered = query.rules.bounds_origin == BoundsOrigin::Centered;
        // Q3's tree classification tests size[0] alone. Support planes still
        // use both actual centered bounds, including asymmetric f32 rounding.
        let tree_point = mins == Vec3::default() && (centered || maxs == Vec3::default());
        let extents = if tree_point {
            Vec3::default()
        } else if centered {
            maxs
        } else {
            Vec3(std::array::from_fn(|axis| {
                (-mins.0[axis]).max(maxs.0[axis])
            }))
        };
        let bounds = Bounds {
            mins: Vec3(std::array::from_fn(|axis| {
                (if start.0[axis] < end.0[axis] {
                    start.0[axis]
                } else {
                    end.0[axis]
                }) + mins.0[axis]
            })),
            maxs: Vec3(std::array::from_fn(|axis| {
                (if start.0[axis] < end.0[axis] {
                    end.0[axis]
                } else {
                    start.0[axis]
                }) + maxs.0[axis]
            })),
        };
        Self {
            query,
            start,
            end,
            mins,
            maxs,
            extents,
            tree_point,
            bounds,
            support_point: !centered && tree_point,
        }
    }

    fn distance(&self, plane: Plane, point_shortcut: bool) -> f32 {
        if point_shortcut && self.support_point {
            return plane.distance;
        }
        let offset = Vec3(std::array::from_fn(|axis| {
            if plane.normal.0[axis] < 0.0 {
                self.maxs.0[axis]
            } else {
                self.mins.0[axis]
            }
        }));
        plane.distance - offset.dot(plane.normal)
    }

    pub fn end_position(&self, fraction: f32) -> Vec3 {
        if self.query.start == self.query.end
            && self.query.rules.position_endpoint == PositionEndpoint::Start
        {
            self.query.start
        } else if fraction == 1.0 {
            self.query.end
        } else {
            self.query.start.lerp(self.query.end, fraction)
        }
    }
}

pub(super) fn position_brush(
    work: &BrushWork,
    planes: &[Plane],
    brush: Brush,
    bounds: Option<Bounds>,
    trace: &mut Trace,
) {
    if brush.plane_count == 0 {
        return;
    }
    let mut first = 0;
    if work.query.rules.position == PositionRules::AxialPrefix
        && let Some(bounds) = bounds
    {
        if (0..3).any(|axis| {
            work.bounds.mins.0[axis] > bounds.maxs.0[axis]
                || work.bounds.maxs.0[axis] < bounds.mins.0[axis]
        }) {
            return;
        }
        first = 6;
    }
    for &plane in &planes[brush.first_plane as usize + first
        ..brush.first_plane as usize + brush.plane_count as usize]
    {
        if work.start.dot(plane.normal) - work.distance(plane, false) > 0.0 {
            return;
        }
    }
    trace.start_solid = true;
    trace.all_solid = true;
    trace.fraction = 0.0;
    trace.contents = brush.contents;
}

pub(super) fn clip_brush(
    work: &BrushWork,
    planes: &[Plane],
    brush: Brush,
    surfaces: &[SurfaceFlags],
    trace: &mut Trace,
) {
    if brush.plane_count == 0 {
        return;
    }
    let sides = &planes
        [brush.first_plane as usize..brush.first_plane as usize + brush.plane_count as usize];
    let rules = work.query.rules;
    let mut enter = -1.0f32;
    let mut leave = 1.0f32;
    let mut contact = Plane::default();
    let mut surface = SurfaceFlags::default();
    let mut start_out = false;
    let mut end_out = false;
    for (index, &plane) in sides.iter().enumerate() {
        let distance = work.distance(plane, true);
        let d1 = work.start.dot(plane.normal) - distance;
        let d2 = work.end.dot(plane.normal) - distance;
        start_out |= d1 > 0.0;
        end_out |= d2 > 0.0;
        if d1 > 0.0
            && (d2 >= d1
                || rules.outside_brush == OutsideBrush::EndBeyondEpsilon
                    && f64::from(d2) >= rules.contact_epsilon)
        {
            return;
        }
        if d1 <= 0.0 && d2 <= 0.0 {
            continue;
        }
        if d1 > d2 {
            let mut fraction =
                ((f64::from(d1) - rules.contact_epsilon) / f64::from(d1 - d2)) as f32;
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
    if !start_out {
        trace.start_solid = true;
        if !end_out {
            trace.all_solid = true;
            if rules.all_solid == AllSolid::BlockAtStart {
                trace.fraction = 0.0;
                trace.contents = brush.contents;
            }
        }
    } else if enter < leave && enter > -1.0 && enter < trace.fraction {
        trace.fraction = enter.max(0.0);
        trace.plane = contact;
        trace.surface = surface;
        trace.contents = brush.contents;
    }
}

/// Loaded geometry and stack-built temporary bodies share these kernels.
pub(crate) fn trace_brushes(
    query: TraceQuery,
    planes: &[Plane],
    brushes: &[Brush],
    surfaces: &[SurfaceFlags],
) -> Trace {
    let work = BrushWork::new(query);
    let mut trace = Trace::clear(query.end);
    for &brush in brushes {
        if !brush.contents.intersects(query.mask) {
            continue;
        }
        if query.start == query.end {
            let sides = &planes[brush.first_plane as usize
                ..brush.first_plane as usize + brush.plane_count as usize];
            position_brush(&work, planes, brush, axial_prefix(sides), &mut trace);
        } else {
            clip_brush(&work, planes, brush, surfaces, &mut trace);
        }
        if trace.fraction == 0.0 {
            break;
        }
    }
    trace.end = work.end_position(trace.fraction);
    trace
}
