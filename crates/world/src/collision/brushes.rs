use super::patches::{Patch, PatchInput};
use super::surfaces::{SurfaceRows, SurfaceStorage, SurfaceTable};
use super::tree::BrushScratch;
use super::tree::Topology;
pub use super::tree::{BrushTree, CollisionLeaf, ModelRoot};
use super::{
    AllSolid, BoundsOrigin, Contents, EntityTracePolicy, FractionClamp, OutsideBrush,
    PositionEndpoint, PositionRules, Trace, TraceError, TraceQuery, TraceRules,
};
use qa_core::primitives::{Bounds, Plane, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub sides: qa_core::primitives::IndexSpan,
    pub contents: Contents,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GeometryError {
    Plane,
    Surface,
    BrushRange,
    TreePlane,
    TreeRange,
    TreeCycle,
    Capacity,
    PatchGrid,
}

pub(super) struct BrushMap {
    pub(super) planes: Box<[Plane]>,
    pub(super) brushes: Box<[Brush]>,
    pub(super) patches: Box<[Patch]>,
    pub(super) patch_plane_count: usize,
    pub(super) surfaces: SurfaceStorage,
    pub(super) topology: Topology,
    pub(super) axial_bounds: Box<[Option<Bounds>]>,
}

impl BrushMap {
    /// Side geometry and BSP/model membership retain their original numeric IDs.
    pub(crate) fn load_tree(
        mut planes: Vec<Plane>,
        brushes: Vec<Brush>,
        patches: Vec<PatchInput>,
        mut surfaces: SurfaceTable<'_>,
        tree: BrushTree,
    ) -> Result<(Self, Vec<ModelRoot>), GeometryError> {
        if planes.iter().any(|plane| !plane.is_valid()) {
            return Err(GeometryError::Plane);
        }
        // BSP node types keep their native axial shortcut in Topology. Brush
        // membership instead tests the authored normal, including negative
        // axial sides (Q3 cm_test.c CM_PointContents). Preserve the native
        // contact bytes independently of the geometric orientation.
        for plane in &mut planes {
            if plane.axis.is_some()
                && plane.axis != Plane::oriented(plane.normal, plane.distance).axis
            {
                plane.encoding = Some(plane.type_sign());
                plane.axis = None;
            }
        }
        if brushes.iter().any(|brush| !brush.sides.fits(planes.len())) {
            return Err(GeometryError::BrushRange);
        }
        if surfaces.sides.len() != planes.len()
            || patches
                .iter()
                .any(|patch| patch.surface as usize >= surfaces.records.len())
        {
            return Err(GeometryError::Surface);
        }
        let mut generated = Vec::new();
        generated
            .try_reserve(patches.len())
            .map_err(|_| GeometryError::Capacity)?;
        for patch in patches {
            generated.push(super::patches::generate(
                patch,
                &mut planes,
                &mut surfaces.sides,
            )?);
        }
        if generated.iter().any(|patch| !patch.bounds.is_valid()) {
            return Err(GeometryError::PatchGrid);
        }
        let patch_plane_count = generated.iter().map(Patch::plane_count).max().unwrap_or(0);
        let surfaces = SurfaceStorage::load(surfaces, planes.len())?;
        let (topology, models) = Topology::load(tree, &brushes, &generated)?;
        let axial_bounds = brushes
            .iter()
            .map(|brush| axial_prefix(&planes[brush.sides.range()]))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok((
            Self {
                planes: planes.into_boxed_slice(),
                brushes: brushes.into_boxed_slice(),
                patches: generated.into_boxed_slice(),
                patch_plane_count,
                surfaces,
                topology,
                axial_bounds,
            },
            models,
        ))
    }

    pub(crate) fn scratch_capacity(&self) -> (usize, usize, usize) {
        (
            self.topology.scratch_capacity(),
            self.brushes.len() + self.patches.len(),
            self.patch_plane_count,
        )
    }

    pub(crate) fn trace_root(
        &self,
        root: ModelRoot,
        query: TraceQuery,
        scratch: &mut BrushScratch,
    ) -> Result<Trace, TraceError> {
        self.topology.trace(self, root, query, scratch)
    }

    pub(crate) fn point_contents_root(
        &self,
        root: ModelRoot,
        point: Vec3,
        rules: EntityTracePolicy,
    ) -> Contents {
        let leaf = self.topology.point_leaf(root, point);
        if rules.contents_leaf_gate() == super::LeafGate::StoredContents
            && let Some(contents) = leaf.stored_contents
        {
            return contents;
        }
        let mut contents = Contents::EMPTY;
        for &id in self.topology.members(leaf) {
            let brush = self.brushes[id as usize];
            let planes = &self.planes[brush.sides.range()];
            if !planes
                .iter()
                .any(|plane| plane.signed_distance(point) > 0.0)
            {
                contents |= brush.contents;
            }
        }
        contents
    }
}

fn axial_prefix(planes: &[Plane]) -> Option<Bounds> {
    if planes.len() < 6 {
        return None;
    }
    let mut bounds = Bounds::default();
    for axis in 0..3 {
        let negative = Vec3::axis(axis, -1.0);
        let positive = Vec3::axis(axis, 1.0);
        let pair = &planes[axis * 2..axis * 2 + 2];
        let (min, max) = if pair[0].normal == negative && pair[1].normal == positive {
            (-pair[0].distance, pair[1].distance)
        } else if pair[0].normal == positive && pair[1].normal == negative {
            (-pair[1].distance, pair[0].distance)
        } else {
            return None;
        };
        bounds.mins.0[axis] = min;
        bounds.maxs.0[axis] = max;
    }
    bounds.is_valid().then_some(bounds)
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
        Self::build(query, true)
    }

    pub(super) fn centered(query: TraceQuery<'a>) -> Self {
        Self::build(query, false)
    }

    fn build(query: TraceQuery<'a>, recenter: bool) -> Self {
        let centered =
            query.rules.bounds_origin == BoundsOrigin::Centered || query.capsule.is_some();
        let mut centered_query = query;
        if centered && recenter {
            centered_query.center_box();
        }
        let TraceQuery {
            start,
            end,
            mins,
            maxs,
            ..
        } = centered_query;
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
        let bounds = if query.capsule.is_some() {
            query.swept_bounds(0.0)
        } else {
            Bounds::swept(start, end, mins, maxs, 0.0)
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

    pub(super) fn distance(&self, plane: Plane, point_shortcut: bool) -> f32 {
        if point_shortcut && self.support_point {
            return plane.distance;
        }
        let offset = Bounds {
            mins: self.mins,
            maxs: self.maxs,
        }
        .corner(plane.normal, false);
        plane.distance - offset.dot(plane.normal)
    }

    fn distances(&self, plane: Plane, point_shortcut: bool) -> (f32, f32) {
        let (start, end, distance) = if let Some(capsule) = self.query.capsule {
            let (start, end) = capsule.closest_segment(self.start, self.end, plane.normal);
            (start, end, plane.distance + capsule.radius)
        } else {
            (self.start, self.end, self.distance(plane, point_shortcut))
        };
        (
            start.dot(plane.normal) - distance,
            end.dot(plane.normal) - distance,
        )
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
    if brush.sides.count == 0 {
        return;
    }
    let mut first = 0;
    if work.query.rules.position == PositionRules::AxialPrefix
        && let Some(bounds) = bounds
    {
        if !work.bounds.overlaps(bounds) {
            return;
        }
        first = 6;
    }
    for &plane in &planes[brush.sides.range()][first..] {
        if work.distances(plane, false).0 > 0.0 {
            return;
        }
    }
    trace.start_solid = true;
    trace.all_solid = true;
    trace.fraction = 0.0;
    trace.contents = brush.contents;
}

pub(super) struct ConvexInterval {
    pub enter: f32,
    pub leave: f32,
    pub contact: usize,
    secondary_contact: Option<usize>,
    start_out: bool,
    end_out: bool,
}

/// The side reducer owns selection and ordering; the caller supplies distances
/// in its native arithmetic order and retains the side's geometry and surface.
pub(super) fn convex_interval(
    rules: TraceRules,
    side_count: usize,
    mut distances: impl FnMut(usize) -> (f32, f32),
) -> Option<ConvexInterval> {
    let mut interval = ConvexInterval {
        enter: -1.0,
        leave: 1.0,
        contact: 0,
        secondary_contact: None,
        start_out: false,
        end_out: false,
    };
    let mut second_enter = -1.0f32;
    for index in 0..side_count {
        let (d1, d2) = distances(index);
        interval.start_out |= d1 > 0.0;
        interval.end_out |= d2 > 0.0;
        if d1 > 0.0
            && (d2 >= d1
                || rules.outside_brush == OutsideBrush::EndBeyondEpsilon
                    && f64::from(d2) >= rules.contact_epsilon)
        {
            return None;
        }
        if d1 <= 0.0 && d2 <= 0.0 {
            continue;
        }
        if d1 > d2 {
            let mut fraction = rules.contact_fraction(d1, d2, true);
            if rules.fraction_clamp == FractionClamp::PerPlane && fraction < 0.0 {
                fraction = 0.0;
            }
            if fraction > interval.enter {
                interval.enter = fraction;
                interval.contact = index;
            } else if rules.secondary_contact && fraction > second_enter {
                // Native KEX only records this else branch; it does not move
                // the previous primary plane into the secondary slot.
                second_enter = fraction;
                interval.secondary_contact = Some(index);
            }
        } else {
            let mut fraction = rules.contact_fraction(d1, d2, false);
            if rules.fraction_clamp == FractionClamp::PerPlane && fraction > 1.0 {
                fraction = 1.0;
            }
            if fraction < interval.leave {
                interval.leave = fraction;
            }
        }
    }
    Some(interval)
}

pub(super) fn clip_brush(
    work: &BrushWork,
    planes: &[Plane],
    brush: Brush,
    surfaces: SurfaceRows<'_>,
    trace: &mut Trace,
) {
    if brush.sides.count == 0 {
        return;
    }
    let sides = &planes[brush.sides.range()];
    let rules = work.query.rules;
    let Some(interval) = convex_interval(rules, sides.len(), |index| {
        let plane = sides[index];
        work.distances(plane, true)
    }) else {
        return;
    };
    if !interval.start_out {
        trace.start_solid = true;
        if !interval.end_out {
            trace.all_solid = true;
            if rules.all_solid == AllSolid::BlockAtStart {
                trace.fraction = 0.0;
                trace.contents = brush.contents;
            }
        }
    } else if interval.enter < interval.leave
        && interval.enter > -1.0
        && interval.enter < trace.fraction
    {
        let (surface, surface_id) = surfaces.contact(brush.sides.first as usize + interval.contact);
        trace.fraction = interval.enter.max(0.0);
        trace.plane = sides[interval.contact];
        trace.surface = surface;
        trace.surface_id = surface_id;
        if let Some(index) = interval.secondary_contact {
            trace.secondary_plane = Some(sides[index]);
            // qsrc stores the primary side's surface in both native slots.
            trace.secondary_surface_id = surface_id;
        }
        trace.contents = brush.contents;
    }
}

/// Loaded geometry and stack-built temporary bodies share these kernels.
pub(super) fn trace_brushes(
    work: &BrushWork<'_>,
    planes: &[Plane],
    brushes: &[Brush],
    surfaces: SurfaceRows<'_>,
) -> Trace {
    let query = work.query;
    let mut trace = Trace::clear(query.end);
    for &brush in brushes {
        if !brush.contents.intersects(query.mask) {
            continue;
        }
        if query.start == query.end {
            let sides = &planes[brush.sides.range()];
            position_brush(work, planes, brush, axial_prefix(sides), &mut trace);
        } else {
            clip_brush(work, planes, brush, surfaces, &mut trace);
        }
        if trace.fraction == 0.0 {
            break;
        }
    }
    trace.end = work.end_position(trace.fraction);
    trace
}
