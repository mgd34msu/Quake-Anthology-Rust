//! Quake III client presentation collision world.
//!
//! Port of `src/app/bootstrap/q3-client/collision.ts`
//! (`q3ClientCollision`). Cgame clips its own snapshot entities after
//! the geometry trace, so this adapter only forwards geometry queries
//! to the shared map under the `q3:cgame` binary32 numeric profile with
//! the caller's curve policy. Temporary box geometry dispatches through
//! the injected map seam because the box hull lives with the collision
//! owner, outside this wave's scope.

use qa_content::q3::base::world::{TraceContact, TraceShape, TraceSolidity};
use qa_content::q3::presentation::collision_host::{CollisionWorld, TraceQuery, TraceResult};
use qa_core::math::Vec3;

/// Collision map settings snapshot (donor `CollisionMapSettings`).
///
/// The donor reads live cvars; the owner refreshes this snapshot with
/// [`Q3ClientCollision::set_settings`] when `cm_noCurves` or
/// `cm_playerCurveClip` change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionMapSettings {
    /// Skip curve patches.
    pub no_curves: bool,
    /// Clip players against curve patches.
    pub player_curve_clip: bool,
}

/// Collision counters (donor `CollisionCounters`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CollisionCounters {
    /// Geometry traces.
    pub c_traces: i32,
    /// Brush traces.
    pub c_brush_traces: i32,
    /// Patch traces.
    pub c_patch_traces: i32,
    /// Point-contents queries.
    pub c_pointcontents: i32,
}

impl CollisionCounters {
    /// Reset all counters.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Shared-map trace query under Q3 policy.
#[derive(Debug, Clone, Copy)]
pub struct Q3MapTrace<'a> {
    /// Trace start.
    pub start: Vec3,
    /// Trace end.
    pub end: Vec3,
    /// Trace shape.
    pub shape: &'a TraceShape,
    /// Target model index.
    pub model: i32,
    /// Target origin.
    pub origin: Vec3,
    /// Target angles.
    pub angles: Vec3,
    /// Contents mask.
    pub mask: i32,
    /// Trace curve patches.
    pub curves: bool,
    /// Clip players against curve patches.
    pub player_curve_clip: bool,
}

/// Shared-map trace hit under Q3 policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3MapTraceHit {
    /// Completed fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Started inside solid.
    pub all_solid: bool,
    /// Start point is solid.
    pub start_solid: bool,
    /// Contact record.
    pub contact: TraceContact,
    /// Contents at the hit.
    pub contents: i32,
    /// Surface flags at the hit.
    pub surface_flags: i32,
}

/// Shared-map queries backing the client collision world.
///
/// All queries run under the `q3:cgame` binary32 numeric profile with a
/// Q3 contents policy; the map result is always a Q3 record.
pub trait Q3ClientMapQueries {
    /// Trace shared-map geometry.
    fn trace_q3(&mut self, query: &Q3MapTrace<'_>) -> Q3MapTraceHit;
    /// Read shared-map point contents.
    fn point_contents_q3(
        &mut self,
        point: Vec3,
        model: i32,
        origin: Vec3,
        angles: Vec3,
        curves: bool,
        player_curve_clip: bool,
    ) -> i32;
    /// Trace the temporary box hull owned by the collision owner.
    fn box_trace_q3(&mut self, mins: Vec3, maxs: Vec3, query: &Q3MapTrace<'_>) -> Q3MapTraceHit;
}

fn zero_vec3() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

fn solidity(all_solid: bool, start_solid: bool) -> TraceSolidity {
    if all_solid {
        TraceSolidity::AllSolid
    } else if start_solid {
        TraceSolidity::StartSolid
    } else {
        TraceSolidity::Clear
    }
}

fn view(hit: &Q3MapTraceHit) -> TraceResult {
    TraceResult {
        fraction: hit.fraction,
        end: hit.end,
        solidity: solidity(hit.all_solid, hit.start_solid),
        contact: hit.contact,
        contents: hit.contents,
        surface_flags: hit.surface_flags,
    }
}

/// Client presentation collision world over shared-map queries.
pub struct Q3ClientCollision<Q> {
    queries: Q,
    settings: CollisionMapSettings,
    counters: CollisionCounters,
}

impl<Q: Q3ClientMapQueries> Q3ClientCollision<Q> {
    /// Wrap shared-map queries with curve settings.
    pub fn new(queries: Q, settings: CollisionMapSettings) -> Self {
        Self {
            queries,
            settings,
            counters: CollisionCounters::default(),
        }
    }

    /// Current curve settings.
    #[must_use]
    pub fn settings(&self) -> CollisionMapSettings {
        self.settings
    }

    /// Refresh curve settings from live cvars.
    pub fn set_settings(&mut self, settings: CollisionMapSettings) {
        self.settings = settings;
    }

    /// Collision counters.
    #[must_use]
    pub fn counters(&self) -> &CollisionCounters {
        &self.counters
    }

    fn traced(&mut self, query: &TraceQuery, model: i32, origin: Vec3, angles: Vec3) -> TraceResult {
        self.counters.c_traces += 1;
        let hit = self.queries.trace_q3(&Q3MapTrace {
            start: query.start,
            end: query.end,
            shape: &query.shape,
            model,
            origin,
            angles,
            mask: query.mask,
            curves: !self.settings.no_curves,
            player_curve_clip: self.settings.player_curve_clip,
        });
        view(&hit)
    }

    fn contents(&mut self, point: Vec3, model: i32, origin: Vec3, angles: Vec3) -> i32 {
        self.counters.c_pointcontents += 1;
        self.queries.point_contents_q3(
            point,
            model,
            origin,
            angles,
            !self.settings.no_curves,
            self.settings.player_curve_clip,
        )
    }
}

impl<Q: Q3ClientMapQueries> CollisionWorld for Q3ClientCollision<Q> {
    fn trace(&mut self, query: &TraceQuery) -> TraceResult {
        let zero = zero_vec3();
        let model = query.model_index.unwrap_or(0);
        self.traced(query, model, zero, zero)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        let zero = zero_vec3();
        self.contents(point, 0, zero, zero)
    }

    fn transformed_trace(&mut self, query: &TraceQuery, model_index: i32, origin: Vec3, angles: Vec3) -> TraceResult {
        self.traced(query, model_index, origin, angles)
    }

    fn transformed_point_contents(&mut self, point: Vec3, model_index: i32, origin: Vec3, angles: Vec3) -> i32 {
        self.contents(point, model_index, origin, angles)
    }

    fn box_trace(&mut self, mins: Vec3, maxs: Vec3, query: &TraceQuery, origin: Vec3) -> TraceResult {
        self.counters.c_traces += 1;
        let hit = self.queries.box_trace_q3(
            mins,
            maxs,
            &Q3MapTrace {
                start: query.start,
                end: query.end,
                shape: &query.shape,
                model: query.model_index.unwrap_or(0),
                origin,
                angles: zero_vec3(),
                mask: query.mask,
                curves: !self.settings.no_curves,
                player_curve_clip: self.settings.player_curve_clip,
            },
        );
        view(&hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubMap {
        traces: Vec<(i32, Vec3, Vec3, i32, bool, bool)>,
        contents: Vec<(Vec3, i32, bool, bool)>,
        boxes: Vec<(Vec3, Vec3)>,
    }

    impl StubMap {
        fn new() -> Self {
            Self {
                traces: Vec::new(),
                contents: Vec::new(),
                boxes: Vec::new(),
            }
        }

        fn hit(fraction: f32) -> Q3MapTraceHit {
            Q3MapTraceHit {
                fraction,
                end: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                all_solid: false,
                start_solid: fraction == 0.0,
                contact: TraceContact::None,
                contents: 5,
                surface_flags: 6,
            }
        }
    }

    impl Q3ClientMapQueries for StubMap {
        fn trace_q3(&mut self, query: &Q3MapTrace<'_>) -> Q3MapTraceHit {
            self.traces.push((
                query.model,
                query.origin,
                query.angles,
                query.mask,
                query.curves,
                query.player_curve_clip,
            ));
            Self::hit(0.5)
        }

        fn point_contents_q3(
            &mut self,
            point: Vec3,
            model: i32,
            origin: Vec3,
            _angles: Vec3,
            curves: bool,
            player_curve_clip: bool,
        ) -> i32 {
            self.contents.push((point, model, curves, player_curve_clip));
            let _ = origin;
            9
        }

        fn box_trace_q3(&mut self, mins: Vec3, maxs: Vec3, query: &Q3MapTrace<'_>) -> Q3MapTraceHit {
            self.boxes.push((mins, maxs));
            let _ = query;
            Self::hit(1.0)
        }
    }

    fn settings() -> CollisionMapSettings {
        CollisionMapSettings {
            no_curves: false,
            player_curve_clip: true,
        }
    }

    fn query() -> TraceQuery {
        TraceQuery {
            start: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            end: Vec3 { x: 0.0, y: 0.0, z: 8.0 },
            shape: TraceShape::Point,
            mask: 1,
            model_index: Some(3),
        }
    }

    #[test]
    fn trace_forwards_model_and_policy() {
        let mut world = Q3ClientCollision::new(StubMap::new(), settings());
        let result = world.trace(&query());
        assert_eq!(result.fraction, 0.5);
        assert_eq!(result.solidity, TraceSolidity::Clear);
        assert_eq!(result.contents, 5);
        assert_eq!(result.surface_flags, 6);
        assert_eq!(world.counters().c_traces, 1);
    }

    #[test]
    fn trace_defaults_missing_model_to_world() {
        let mut world = Q3ClientCollision::new(StubMap::new(), settings());
        let mut plain = query();
        plain.model_index = None;
        world.trace(&plain);
        // The stub records (model, origin, angles, mask, curves, player clip).
        world.point_contents(Vec3 { x: 1.0, y: 1.0, z: 1.0 });
        assert_eq!(world.counters().c_traces, 1);
        assert_eq!(world.counters().c_pointcontents, 1);
    }

    #[test]
    fn transformed_queries_carry_origin_and_angles() {
        let mut world = Q3ClientCollision::new(StubMap::new(), settings());
        let origin = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
        let angles = Vec3 {
            x: 0.0,
            y: 90.0,
            z: 0.0,
        };
        world.transformed_trace(&query(), 7, origin, angles);
        assert_eq!(
            world.transformed_point_contents(Vec3 { x: 0.0, y: 0.0, z: 0.0 }, 7, origin, angles),
            9
        );
        assert_eq!(world.counters().c_traces, 1);
        assert_eq!(world.counters().c_pointcontents, 1);
    }

    #[test]
    fn settings_refresh_changes_curve_policy() {
        let mut world = Q3ClientCollision::new(StubMap::new(), settings());
        world.set_settings(CollisionMapSettings {
            no_curves: true,
            player_curve_clip: false,
        });
        assert_eq!(
            world.settings(),
            CollisionMapSettings {
                no_curves: true,
                player_curve_clip: false
            }
        );
        world.trace(&query());
        assert_eq!(world.counters().c_traces, 1);
    }

    #[test]
    fn solidity_maps_start_and_all_solid() {
        assert_eq!(solidity(false, false), TraceSolidity::Clear);
        assert_eq!(solidity(false, true), TraceSolidity::StartSolid);
        assert_eq!(solidity(true, false), TraceSolidity::AllSolid);
        assert_eq!(solidity(true, true), TraceSolidity::AllSolid);
    }

    #[test]
    fn box_trace_dispatches_to_owner_hull() {
        let mut world = Q3ClientCollision::new(StubMap::new(), settings());
        let result = world.box_trace(
            Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            &query(),
            Vec3 { x: 2.0, y: 0.0, z: 0.0 },
        );
        assert_eq!(result.fraction, 1.0);
        assert_eq!(world.counters().c_traces, 1);
    }
}
