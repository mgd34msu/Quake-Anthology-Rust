//! Quake III collision world translated from id Software's `cm_trace.c`,
//! `cm_test.c` and `cm_load.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/world.ts`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_core::math::{add3, angles_to_axis, dot3, scale3, sub3, vec3, Bounds, Plane, Vec3};
use qa_core::math::{js_max_f32 as js_max, js_min_f32 as js_min};

use super::clip_models::{
    ClipState, SourceClipModels, TemporaryStorage, SOURCE_BOX_MODEL_HANDLE, SOURCE_CAPSULE_MODEL_HANDLE,
};
use super::counters::CollisionCounters;
use super::map_resource::{CollisionBoxHull, CollisionMapData, CollisionPlane};
use super::patch::{position_in_patch, trace_patch, CollisionDebugSurface, PatchShape};
use super::settings::CollisionMapSettings;
use super::topology::{BoxLeafList, CollisionTopology, SourceClusterPVS};
use crate::collision::{convert_contents, trace_brush_media, MediumBrush, TraceMedia};
use crate::error::WorldError;
use crate::save::value::SaveJson;
use crate::spatial::CollisionFamily;

/// Sweep body for world traces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point sweep.
    Point,
    /// Box sweep with model-frame corners.
    Box {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
    /// Capsule sweep with model-frame corners.
    Capsule {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
}

/// Trace query against the world or one submodel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceQuery {
    /// Sweep start.
    pub start: Vec3,
    /// Sweep end.
    pub end: Vec3,
    /// Sweep body.
    pub shape: TraceShape,
    /// Contents mask.
    pub mask: i32,
    /// Submodel index (world when `None`).
    pub model_index: Option<i32>,
    /// Trace patch surfaces (`None` enables).
    pub curves: Option<bool>,
    /// Clip point traces against patches (`None` enables).
    pub player_curve_clip: Option<bool>,
}

/// How far a trace got before stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// The sweep completed.
    Clear,
    /// The sweep started inside solid geometry.
    StartSolid,
    /// The whole sweep stayed inside solid geometry.
    AllSolid,
}

/// What a trace touched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// Nothing.
    None,
    /// A contact plane.
    Plane(Plane),
}

/// Source trace view over a [`SourceTraceResult`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceResult {
    /// Impact fraction.
    pub fraction: f32,
    /// Sweep end at impact.
    pub end: Vec3,
    /// Solidity at impact.
    pub solidity: TraceSolidity,
    /// Contact plane, when the trace hit and moved.
    pub contact: TraceContact,
    /// Contents at impact.
    pub contents: i32,
    /// Surface flags at impact.
    pub surface_flags: i32,
}

/// Source trace plane with BSP type and sign bits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceTracePlane {
    /// Unit normal.
    pub normal: Vec3,
    /// Distance from the origin.
    pub distance: f32,
    /// Axial type.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: i32,
}

/// Raw source trace record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceTraceResult {
    /// The whole sweep stayed inside solid geometry.
    pub all_solid: bool,
    /// The sweep started inside solid geometry.
    pub start_solid: bool,
    /// Impact fraction.
    pub fraction: f32,
    /// Sweep end at impact.
    pub end: Vec3,
    /// Impact plane.
    pub plane: SourceTracePlane,
    /// Surface flags at impact.
    pub surface_flags: i32,
    /// Contents at impact.
    pub contents: i32,
}

/// Prepared capsule sweep against the temporary box model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapsuleReplacementTrace {
    /// Centered sweep start.
    pub start: Vec3,
    /// Centered sweep end.
    pub end: Vec3,
    /// Center-relative minimum corner.
    pub mins: Vec3,
    /// Center-relative extents.
    pub extents: Vec3,
    /// Capsule radius.
    pub radius: f32,
    /// End-cap offset from the center.
    pub offset: Vec3,
    /// Replacement bounds.
    pub bounds: Bounds,
    /// The sweep does not move.
    pub stationary: bool,
    /// The sweep is a point trace.
    pub point_trace: bool,
}

/// Model-frame transform for submodel queries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTransform {
    /// Model origin.
    pub origin: Vec3,
    /// Model angles.
    pub angles: Vec3,
}

/// Debug/profile wiring for a collision world.
#[derive(Debug, Clone)]
pub enum CollisionWorldProfile {
    /// Headless world without debug or settings.
    Disabled,
    /// Shared world with a common debug surface and settings.
    Shared {
        /// Common debug surface.
        owner: Rc<CollisionDebugSurface>,
        /// Common settings.
        settings: CollisionMapSettings,
    },
}

/// Empty source trace: fraction 1, clear of everything.
#[must_use]
pub fn empty_source_trace() -> SourceTraceResult {
    SourceTraceResult {
        all_solid: false,
        start_solid: false,
        fraction: 1.0,
        end: vec3(0.0, 0.0, 0.0),
        plane: SourceTracePlane {
            normal: vec3(0.0, 0.0, 0.0),
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        },
        surface_flags: 0,
        contents: 0,
    }
}

/// Project a source record into its trace view.
#[must_use]
pub fn source_trace_view(result: &SourceTraceResult) -> TraceResult {
    TraceResult {
        fraction: result.fraction,
        end: result.end,
        solidity: if result.all_solid {
            TraceSolidity::AllSolid
        } else if result.start_solid {
            TraceSolidity::StartSolid
        } else {
            TraceSolidity::Clear
        },
        contact: if result.fraction == 1.0 || result.start_solid {
            TraceContact::None
        } else {
            TraceContact::Plane(Plane {
                normal: result.plane.normal,
                distance: result.plane.distance,
            })
        },
        contents: result.contents,
        surface_flags: result.surface_flags,
    }
}

/// Interpolate a sweep end at `fraction`.
#[must_use]
pub fn source_trace_end(start: Vec3, end: Vec3, fraction: f64) -> Vec3 {
    let reach =
        |from: f32, to: f32| -> f32 { (f64::from(from) + f64::from((fraction * f64::from(to - from)) as f32)) as f32 };
    vec3(reach(start.x, end.x), reach(start.y, end.y), reach(start.z, end.z))
}

/// Squared distance between two collision vectors.
#[must_use]
pub fn collision_vector_distance_squared(a: Vec3, b: Vec3) -> f32 {
    dot3(sub3(a, b), sub3(a, b))
}

pub(crate) fn world_at<T>(values: &[T], index: i64) -> Result<&T, WorldError> {
    if index < 0 {
        return Err(WorldError::BadCollisionRecord(format!(
            "collision index {index} out of range"
        )));
    }
    values
        .get(index as usize)
        .ok_or_else(|| WorldError::BadCollisionRecord(format!("collision index {index} out of range")))
}

struct ShapeInfo {
    center: Vec3,
    mins: Vec3,
    shape: PatchShape,
}

fn shape_info(shape: &TraceShape) -> Result<ShapeInfo, WorldError> {
    match shape {
        TraceShape::Point => Ok(ShapeInfo {
            center: vec3(0.0, 0.0, 0.0),
            mins: vec3(0.0, 0.0, 0.0),
            shape: PatchShape::Point {
                mins: vec3(0.0, 0.0, 0.0),
                extents: vec3(0.0, 0.0, 0.0),
            },
        }),
        TraceShape::Box { mins, maxs } | TraceShape::Capsule { mins, maxs } => {
            let corners = [*mins, *maxs];
            if corners
                .iter()
                .any(|corner| !corner.x.is_finite() || !corner.y.is_finite() || !corner.z.is_finite())
                || mins.x > maxs.x
                || mins.y > maxs.y
                || mins.z > maxs.z
            {
                return Err(WorldError::BadCollisionRecord(
                    "trace shape requires finite ordered bounds".to_string(),
                ));
            }
            let center = scale3(add3(*mins, *maxs), 0.5);
            let mins = sub3(*mins, center);
            let extents = sub3(*maxs, center);
            if matches!(shape, TraceShape::Capsule { .. }) {
                let radius = js_min(extents.x, extents.z);
                Ok(ShapeInfo {
                    center,
                    mins,
                    shape: PatchShape::Capsule {
                        extents,
                        radius,
                        offset: vec3(0.0, 0.0, extents.z - radius),
                    },
                })
            } else {
                Ok(ShapeInfo {
                    center,
                    mins,
                    shape: PatchShape::Box { mins, extents },
                })
            }
        }
    }
}

/// Unrounded sweep expansion; callers add it to the plane distance inside one
/// binary64 chain that rounds once, mirroring the donor's
/// `Math.fround(plane.distance + expansion)`.
fn expansion(shape: &PatchShape, plane: &CollisionPlane) -> Result<f64, WorldError> {
    match shape {
        PatchShape::Point { .. } => Ok(0.0),
        PatchShape::Capsule { radius, offset, .. } => {
            Ok(f64::from(*radius) + f64::from(dot3(plane.normal, *offset).abs()))
        }
        PatchShape::Box { mins, extents } => {
            if !(0..8).contains(&plane.signbits) {
                return Err(WorldError::BadCollisionRecord(
                    "CM brush plane signbits outside trace offsets".to_string(),
                ));
            }
            // cm_trace.c's eight centered-box corners, indexed by the stored
            // signs; a set bit selects the maximum corner.
            Ok(-f64::from(dot3(
                plane.normal,
                vec3(
                    if plane.signbits & 1 != 0 { extents.x } else { mins.x },
                    if plane.signbits & 2 != 0 { extents.y } else { mins.y },
                    if plane.signbits & 4 != 0 { extents.z } else { mins.z },
                ),
            )))
        }
    }
}

/// Node-split interpolation with the donor's single rounding: the binary64
/// chain rounds once, unlike the shared per-operation `lerp3`.
fn trace_lerp(from: Vec3, to: Vec3, fraction: f64) -> Vec3 {
    vec3(
        (f64::from(from.x) + fraction * (f64::from(to.x) - f64::from(from.x))) as f32,
        (f64::from(from.y) + fraction * (f64::from(to.y) - f64::from(from.y))) as f32,
        (f64::from(from.z) + fraction * (f64::from(to.z) - f64::from(from.z))) as f32,
    )
}

/// Stationary capsule bounds with the donor's single rounding per component.
fn capsule_position_bounds(start: Vec3, offset: Vec3, radius: f32) -> Bounds {
    let grow = |at: f32, span: f32| -> (f32, f32) {
        (
            (f64::from(at) - f64::from(span) - f64::from(radius)) as f32,
            (f64::from(at) + f64::from(span) + f64::from(radius)) as f32,
        )
    };
    let (min_x, max_x) = grow(start.x, offset.x.abs());
    let (min_y, max_y) = grow(start.y, offset.y.abs());
    let (min_z, max_z) = grow(start.z, offset.z.abs());
    Bounds {
        min: vec3(min_x, min_y, min_z),
        max: vec3(max_x, max_y, max_z),
    }
}

fn trace_query_finite(query: &TraceQuery) -> bool {
    query.start.x.is_finite()
        && query.start.y.is_finite()
        && query.start.z.is_finite()
        && query.end.x.is_finite()
        && query.end.y.is_finite()
        && query.end.z.is_finite()
}

/// Collision world: traces, contents, and topology over a map.
#[derive(Debug)]
pub struct CollisionWorld {
    map: Rc<CollisionMapData>,
    topology: CollisionTopology,
    debug: Option<Rc<CollisionDebugSurface>>,
    settings: RefCell<Option<CollisionMapSettings>>,
    counters: Rc<CollisionCounters>,
    clip_state: RefCell<ClipState>,
}

impl CollisionWorld {
    /// Borrow a map with a debug profile and shared counters.
    #[must_use]
    pub fn new(map: Rc<CollisionMapData>, profile: CollisionWorldProfile, counters: Rc<CollisionCounters>) -> Self {
        let (debug, settings) = match profile {
            CollisionWorldProfile::Disabled => (None, None),
            CollisionWorldProfile::Shared { owner, settings } => (Some(owner), Some(settings)),
        };
        let topology = CollisionTopology::new(map.clone(), counters.clone());
        Self {
            map,
            topology,
            debug,
            settings: RefCell::new(settings),
            counters,
            clip_state: RefCell::new(ClipState::new()),
        }
    }

    /// Shared source counters.
    #[must_use]
    pub fn counters(&self) -> &Rc<CollisionCounters> {
        &self.counters
    }

    /// Borrow the map records.
    #[must_use]
    pub fn map(&self) -> &Rc<CollisionMapData> {
        &self.map
    }

    /// Borrow the topology.
    #[must_use]
    pub fn topology(&self) -> &CollisionTopology {
        &self.topology
    }

    /// Borrow the debug surface, if shared.
    #[must_use]
    pub fn debug(&self) -> Option<&Rc<CollisionDebugSurface>> {
        self.debug.as_ref()
    }

    /// Borrow the clip-model state.
    pub(crate) fn clip_state(&self) -> &RefCell<ClipState> {
        &self.clip_state
    }

    /// Area count.
    #[must_use]
    pub fn area_count(&self) -> usize {
        self.topology.area_count
    }

    /// Cluster count.
    #[must_use]
    pub fn cluster_count(&self) -> i32 {
        self.topology.cluster_count
    }

    /// Model count.
    #[must_use]
    pub fn model_count(&self) -> usize {
        self.map.models.len()
    }

    /// Whether the map ships BSP nodes.
    #[must_use]
    pub fn has_nodes(&self) -> bool {
        !self.map.nodes.is_empty()
    }

    /// Borrow the initialized box hull, if any.
    #[must_use]
    pub fn box_storage(&self) -> Option<&CollisionBoxHull> {
        self.map.box_hull.as_ref()
    }

    /// Register and bind common settings to this world.
    pub fn bind_settings(&self, settings: CollisionMapSettings) -> Result<(), WorldError> {
        settings.register_map()?;
        *self.settings.borrow_mut() = Some(settings);
        Ok(())
    }

    fn no_curves(&self) -> Result<bool, WorldError> {
        match self.settings.borrow().as_ref() {
            None => Ok(false),
            Some(settings) => settings.no_curves(),
        }
    }

    fn player_curve_clip(&self) -> Result<bool, WorldError> {
        match self.settings.borrow().as_ref() {
            None => Ok(true),
            Some(settings) => settings.player_curve_clip(),
        }
    }

    fn settings_no_areas(&self) -> Result<Option<bool>, WorldError> {
        match self.settings.borrow().as_ref() {
            None => Ok(None),
            Some(settings) => Ok(Some(settings.no_areas()?)),
        }
    }

    /// Advance the map visit stamp.
    pub fn advance_check_count(&self) {
        CollisionCounters::bump(&self.map.check_count);
    }

    /// Bounds of a submodel.
    pub fn model_bounds(&self, index: i32) -> Result<Bounds, WorldError> {
        Ok(world_at(&self.map.models, index as i64)?.bounds)
    }

    /// Leaf containing a point.
    pub fn point_leafnum(&self, point: Vec3) -> Result<usize, WorldError> {
        self.topology.point_leafnum(point)
    }

    /// Leaves touched by bounds.
    pub fn box_leafnums(&self, bounds: &Bounds, max_leaves: usize) -> Result<BoxLeafList, WorldError> {
        self.topology.box_leafnums(bounds, max_leaves)
    }

    /// Brush indexes touched by bounds.
    pub fn box_brushes(&self, bounds: &Bounds, max_brushes: i32) -> Result<Vec<usize>, WorldError> {
        self.topology.box_brushes(bounds, max_brushes)
    }

    /// Area containing a leaf.
    pub fn leaf_area(&self, index: i32) -> Result<i32, WorldError> {
        self.topology.leaf_area(index)
    }

    /// Cluster containing a leaf.
    pub fn leaf_cluster(&self, index: i32) -> Result<i32, WorldError> {
        self.topology.leaf_cluster(index)
    }

    /// Borrow a cluster's PVS row.
    pub fn cluster_pvs(&self, cluster: i32) -> SourceClusterPVS<'_> {
        self.topology.cluster_pvs(cluster)
    }

    /// Test cluster visibility through the PVS.
    pub fn cluster_visible(&self, from: i32, to: i32) -> Result<bool, WorldError> {
        self.topology.cluster_visible(from, to)
    }

    /// Capture the portal/area checkpoint.
    pub fn capture_portal_checkpoint(&self) -> SaveJson {
        self.topology.capture_portal_checkpoint()
    }

    /// Restore a portal/area checkpoint.
    pub fn restore_portal_checkpoint(&self, value: &SaveJson) -> Result<(), WorldError> {
        self.topology.restore_portal_checkpoint(value)
    }

    /// Adjust the portal reference count between two areas and reflood.
    pub fn adjust_area_portal_state(&self, area1: i32, area2: i32, open: bool) -> Result<(), WorldError> {
        self.topology.adjust_area_portal_state(area1, area2, open)
    }

    /// Test whether two areas connect, honoring common settings.
    pub fn areas_connected(&self, area1: i32, area2: i32) -> Result<bool, WorldError> {
        let no_areas = self.settings_no_areas()?;
        self.topology.areas_connected(area1, area2, no_areas)
    }

    /// Write area visibility bits; returns the byte count.
    pub fn write_area_bits(&self, buffer: &mut [u8], area: i32) -> Result<usize, WorldError> {
        let no_areas = self.settings_no_areas()?;
        self.topology.write_area_bits(buffer, area, no_areas)
    }

    /// Fresh area visibility bits.
    pub fn area_bits(&self, area: i32) -> Result<Vec<u8>, WorldError> {
        let no_areas = self.settings_no_areas()?;
        self.topology.area_bits(area, no_areas)
    }

    /// Override area connectivity directly (unshared worlds only).
    pub fn set_no_areas(&self, enabled: bool) -> Result<(), WorldError> {
        if self.settings.borrow().is_some() {
            return Err(WorldError::BadCollisionRecord(
                "Shared collision worlds read cm_noAreas from their common cvars".to_string(),
            ));
        }
        self.topology.set_no_areas(enabled);
        Ok(())
    }

    /// Borrow this world's clip models.
    #[must_use]
    pub fn source_clip_models(&self) -> SourceClipModels<'_> {
        SourceClipModels::new(self, TemporaryStorage::World)
    }

    /// Contents at a point inside a submodel.
    pub fn point_contents(&self, point: Vec3, model_index: i32) -> Result<i32, WorldError> {
        if !trace_point_finite(point) {
            return Err(WorldError::BadCollisionRecord(
                "point contents requires finite coordinates".to_string(),
            ));
        }
        let model = world_at(&self.map.models, model_index as i64)?;
        let mut contents = 0;
        if model_index != 0 {
            for item in 0..model.brushes.len() {
                let index = self.map.model_brush_at(model_index as usize, item)?;
                contents |= self.brush_point_contents(point, index)?;
            }
            return Ok(contents);
        }
        let leaf = world_at(&self.map.leaves, self.topology.point_leafnum(point)? as i64)?;
        for index in 0..leaf.brush_count {
            let brush = self.map.leaf_brush_at(leaf.first_brush + index)?;
            contents |= self.brush_point_contents(point, brush)?;
        }
        Ok(contents)
    }

    fn brush_point_contents(&self, point: Vec3, index: i32) -> Result<i32, WorldError> {
        let brush = world_at(&self.map.brushes, index as i64)?;
        let mut inside = true;
        for side in 0..brush.side_count {
            let record = world_at(&self.map.brush_sides, (brush.first_side + side) as i64)?;
            let plane = world_at(&self.map.planes, record.plane as i64)?;
            if dot3(point, plane.normal) > plane.distance {
                inside = false;
                break;
            }
        }
        Ok(if inside { brush.contents } else { 0 })
    }

    /// Contents at a point inside a transformed submodel.
    pub fn transformed_point_contents(
        &self,
        point: Vec3,
        model_index: i32,
        origin: Vec3,
        angles: Vec3,
    ) -> Result<i32, WorldError> {
        let local = sub3(point, origin);
        if model_index == SOURCE_BOX_MODEL_HANDLE || (angles.x == 0.0 && angles.y == 0.0 && angles.z == 0.0) {
            return self.point_contents(local, model_index);
        }
        let axis = angles_to_axis(angles);
        let rotated = vec3(dot3(local, axis[0]), dot3(local, axis[1]), dot3(local, axis[2]));
        self.point_contents(rotated, model_index)
    }

    /// Trace a sweep and project its source record.
    pub fn trace(&self, query: &TraceQuery) -> Result<TraceResult, WorldError> {
        Ok(source_trace_view(&self.trace_source(query)?))
    }
}

fn trace_point_finite(point: Vec3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

struct TraceContext<'a> {
    query: &'a TraceQuery,
    model_index: i32,
    start: Vec3,
    end: Vec3,
    shape: PatchShape,
    size_mins: Vec3,
    stationary: bool,
    point_trace: bool,
    replacement: Option<&'a CapsuleReplacementTrace>,
    position_bounds: Option<Bounds>,
    fraction: f32,
    all_solid: bool,
    start_solid: bool,
    trace_plane: SourceTracePlane,
    contents: i32,
    surface_flags: i32,
}

impl CollisionWorld {
    /// Raw source trace against the world or one submodel.
    pub fn trace_source(&self, query: &TraceQuery) -> Result<SourceTraceResult, WorldError> {
        if query.model_index == Some(SOURCE_CAPSULE_MODEL_HANDLE) {
            let narrowed = TraceQuery {
                model_index: None,
                ..*query
            };
            return self.source_clip_models().trace(&narrowed, SOURCE_CAPSULE_MODEL_HANDLE);
        }
        let info = shape_info(&query.shape)?;
        let start = add3(query.start, info.center);
        let end = add3(query.end, info.center);
        let stationary = query.start.x == query.end.x && query.start.y == query.end.y && query.start.z == query.end.z;
        let point_trace = !stationary && info.mins.x == 0.0 && info.mins.y == 0.0 && info.mins.z == 0.0;
        self.run_trace(TraceContext {
            query,
            model_index: query.model_index.unwrap_or(0),
            start,
            end,
            shape: info.shape,
            size_mins: info.mins,
            stationary,
            point_trace,
            replacement: None,
            position_bounds: None,
            fraction: 1.0,
            all_solid: false,
            start_solid: false,
            trace_plane: empty_source_trace().plane,
            contents: 0,
            surface_flags: 0,
        })
    }

    /// Raw source trace for a prepared capsule replacement.
    pub fn trace_capsule_replacement_source(
        &self,
        query: &TraceQuery,
        prepared: &CapsuleReplacementTrace,
    ) -> Result<SourceTraceResult, WorldError> {
        let narrowed = TraceQuery {
            model_index: Some(SOURCE_BOX_MODEL_HANDLE),
            ..*query
        };
        let shape = PatchShape::Capsule {
            extents: prepared.extents,
            radius: prepared.radius,
            offset: prepared.offset,
        };
        self.run_trace(TraceContext {
            query: &narrowed,
            model_index: SOURCE_BOX_MODEL_HANDLE,
            start: prepared.start,
            end: prepared.end,
            shape,
            size_mins: prepared.mins,
            stationary: prepared.stationary,
            point_trace: prepared.point_trace,
            replacement: Some(prepared),
            position_bounds: None,
            fraction: 1.0,
            all_solid: false,
            start_solid: false,
            trace_plane: empty_source_trace().plane,
            contents: 0,
            surface_flags: 0,
        })
    }

    fn run_trace(&self, mut ctx: TraceContext) -> Result<SourceTraceResult, WorldError> {
        if !trace_query_finite(ctx.query) {
            return Err(WorldError::BadCollisionRecord(
                "trace requires finite coordinates and integer contents mask".to_string(),
            ));
        }
        world_at(&self.map.models, ctx.model_index as i64)?;
        if ctx.replacement.is_none() {
            self.advance_check_count();
            CollisionCounters::bump(&self.counters.c_traces);
        }
        if ctx.stationary {
            // CM_TestBoxInBrush reads retained bounds, including the capsule
            // swap's original bounds.
            let bounds = match ctx.replacement {
                Some(prepared) => prepared.bounds,
                None => match &ctx.shape {
                    PatchShape::Capsule { radius, offset, .. } => capsule_position_bounds(ctx.start, *offset, *radius),
                    PatchShape::Point { .. } => Bounds {
                        min: add3(ctx.start, ctx.size_mins),
                        max: add3(ctx.start, vec3(0.0, 0.0, 0.0)),
                    },
                    PatchShape::Box { extents, .. } => Bounds {
                        min: add3(ctx.start, ctx.size_mins),
                        max: add3(ctx.start, *extents),
                    },
                },
            };
            ctx.position_bounds = Some(bounds);
            if ctx.model_index != 0 {
                let model = world_at(&self.map.models, ctx.model_index as i64)?;
                for item in 0..model.brushes.len() {
                    let index = self.map.model_brush_at(ctx.model_index as usize, item)?;
                    self.trace_brush(&mut ctx, index)?;
                    if ctx.fraction == 0.0 {
                        break;
                    }
                }
                if self.trace_curves(ctx.query)? {
                    let model = world_at(&self.map.models, ctx.model_index as i64)?;
                    for item in 0..model.surfaces.len() {
                        let index = self.map.model_surface_at(ctx.model_index as usize, item)?;
                        self.trace_patch_surface(&mut ctx, index)?;
                        if ctx.fraction == 0.0 {
                            break;
                        }
                    }
                }
            } else {
                let extents = match &ctx.shape {
                    PatchShape::Point { .. } => vec3(0.0, 0.0, 0.0),
                    PatchShape::Box { extents, .. } | PatchShape::Capsule { extents, .. } => *extents,
                };
                let envelope = Bounds {
                    min: sub3(add3(ctx.start, ctx.size_mins), vec3(1.0, 1.0, 1.0)),
                    max: add3(add3(ctx.start, extents), vec3(1.0, 1.0, 1.0)),
                };
                let leaves = self.topology.box_leafnums(&envelope, 1024)?;
                // The stationary leaf list advances its own stamp; the trace
                // below continues with a fresh epoch.
                self.advance_check_count();
                for leaf in &leaves.leaves {
                    self.leaf_trace(&mut ctx, *leaf)?;
                    if ctx.all_solid {
                        break;
                    }
                }
            }
        } else if ctx.model_index != 0 {
            let model = world_at(&self.map.models, ctx.model_index as i64)?;
            for item in 0..model.brushes.len() {
                let index = self.map.model_brush_at(ctx.model_index as usize, item)?;
                self.trace_brush(&mut ctx, index)?;
                if ctx.fraction == 0.0 {
                    break;
                }
            }
            if self.trace_curves(ctx.query)? {
                let model = world_at(&self.map.models, ctx.model_index as i64)?;
                for item in 0..model.surfaces.len() {
                    let index = self.map.model_surface_at(ctx.model_index as usize, item)?;
                    self.trace_patch_surface(&mut ctx, index)?;
                    if ctx.fraction == 0.0 {
                        break;
                    }
                }
            }
        } else if self.map.nodes.is_empty() {
            self.leaf_trace(&mut ctx, 0)?;
        } else {
            let (start, end) = (ctx.start, ctx.end);
            self.tree_trace(&mut ctx, 0, 0.0, 1.0, start, end)?;
        }
        Ok(SourceTraceResult {
            all_solid: ctx.all_solid,
            start_solid: ctx.start_solid,
            fraction: ctx.fraction,
            end: if ctx.fraction == 1.0 {
                ctx.query.end
            } else {
                source_trace_end(ctx.query.start, ctx.query.end, f64::from(ctx.fraction))
            },
            plane: ctx.trace_plane,
            surface_flags: ctx.surface_flags,
            contents: ctx.contents,
        })
    }

    fn trace_curves(&self, query: &TraceQuery) -> Result<bool, WorldError> {
        if query.curves == Some(false) {
            return Ok(false);
        }
        Ok(!self.no_curves()?)
    }

    fn trace_brush(&self, ctx: &mut TraceContext, index: i32) -> Result<(), WorldError> {
        let brush = world_at(&self.map.brushes, index as i64)?;
        if brush.check_count.get() == self.map.check_count.get() {
            return Ok(());
        }
        brush.check_count.set(self.map.check_count.get());
        let flags = brush.contents;
        if flags & ctx.query.mask == 0 {
            return Ok(());
        }
        if brush.side_count == 0 {
            return Ok(());
        }
        if !ctx.stationary {
            CollisionCounters::bump(&self.counters.c_brush_traces);
        }
        if let Some(position) = ctx.position_bounds {
            let brush_bounds = self.map.brush_bounds(index as usize)?;
            if position.min.x > brush_bounds.max.x
                || position.min.y > brush_bounds.max.y
                || position.min.z > brush_bounds.max.z
                || position.max.x < brush_bounds.min.x
                || position.max.y < brush_bounds.min.y
                || position.max.z < brush_bounds.min.z
            {
                return Ok(());
            }
        }
        let (first_side, side_count) = (brush.first_side, brush.side_count);
        let mut enter_fraction = -1.0f32;
        let mut leave_fraction = 1.0f32;
        let mut start_out = false;
        let mut get_out = false;
        let mut lead: Option<(SourceTracePlane, i32)> = None;
        // Stationary box-model tests skip the six axial planes, which the
        // retained bounds already cover.
        let first = if ctx.stationary { 6 } else { 0 };
        for side in first..side_count {
            let record = world_at(&self.map.brush_sides, (first_side + side) as i64)?;
            let plane = world_at(&self.map.planes, record.plane as i64)?;
            let distance = (f64::from(plane.distance) + expansion(&ctx.shape, plane)?) as f32;
            let (mut first, mut last) = (ctx.start, ctx.end);
            if let PatchShape::Capsule { offset, .. } = &ctx.shape {
                if dot3(plane.normal, *offset) > 0.0 {
                    first = sub3(ctx.start, *offset);
                    last = sub3(ctx.end, *offset);
                } else {
                    first = add3(ctx.start, *offset);
                    last = add3(ctx.end, *offset);
                }
            }
            let d1 = dot3(first, plane.normal) - distance;
            let d2 = dot3(last, plane.normal) - distance;
            if ctx.stationary {
                if d1 > 0.0 {
                    return Ok(());
                }
                continue;
            }
            if d1 > 0.0 {
                start_out = true;
            }
            if d2 > 0.0 {
                get_out = true;
            }
            if d1 > 0.0 && (d2 >= 0.125 || d2 >= d1) {
                return Ok(());
            }
            if d1 <= 0.0 && d2 <= 0.0 {
                continue;
            }
            if d1 > d2 {
                let crossed = 0.0f32.max((d1 - 0.125) / (d1 - d2));
                if crossed > enter_fraction {
                    enter_fraction = crossed;
                    lead = Some((
                        SourceTracePlane {
                            normal: plane.normal,
                            distance: plane.distance,
                            plane_type: plane.plane_type,
                            signbits: plane.signbits,
                        },
                        record.surface_flags,
                    ));
                }
            } else {
                leave_fraction = leave_fraction.min(1.0f32.min((d1 + 0.125) / (d1 - d2)));
            }
        }
        if !start_out {
            ctx.start_solid = true;
            if !get_out {
                ctx.all_solid = true;
                ctx.fraction = 0.0;
                ctx.contents = flags;
            }
            return Ok(());
        }
        if enter_fraction < leave_fraction && enter_fraction > -1.0 && enter_fraction < ctx.fraction {
            if let Some((plane, surface_flags)) = lead {
                ctx.fraction = enter_fraction.max(0.0);
                ctx.trace_plane = plane;
                ctx.contents = flags;
                ctx.surface_flags = surface_flags;
            }
        }
        Ok(())
    }

    fn trace_patch_surface(&self, ctx: &mut TraceContext, index: i32) -> Result<(), WorldError> {
        let surface_number = usize::try_from(index)
            .map_err(|_| WorldError::BadCollisionRecord(format!("collision index {index} out of range")))?;
        let patch = self.map.patch(surface_number)?;
        let Some(patch) = patch else {
            return Ok(());
        };
        if patch.check_count.get() == self.map.check_count.get() {
            return Ok(());
        }
        patch.check_count.set(self.map.check_count.get());
        if patch.contents & ctx.query.mask == 0 {
            return Ok(());
        }
        if ctx.stationary {
            let position = match &ctx.shape {
                PatchShape::Point { mins, extents } => PatchShape::Box {
                    mins: *mins,
                    extents: *extents,
                },
                shape => *shape,
            };
            if position_in_patch(&patch.collide, ctx.start, &position)? {
                ctx.start_solid = true;
                ctx.all_solid = true;
                ctx.fraction = 0.0;
                ctx.contents = patch.contents;
            }
            return Ok(());
        }
        CollisionCounters::bump(&self.counters.c_patch_traces);
        let point_trace = match ctx.replacement {
            Some(prepared) => prepared.point_trace,
            None => ctx.point_trace,
        };
        let patch_shape = if point_trace {
            let extents = match &ctx.shape {
                PatchShape::Point { extents, .. } => *extents,
                PatchShape::Box { extents, .. } => *extents,
                PatchShape::Capsule { extents, .. } => *extents,
            };
            PatchShape::Point {
                mins: ctx.size_mins,
                extents,
            }
        } else {
            ctx.shape
        };
        if matches!(patch_shape, PatchShape::Point { .. })
            && (ctx.query.player_curve_clip == Some(false) || !self.player_curve_clip()?)
        {
            return Ok(());
        }
        if let Some(hit) = trace_patch(
            &patch.collide,
            ctx.start,
            ctx.end,
            &patch_shape,
            ctx.fraction,
            self.debug.as_deref(),
        )? {
            ctx.fraction = hit.fraction;
            ctx.trace_plane.normal = hit.plane.normal;
            ctx.trace_plane.distance = hit.plane.distance;
            ctx.contents = patch.contents;
            ctx.surface_flags = patch.surface_flags;
        }
        Ok(())
    }

    fn leaf_trace(&self, ctx: &mut TraceContext, leaf: usize) -> Result<(), WorldError> {
        let record = world_at(&self.map.leaves, leaf as i64)?;
        for index in 0..record.brush_count {
            let brush = self.map.leaf_brush_at(record.first_brush + index)?;
            self.trace_brush(ctx, brush)?;
            if ctx.fraction == 0.0 {
                return Ok(());
            }
        }
        if !self.trace_curves(ctx.query)? {
            return Ok(());
        }
        let record = world_at(&self.map.leaves, leaf as i64)?;
        for index in 0..record.surface_count {
            let surface = self.map.leaf_surface_at(record.first_surface + index)?;
            self.trace_patch_surface(ctx, surface)?;
            if ctx.fraction == 0.0 {
                return Ok(());
            }
        }
        Ok(())
    }

    fn tree_trace(
        &self,
        ctx: &mut TraceContext,
        index: i32,
        p1_fraction: f32,
        p2_fraction: f32,
        p1: Vec3,
        p2: Vec3,
    ) -> Result<(), WorldError> {
        if ctx.fraction <= p1_fraction {
            return Ok(());
        }
        if index < 0 {
            return self.leaf_trace(ctx, (-1 - index) as usize);
        }
        let node = world_at(&self.map.nodes, index as i64)?;
        let plane = world_at(&self.map.planes, node.plane as i64)?;
        let t1 = if plane.plane_type < 3 {
            let along = if plane.plane_type == 0 {
                p1.x
            } else if plane.plane_type == 1 {
                p1.y
            } else {
                p1.z
            };
            along - plane.distance
        } else {
            dot3(p1, plane.normal) - plane.distance
        };
        let t2 = if plane.plane_type < 3 {
            let along = if plane.plane_type == 0 {
                p2.x
            } else if plane.plane_type == 1 {
                p2.y
            } else {
                p2.z
            };
            along - plane.distance
        } else {
            dot3(p2, plane.normal) - plane.distance
        };
        let offset = if !ctx.point_trace && !matches!(ctx.shape, PatchShape::Point { .. }) {
            let extents = match &ctx.shape {
                PatchShape::Box { extents, .. } => *extents,
                PatchShape::Capsule { extents, .. } => *extents,
                PatchShape::Point { .. } => vec3(0.0, 0.0, 0.0),
            };
            if plane.plane_type == 0 {
                extents.x
            } else if plane.plane_type == 1 {
                extents.y
            } else if plane.plane_type == 2 {
                extents.z
            } else {
                2048.0
            }
        } else {
            0.0
        };
        // The donor widens the binary32 inputs once and runs the split in
        // unrounded binary64, rounding only at the stored fractions/points.
        let (t1, t2, reach) = (f64::from(t1), f64::from(t2), f64::from(offset));
        if t1 >= reach + 1.0 && t2 >= reach + 1.0 {
            return self.tree_trace(ctx, node.children[0], p1_fraction, p2_fraction, p1, p2);
        }
        if t1 < -reach - 1.0 && t2 < -reach - 1.0 {
            return self.tree_trace(ctx, node.children[1], p1_fraction, p2_fraction, p1, p2);
        }
        let mut side: usize = 0;
        let (mut f1, mut f2) = (1.0f64, 0.0f64);
        if t1 < t2 {
            side = 1;
            f2 = (t1 + reach + 0.125) / (t1 - t2);
            f1 = (t1 - reach + 0.125) / (t1 - t2);
        } else if t1 > t2 {
            f2 = (t1 - reach - 0.125) / (t1 - t2);
            f1 = (t1 + reach + 0.125) / (t1 - t2);
        }
        let f1 = js_max(0.0, js_min(1.0, f1 as f32));
        let f2 = js_max(0.0, js_min(1.0, f2 as f32));
        let mid1 = trace_lerp(p1, p2, f64::from(f1));
        let mid2 = trace_lerp(p1, p2, f64::from(f2));
        let span = f64::from(p2_fraction) - f64::from(p1_fraction);
        let mid1_fraction = (f64::from(p1_fraction) + span * f64::from(f1)) as f32;
        let mid2_fraction = (f64::from(p1_fraction) + span * f64::from(f2)) as f32;
        let children = node.children;
        self.tree_trace(ctx, children[side], p1_fraction, mid1_fraction, p1, mid1)?;
        self.tree_trace(ctx, children[1 - side], mid2_fraction, p2_fraction, mid2, p2)
    }
}

impl CollisionWorld {
    /// Transformed sweep projected into its trace view.
    pub fn transformed_trace(&self, query: &TraceQuery, origin: Vec3, angles: Vec3) -> Result<TraceResult, WorldError> {
        Ok(source_trace_view(
            &self.transformed_trace_source(query, origin, angles)?,
        ))
    }

    /// Raw source trace against a transformed submodel.
    pub fn transformed_trace_source(
        &self,
        query: &TraceQuery,
        origin: Vec3,
        angles: Vec3,
    ) -> Result<SourceTraceResult, WorldError> {
        if query.model_index == Some(SOURCE_CAPSULE_MODEL_HANDLE) {
            return self
                .source_clip_models()
                .transformed_trace(query, SOURCE_CAPSULE_MODEL_HANDLE, origin, angles);
        }
        if !trace_point_finite(origin)
            || (query.model_index != Some(SOURCE_BOX_MODEL_HANDLE) && !trace_point_finite(angles))
        {
            return Err(WorldError::BadCollisionRecord(
                "model transform requires finite coordinates".to_string(),
            ));
        }
        let info = shape_info(&query.shape)?;
        let axis = if query.model_index != Some(SOURCE_BOX_MODEL_HANDLE)
            && (angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0)
        {
            Some(angles_to_axis(angles))
        } else {
            None
        };
        let rotate = |point: Vec3| -> Vec3 {
            match &axis {
                None => point,
                Some(axis) => vec3(dot3(point, axis[0]), dot3(point, axis[1]), dot3(point, axis[2])),
            }
        };
        let start = rotate(sub3(add3(query.start, info.center), origin));
        let finish = rotate(sub3(add3(query.end, info.center), origin));
        let stationary = start.x == finish.x && start.y == finish.y && start.z == finish.z;
        let first_extents = match &info.shape {
            PatchShape::Point { extents, .. } => *extents,
            PatchShape::Box { extents, .. } => *extents,
            PatchShape::Capsule { extents, .. } => *extents,
        };
        let center = scale3(add3(info.mins, first_extents), 0.5);
        let mins = sub3(info.mins, center);
        let mut shape = info.shape;
        if let PatchShape::Capsule { offset, .. } = &shape {
            // CM_Trace retains the supplied sphere while centering the stored
            // sizes again.
            let offset = match &axis {
                None => *offset,
                Some(axis) => vec3(axis[0].z * offset.z, -axis[1].z * offset.z, axis[2].z * offset.z),
            };
            if let PatchShape::Capsule {
                extents, offset: slot, ..
            } = &mut shape
            {
                *extents = sub3(first_extents, center);
                *slot = offset;
            }
        } else if let PatchShape::Box { .. } = &shape {
            shape = PatchShape::Box {
                mins,
                extents: sub3(first_extents, center),
            };
        }
        let mut result = self.run_trace(TraceContext {
            query,
            model_index: query.model_index.unwrap_or(0),
            start: add3(start, center),
            end: add3(finish, center),
            shape,
            size_mins: mins,
            stationary,
            point_trace: !stationary && mins.x == 0.0 && mins.y == 0.0 && mins.z == 0.0,
            replacement: None,
            position_bounds: None,
            fraction: 1.0,
            all_solid: false,
            start_solid: false,
            trace_plane: empty_source_trace().plane,
            contents: 0,
            surface_flags: 0,
        })?;
        result.end = source_trace_end(query.start, query.end, f64::from(result.fraction));
        let Some(axis) = axis else {
            return Ok(result);
        };
        if result.fraction == 1.0 {
            return Ok(result);
        }
        let normal = result.plane.normal;
        result.plane.normal = add3(
            add3(scale3(axis[0], normal.x), scale3(axis[1], normal.y)),
            scale3(axis[2], normal.z),
        );
        Ok(result)
    }

    /// Classify the media along a sweep without changing visitation.
    pub fn trace_media(
        &self,
        query: &TraceQuery,
        fraction: f64,
        transform: Option<&ModelTransform>,
    ) -> Result<TraceMedia, WorldError> {
        let info = shape_info(&query.shape)?;
        let model_index = query.model_index.unwrap_or(0);
        let axis = if transform.is_some() && model_index != SOURCE_BOX_MODEL_HANDLE {
            transform.map(|transform| angles_to_axis(transform.angles))
        } else {
            None
        };
        let rotate = |point: Vec3| -> Vec3 {
            match &axis {
                None => point,
                Some(axis) => vec3(dot3(point, axis[0]), dot3(point, axis[1]), dot3(point, axis[2])),
            }
        };
        let local = |point: Vec3| -> Vec3 {
            match transform {
                None => add3(point, info.center),
                Some(transform) => rotate(sub3(add3(point, info.center), transform.origin)),
            }
        };
        let mut start = local(query.start);
        let mut end = local(query.end);
        let mut shape = info.shape;
        let mut mins = info.mins;
        if transform.is_some() {
            let extents = match &shape {
                PatchShape::Point { extents, .. } => *extents,
                PatchShape::Box { extents, .. } => *extents,
                PatchShape::Capsule { extents, .. } => *extents,
            };
            let center = scale3(add3(mins, extents), 0.5);
            mins = sub3(mins, center);
            start = add3(start, center);
            end = add3(end, center);
            match &mut shape {
                PatchShape::Capsule {
                    extents: slot, offset, ..
                } => {
                    *slot = sub3(extents, center);
                    if let Some(axis) = &axis {
                        *offset = vec3(axis[0].z * offset.z, -axis[1].z * offset.z, axis[2].z * offset.z);
                    }
                }
                PatchShape::Box {
                    mins: slot_mins,
                    extents: slot,
                } => {
                    *slot_mins = mins;
                    *slot = sub3(extents, center);
                }
                PatchShape::Point { .. } => {}
            }
        }
        let (size_max, size_min) = match &shape {
            PatchShape::Capsule { radius, offset, .. } => {
                let grown = vec3(
                    offset.x.abs() + radius,
                    offset.y.abs() + radius,
                    offset.z.abs() + radius,
                );
                (grown, scale3(grown, -1.0))
            }
            PatchShape::Box { extents, .. } => (*extents, mins),
            PatchShape::Point { extents, .. } => (*extents, mins),
        };
        let reached = source_trace_end(start, end, fraction);
        // The donor rounds each envelope component once from binary64.
        let envelope = Bounds {
            min: vec3(
                (f64::from(js_min(start.x, reached.x)) + f64::from(size_min.x) - 1.0) as f32,
                (f64::from(js_min(start.y, reached.y)) + f64::from(size_min.y) - 1.0) as f32,
                (f64::from(js_min(start.z, reached.z)) + f64::from(size_min.z) - 1.0) as f32,
            ),
            max: vec3(
                (f64::from(js_max(start.x, reached.x)) + f64::from(size_max.x) + 1.0) as f32,
                (f64::from(js_max(start.y, reached.y)) + f64::from(size_max.y) + 1.0) as f32,
                (f64::from(js_max(start.z, reached.z)) + f64::from(size_max.z) + 1.0) as f32,
            ),
        };
        let mut candidates = Vec::new();
        if model_index != 0 {
            let model = world_at(&self.map.models, model_index as i64)?;
            for item in 0..model.brushes.len() {
                candidates.push(self.map.model_brush_at(model_index as usize, item)?);
            }
        } else {
            self.topology.visit_leaves(&envelope, &mut |leafnum| {
                let leaf = world_at(&self.map.leaves, leafnum as i64)?;
                for index in 0..leaf.brush_count {
                    candidates.push(self.map.leaf_brush_at(leaf.first_brush + index)?);
                }
                Ok(())
            })?;
        }
        let mut seen = HashSet::new();
        let mut brushes = Vec::new();
        for candidate in candidates {
            if !seen.insert(candidate) {
                continue;
            }
            let brush = world_at(&self.map.brushes, candidate as i64)?;
            if convert_contents(brush.contents, CollisionFamily::Q3, CollisionFamily::Q1) == -1 {
                continue;
            }
            let mut distances = Vec::new();
            for side in 0..brush.side_count {
                let record = world_at(&self.map.brush_sides, (brush.first_side + side) as i64)?;
                let plane = world_at(&self.map.planes, record.plane as i64)?;
                let grown = match &shape {
                    PatchShape::Capsule { radius, .. } => f64::from(*radius),
                    other => expansion(other, plane)?,
                };
                let distance = (f64::from(plane.distance) + grown) as f32;
                let (mut first, mut last) = (start, end);
                if let PatchShape::Capsule { offset, .. } = &shape {
                    if dot3(plane.normal, *offset) > 0.0 {
                        first = sub3(start, *offset);
                        last = sub3(end, *offset);
                    } else {
                        first = add3(start, *offset);
                        last = add3(end, *offset);
                    }
                }
                distances.push((
                    f64::from(dot3(first, plane.normal) - distance),
                    f64::from(dot3(last, plane.normal) - distance),
                ));
            }
            // Distances above are rounded once per operation, exactly as the
            // donor rounds before handing pairs to the shared classifier.
            brushes.push(MediumBrush {
                contents: brush.contents,
                distances,
            });
        }
        Ok(trace_brush_media(&brushes, CollisionFamily::Q3, fraction))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::cvar::CvarRegistry;
    use qa_core::math::Plane;

    use super::super::map_resource::{
        decoded_collision_map, CollisionShader, IndexRange, Q3BspChild, Q3CollisionBrushInput,
        Q3CollisionBrushSideInput, Q3CollisionGeometry, Q3CollisionLeafInput, Q3CollisionModelInput,
        Q3CollisionNodeInput, Q3CollisionSurfaceInput, Q3SurfaceKind,
    };

    fn shader(name: &str, surface: i32, content: i32) -> CollisionShader {
        CollisionShader {
            name: name.to_string(),
            surface_flags: surface,
            content_flags: content,
        }
    }

    /// Cube brush plus a patch sheet east of it, split by a z=0 node.
    fn trace_geometry() -> Q3CollisionGeometry {
        let mut planes = vec![Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
        }];
        for (normal, distance) in [
            (vec3(1.0, 0.0, 0.0), 64.0),
            (vec3(-1.0, 0.0, 0.0), 64.0),
            (vec3(0.0, 1.0, 0.0), 64.0),
            (vec3(0.0, -1.0, 0.0), 64.0),
            (vec3(0.0, 0.0, 1.0), 64.0),
            (vec3(0.0, 0.0, -1.0), 64.0),
        ] {
            planes.push(Plane { normal, distance });
        }
        let mut vertices = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                vertices.push(vec3(100.0 + x as f32 * 64.0, (y - 1) as f32 * 64.0, 0.0));
            }
        }
        Q3CollisionGeometry {
            entities: String::new(),
            shaders: vec![shader("solid", 1, 1), shader("sheet", 2, 2)],
            planes,
            nodes: vec![Q3CollisionNodeInput {
                plane: 0,
                children: [Q3BspChild::Leaf(0), Q3BspChild::Leaf(1)],
            }],
            leaves: vec![
                Q3CollisionLeafInput {
                    cluster: 0,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 1 },
                    surfaces: IndexRange { first: 0, count: 1 },
                },
                Q3CollisionLeafInput {
                    cluster: 1,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 0 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
            ],
            leaf_brushes: vec![0],
            leaf_surfaces: vec![0],
            models: vec![Q3CollisionModelInput {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                brushes: IndexRange { first: 0, count: 0 },
                surfaces: IndexRange { first: 0, count: 0 },
            }],
            brushes: vec![Q3CollisionBrushInput {
                shader: 0,
                sides: IndexRange { first: 0, count: 6 },
            }],
            brush_sides: (1..7)
                .map(|plane| Q3CollisionBrushSideInput { plane, shader: 0 })
                .collect(),
            vertices,
            surfaces: vec![Q3CollisionSurfaceInput {
                kind: Q3SurfaceKind::Patch { width: 3, height: 3 },
                shader: 1,
                vertices: IndexRange { first: 0, count: 9 },
            }],
            visibility: None,
        }
    }

    fn trace_world() -> CollisionWorld {
        let map = decoded_collision_map(&trace_geometry(), None).expect("decode");
        CollisionWorld::new(
            Rc::new(map),
            CollisionWorldProfile::Disabled,
            Rc::new(CollisionCounters::new()),
        )
    }

    fn point_query(start: Vec3, end: Vec3, mask: i32) -> TraceQuery {
        TraceQuery {
            start,
            end,
            shape: TraceShape::Point,
            mask,
            model_index: None,
            curves: None,
            player_curve_clip: None,
        }
    }

    #[test]
    fn world_trace_matches_donor() {
        let world = trace_world();
        let hit = world
            .trace(&point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1))
            .expect("trace");
        assert_eq!(hit.fraction, 0.17937499284744263f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 64.125));
        assert_eq!(hit.solidity, TraceSolidity::Clear);
        assert_eq!(
            hit.contact,
            TraceContact::Plane(Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 64.0,
            })
        );
        assert_eq!(hit.surface_flags, 1);
        assert_eq!(hit.contents, 1);

        let shape = TraceShape::Box {
            mins: vec3(-8.0, -8.0, -8.0),
            maxs: vec3(8.0, 8.0, 8.0),
        };
        let hit = world
            .trace(&TraceQuery {
                start: vec3(0.0, 0.0, 100.0),
                end: vec3(0.0, 0.0, -100.0),
                shape,
                mask: 1,
                model_index: None,
                curves: None,
                player_curve_clip: None,
            })
            .expect("trace");
        assert_eq!(hit.fraction, 0.1393750011920929f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 72.125));

        let shape = TraceShape::Capsule {
            mins: vec3(-8.0, -8.0, -8.0),
            maxs: vec3(8.0, 8.0, 8.0),
        };
        let hit = world
            .trace(&TraceQuery {
                start: vec3(0.0, 0.0, 100.0),
                end: vec3(0.0, 0.0, -100.0),
                shape,
                mask: 1,
                model_index: None,
                curves: None,
                player_curve_clip: None,
            })
            .expect("trace");
        assert_eq!(hit.fraction, 0.1393750011920929f64 as f32);

        let miss = world
            .trace(&point_query(vec3(500.0, 0.0, 100.0), vec3(500.0, 0.0, -100.0), 1))
            .expect("trace");
        assert_eq!(miss.fraction, 1.0);
        assert_eq!(miss.contact, TraceContact::None);

        let stuck = world
            .trace(&point_query(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), 1))
            .expect("trace");
        assert_eq!(stuck.fraction, 0.0);
        assert_eq!(stuck.solidity, TraceSolidity::AllSolid);
        assert_eq!(stuck.contents, 1);
    }

    #[test]
    fn world_contents_and_leaves_match_donor() {
        let world = trace_world();
        assert_eq!(world.point_contents(vec3(0.0, 0.0, 0.0), 0).expect("in"), 1);
        assert_eq!(world.point_contents(vec3(0.0, 0.0, 100.0), 0).expect("out"), 0);
        assert_eq!(world.point_leafnum(vec3(0.0, 0.0, 5.0)).expect("leaf"), 0);
        assert_eq!(world.point_leafnum(vec3(0.0, 0.0, -5.0)).expect("leaf"), 1);
        assert!(world.cluster_visible(0, 1).expect("visible"));
        let media = world
            .trace_media(
                &point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1),
                1.0,
                None,
            )
            .expect("media");
        assert!(media.in_open);
        assert!(!media.in_water);
    }

    #[test]
    fn world_trace_reaches_patches_and_honors_no_curves() {
        let world = trace_world();
        let query = point_query(vec3(164.0, 0.0, 100.0), vec3(164.0, 0.0, -100.0), 3);
        let hit = world.trace(&query).expect("trace");
        assert!(hit.fraction < 1.0, "patch sheet blocks the sweep");
        assert_eq!(hit.contents, 2);
        assert_eq!(hit.surface_flags, 2);

        let cvars = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        let settings = CollisionMapSettings::new(cvars.clone());
        world.bind_settings(settings).expect("bind");
        cvars.borrow_mut().set("cm_noCurves", "1", true).expect("toggle");
        let miss = world.trace(&query).expect("trace");
        assert_eq!(miss.fraction, 1.0);
    }

    #[test]
    fn world_validates_queries() {
        let world = trace_world();
        let error = world
            .trace(&point_query(vec3(f32::NAN, 0.0, 0.0), vec3(0.0, 0.0, 0.0), 1))
            .expect_err("NaN must fail");
        assert_eq!(
            error.to_string(),
            "trace requires finite coordinates and integer contents mask"
        );
        let error = world
            .trace(&TraceQuery {
                model_index: Some(9),
                ..point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1)
            })
            .expect_err("model 9 is missing");
        assert_eq!(error.to_string(), "collision index 9 out of range");
        let error = world
            .trace(&TraceQuery {
                shape: TraceShape::Box {
                    mins: vec3(8.0, 0.0, 0.0),
                    maxs: vec3(-8.0, 0.0, 0.0),
                },
                ..point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1)
            })
            .expect_err("unordered shape must fail");
        assert_eq!(error.to_string(), "trace shape requires finite ordered bounds");
        let error = world
            .point_contents(vec3(f32::INFINITY, 0.0, 0.0), 0)
            .expect_err("infinite point must fail");
        assert_eq!(error.to_string(), "point contents requires finite coordinates");
    }

    #[test]
    fn world_helpers_cover_views_and_topology() {
        assert_eq!(
            source_trace_end(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 10.0), 0.5),
            vec3(0.0, 0.0, 5.0)
        );
        assert_eq!(
            collision_vector_distance_squared(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
            1.0
        );
        let empty = empty_source_trace();
        assert_eq!(source_trace_view(&empty).solidity, TraceSolidity::Clear);
        let world = trace_world();
        assert_eq!(world.model_count(), 1);
        assert!(world.has_nodes());
        assert_eq!(world.cluster_count(), 2);
        assert_eq!(world.area_count(), 1);
        assert!(world.box_storage().is_none());
        assert!(world.debug().is_none());
        let checkpoint = world.capture_portal_checkpoint();
        world.restore_portal_checkpoint(&checkpoint).expect("restore");
        assert!(world.areas_connected(0, 0).expect("self"));
        world.set_no_areas(true).expect("unshared flag");
        let error = world.model_bounds(4).expect_err("model 4 is missing");
        assert_eq!(error.to_string(), "collision index 4 out of range");
    }

    #[test]
    fn world_transformed_trace_shifts_with_origin() {
        let world = trace_world();
        let query = point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1);
        let shifted = world
            .transformed_trace(&query, vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, 0.0))
            .expect("trace");
        assert_eq!(shifted.solidity, TraceSolidity::StartSolid);
        let error = world
            .transformed_trace(&query, vec3(f32::NAN, 0.0, 0.0), vec3(0.0, 0.0, 0.0))
            .expect_err("NaN origin must fail");
        assert_eq!(error.to_string(), "model transform requires finite coordinates");
    }

    #[test]
    fn world_trace_matches_donor_odd_shapes() {
        let world = trace_world();
        let hit = world
            .trace(&TraceQuery {
                start: vec3(100.0, 50.0, 100.0),
                end: vec3(-30.0, -70.0, -100.0),
                shape: TraceShape::Box {
                    mins: vec3(-16.0, -4.0, -2.0),
                    maxs: vec3(4.0, 8.0, 12.0),
                },
                mask: 1,
                model_index: None,
                curves: None,
                player_curve_clip: None,
            })
            .expect("trace");
        assert_eq!(hit.fraction, 0.1693750023841858f64 as f32);
        assert_eq!(
            hit.end,
            vec3(77.98124694824219f64 as f32, 29.674999237060547f64 as f32, 66.125)
        );
        assert_eq!(
            hit.contact,
            TraceContact::Plane(Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 64.0,
            })
        );
        let hit = world
            .trace(&TraceQuery {
                start: vec3(100.0, 50.0, 100.0),
                end: vec3(-30.0, -70.0, -100.0),
                shape: TraceShape::Capsule {
                    mins: vec3(-16.0, -4.0, -12.0),
                    maxs: vec3(4.0, 8.0, 12.0),
                },
                mask: 1,
                model_index: None,
                curves: None,
                player_curve_clip: None,
            })
            .expect("trace");
        assert_eq!(hit.fraction, 0.1528846174478531f64 as f32);
        assert_eq!(
            hit.end,
            vec3(80.125, 31.653846740722656f64 as f32, 69.42308044433594f64 as f32)
        );
        assert_eq!(
            hit.contact,
            TraceContact::Plane(Plane {
                normal: vec3(1.0, 0.0, 0.0),
                distance: 64.0,
            })
        );
    }

    #[test]
    fn world_stationary_shapes_match_donor() {
        let world = trace_world();
        let boxed = |at: Vec3| TraceQuery {
            start: at,
            end: at,
            shape: TraceShape::Box {
                mins: vec3(-8.0, -8.0, -8.0),
                maxs: vec3(8.0, 8.0, 8.0),
            },
            mask: 1,
            model_index: None,
            curves: None,
            player_curve_clip: None,
        };
        let stuck = world.trace(&boxed(vec3(0.0, 0.0, 0.0))).expect("trace");
        assert_eq!(stuck.fraction, 0.0);
        assert_eq!(stuck.solidity, TraceSolidity::AllSolid);
        assert_eq!(stuck.contents, 1);
        let free = world.trace(&boxed(vec3(0.0, 0.0, 100.0))).expect("trace");
        assert_eq!(free.fraction, 1.0);
        assert_eq!(free.solidity, TraceSolidity::Clear);
        assert_eq!(free.contents, 0);
        let capsule = TraceQuery {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 0.0),
            shape: TraceShape::Capsule {
                mins: vec3(-8.0, -8.0, -8.0),
                maxs: vec3(8.0, 8.0, 8.0),
            },
            mask: 1,
            model_index: None,
            curves: None,
            player_curve_clip: None,
        };
        let stuck = world.trace(&capsule).expect("trace");
        assert_eq!(stuck.fraction, 0.0);
        assert_eq!(stuck.solidity, TraceSolidity::AllSolid);
        assert_eq!(stuck.contents, 1);
    }

    #[test]
    fn world_patch_trace_matches_donor_exact() {
        let world = trace_world();
        let query = point_query(vec3(164.0, 0.0, 100.0), vec3(164.0, 0.0, -100.0), 3);
        let hit = world.trace(&query).expect("trace");
        assert_eq!(hit.fraction, 0.49937498569488525f64 as f32);
        assert_eq!(hit.end, vec3(164.0, 0.0, 0.125));
        assert_eq!(hit.contents, 2);
        assert_eq!(hit.surface_flags, 2);
        assert_eq!(
            hit.contact,
            TraceContact::Plane(Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            })
        );
        let source = world.trace_source(&query).expect("trace");
        assert_eq!(source.plane.plane_type, 0);
        assert_eq!(source.plane.signbits, 0);
    }

    #[test]
    fn world_transformed_trace_matches_donor() {
        let world = trace_world();
        let query = point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1);
        let shifted = world
            .transformed_trace(&query, vec3(0.0, 0.0, 10.0), vec3(0.0, 0.0, 0.0))
            .expect("trace");
        assert_eq!(shifted.fraction, 0.12937499582767487f64 as f32);
        assert_eq!(shifted.end, vec3(0.0, 0.0, 74.125));
        assert_eq!(shifted.contents, 1);
        let rotated = world
            .transformed_trace(
                &point_query(vec3(100.0, 0.0, 0.0), vec3(-100.0, 0.0, 0.0), 1),
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 45.0, 0.0),
            )
            .expect("trace");
        assert_eq!(rotated.fraction, 0.046567775309085846f64 as f32);
        assert_eq!(rotated.end, vec3(90.68644714355469f64 as f32, 0.0, 0.0));
        assert_eq!(
            rotated.contact,
            TraceContact::Plane(Plane {
                normal: vec3(0.7071067690849304f64 as f32, 0.7071067690849304f64 as f32, 0.0),
                distance: 64.0,
            })
        );
    }

    #[test]
    fn world_scalar_helpers_match_donor() {
        assert_eq!(js_min(1.0, 2.0), 1.0);
        assert_eq!(js_max(1.0, 2.0), 2.0);
        assert!(js_min(f32::NAN, 1.0).is_nan());
        assert!(js_min(1.0, f32::NAN).is_nan());
        assert!(js_max(f32::NAN, 1.0).is_nan());
        assert!(js_max(1.0, f32::NAN).is_nan());
        assert_eq!(js_min(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(js_min(-0.0, 0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(js_max(0.0, -0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(js_max(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
        // Single-rounding split; per-operation rounding lands one ulp above.
        let from = vec3(-3.0, 0.0, 0.0);
        let to = vec3(-2.4699999999999998f64 as f32, 0.0, 0.0);
        assert_eq!(trace_lerp(from, to, 0.184).x, -2.902479887008667f64 as f32);
        // A non-binary32 fraction stays wide through the reach.
        assert_eq!(
            source_trace_end(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 10.0), 0.49937498569488525),
            vec3(0.0, 0.0, 4.993749618530273f64 as f32)
        );
    }
}
