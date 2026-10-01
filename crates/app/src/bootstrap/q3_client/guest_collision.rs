//! Cgame collision-model traps over shared scene geometry.
//!
//! Port of `src/app/bootstrap/q3-client/guest-collision.ts`
//! (`SharedQvmClientClipModels`). Cgame traces geometry only; its entity
//! clipping remains in the guest module. Handle validation, temporary
//! hull retention, checkpointing, and the `cm_noCurves` /
//! `cm_playerCurveClip` policy follow the donor; geometric queries
//! dispatch through [`GuestCollisionScene`] because the collision owner
//! lives outside this wave's scope. [`ClientCollisionHost`] is
//! infallible, so donor failures record a
//! [`GuestCollisionError`] (drained with
//! [`SharedQvmClientClipModels::take_error`]) and return the bridge's
//! neutral value.

use qa_core::math::Vec3;
use qa_guest::qvm::client_collision_syscalls::{ClientCollisionHost, TraceQuery, TraceRecord, BOX_MODEL_HANDLE};
use qa_guest::qvm::cvar_syscalls::CvarHost;
use thiserror::Error;

/// Temporary capsule-model handle (donor `SOURCE_CAPSULE_MODEL_HANDLE`).
pub const CAPSULE_MODEL_HANDLE: i32 = 254;

/// Guest collision failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GuestCollisionError {
    /// `CM_InlineModel: bad number`.
    #[error("CM_InlineModel: bad number")]
    BadInlineModel(i32),
    /// `CM_ClipHandleToModel: bad handle`.
    #[error("CM_ClipHandleToModel: bad handle {0}")]
    BadClipHandle(i32),
    /// Cgame requested a different collision map.
    #[error("Cgame requested a different collision map: {got}")]
    WrongCollisionMap {
        /// Expected map.
        expected: String,
        /// Requested map.
        got: String,
    },
    /// Temporary-collision checkpoint is invalid.
    #[error("Invalid temporary-collision checkpoint: {0}")]
    BadCheckpoint(String),
}

/// Axis-aligned bounds retained for the temporary hull.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CollisionBounds {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

/// Temporary-collision checkpoint (donor `captureTemporaryCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemporaryCollisionCheckpoint {
    /// Current temporary bounds.
    pub bounds: CollisionBounds,
    /// Retained box-hull bounds (frozen by capsule calls).
    pub box_bounds: CollisionBounds,
}

/// Shared scene geometry backing the cgame clip models.
pub trait GuestCollisionScene {
    /// Whether the world has BSP nodes.
    fn has_nodes(&self) -> bool;
    /// Inline model count.
    fn model_count(&self) -> i32;
    /// Inline-model point contents under Q3 policy.
    fn point_contents(
        &mut self,
        point: Vec3,
        model: i32,
        origin: Vec3,
        angles: Vec3,
        curves: bool,
        player_curve_clip: bool,
    ) -> i32;
    /// Inline-model geometry trace under Q3 policy.
    fn geometry_trace(
        &mut self,
        query: &TraceQuery,
        model: i32,
        origin: Vec3,
        angles: Vec3,
        curves: bool,
        player_curve_clip: bool,
    ) -> TraceRecord;
    /// Temporary box-hull point contents.
    fn temp_box_contents(&mut self, bounds: &CollisionBounds, point: Vec3, origin: Vec3, angles: Vec3) -> i32;
    /// Temporary box-hull trace.
    fn temp_box_trace(
        &mut self,
        bounds: &CollisionBounds,
        query: &TraceQuery,
        origin: Vec3,
        angles: Vec3,
    ) -> TraceRecord;
    /// Temporary capsule-hull point contents.
    fn temp_capsule_contents(&mut self, bounds: &CollisionBounds, point: Vec3, origin: Vec3, angles: Vec3) -> i32;
    /// Temporary capsule-hull trace.
    fn temp_capsule_trace(
        &mut self,
        bounds: &CollisionBounds,
        query: &TraceQuery,
        origin: Vec3,
        angles: Vec3,
    ) -> TraceRecord;
}

/// Resolved clip handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipHandle {
    /// Inline model index.
    Inline(i32),
    /// Temporary box hull.
    TempBox,
    /// Temporary capsule hull.
    TempCapsule,
}

fn zero_vec3() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

fn zero_bounds() -> CollisionBounds {
    CollisionBounds {
        min: zero_vec3(),
        max: zero_vec3(),
    }
}

/// Empty source trace (donor `emptySourceTrace`).
#[must_use]
pub fn empty_trace() -> TraceRecord {
    TraceRecord {
        all_solid: false,
        start_solid: false,
        fraction: 1.0,
        end: zero_vec3(),
        plane_normal: zero_vec3(),
        plane_distance: 0.0,
        plane_type: 0,
        plane_signbits: 0,
        surface_flags: 0,
        contents: 0,
        entity_num: 0,
    }
}

/// Cgame clip models shared with the scene owner.
pub struct SharedQvmClientClipModels<S, C> {
    scene: S,
    cvars: C,
    map: String,
    bounds: CollisionBounds,
    box_bounds: CollisionBounds,
    error: Option<GuestCollisionError>,
}

impl<S: GuestCollisionScene, C: CvarHost> SharedQvmClientClipModels<S, C> {
    /// Wrap scene geometry, cvars, and the expected collision map.
    pub fn new(scene: S, cvars: C, map: &str) -> Self {
        Self {
            scene,
            cvars,
            map: map.to_string(),
            bounds: zero_bounds(),
            box_bounds: zero_bounds(),
            error: None,
        }
    }

    /// Take the recorded donor failure, if any.
    #[must_use]
    pub fn take_error(&mut self) -> Option<GuestCollisionError> {
        self.error.take()
    }

    /// Capture the temporary-hull checkpoint.
    #[must_use]
    pub fn capture_temporary_checkpoint(&self) -> TemporaryCollisionCheckpoint {
        TemporaryCollisionCheckpoint {
            bounds: self.bounds,
            box_bounds: self.box_bounds,
        }
    }

    /// Restore the temporary-hull checkpoint.
    pub fn restore_temporary_checkpoint(
        &mut self,
        checkpoint: &TemporaryCollisionCheckpoint,
    ) -> Result<(), GuestCollisionError> {
        for (label, bounds) in [("box", checkpoint.box_bounds), ("bounds", checkpoint.bounds)] {
            for (axis, value) in [
                ("min.x", bounds.min.x),
                ("min.y", bounds.min.y),
                ("min.z", bounds.min.z),
                ("max.x", bounds.max.x),
                ("max.y", bounds.max.y),
                ("max.z", bounds.max.z),
            ] {
                if !value.is_finite() {
                    return Err(GuestCollisionError::BadCheckpoint(format!(
                        "{label}.{axis} is not finite"
                    )));
                }
            }
        }
        self.temp_box_model(checkpoint.box_bounds.min, checkpoint.box_bounds.max, false);
        self.temp_box_model(checkpoint.bounds.min, checkpoint.bounds.max, true);
        Ok(())
    }

    fn fail(&mut self, error: GuestCollisionError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn policy(&mut self) -> (bool, bool) {
        let no_curves = self.cvars.get("cm_noCurves").map_or(0, |value| value.integer_value);
        let curve_clip = self
            .cvars
            .get("cm_playerCurveClip")
            .map_or(1, |value| value.integer_value);
        (no_curves == 0, curve_clip != 0)
    }

    fn resolve(&mut self, handle: i32) -> Result<ClipHandle, GuestCollisionError> {
        if handle >= 0 && handle < self.scene.model_count() {
            return Ok(ClipHandle::Inline(handle));
        }
        if handle == BOX_MODEL_HANDLE {
            return Ok(ClipHandle::TempBox);
        }
        if handle == CAPSULE_MODEL_HANDLE {
            return Ok(ClipHandle::TempCapsule);
        }
        Err(GuestCollisionError::BadClipHandle(handle))
    }
}

impl<S: GuestCollisionScene, C: CvarHost> ClientCollisionHost for SharedQvmClientClipModels<S, C> {
    fn load_map(&mut self, name: &str) {
        if name != self.map {
            self.fail(GuestCollisionError::WrongCollisionMap {
                expected: self.map.clone(),
                got: name.to_string(),
            });
        }
    }

    fn model_count(&mut self) -> i32 {
        self.scene.model_count()
    }

    fn inline_model(&mut self, index: i32) -> i32 {
        if index < 0 || index >= self.scene.model_count() {
            self.fail(GuestCollisionError::BadInlineModel(index));
            return -1;
        }
        index
    }

    fn temp_box_model(&mut self, mins: Vec3, maxs: Vec3, capsule: bool) -> i32 {
        self.bounds = CollisionBounds { min: mins, max: maxs };
        // The capsule call changes the temporary bounds but leaves the
        // retained box hull intact.
        if !capsule {
            self.box_bounds = self.bounds;
        }
        if capsule {
            CAPSULE_MODEL_HANDLE
        } else {
            BOX_MODEL_HANDLE
        }
    }

    fn has_nodes(&mut self) -> bool {
        self.scene.has_nodes()
    }

    fn point_contents(&mut self, point: Vec3, handle: i32) -> i32 {
        let zero = zero_vec3();
        self.transformed_point_contents(point, handle, zero, zero)
    }

    fn transformed_point_contents(&mut self, point: Vec3, handle: i32, origin: Vec3, angles: Vec3) -> i32 {
        let resolved = match self.resolve(handle) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.fail(error);
                return 0;
            }
        };
        if !self.scene.has_nodes() {
            return 0;
        }
        let (curves, player_curve_clip) = self.policy();
        match resolved {
            ClipHandle::Inline(model) => {
                self.scene
                    .point_contents(point, model, origin, angles, curves, player_curve_clip)
            }
            ClipHandle::TempBox => {
                let bounds = self.box_bounds;
                self.scene.temp_box_contents(&bounds, point, origin, angles)
            }
            ClipHandle::TempCapsule => {
                let bounds = self.bounds;
                self.scene.temp_capsule_contents(&bounds, point, origin, angles)
            }
        }
    }

    fn trace_without_nodes(&mut self, handle: i32) -> Option<TraceRecord> {
        if self.resolve(handle).is_err() {
            self.fail(GuestCollisionError::BadClipHandle(handle));
            return Some(empty_trace());
        }
        if self.scene.has_nodes() {
            None
        } else {
            Some(empty_trace())
        }
    }

    fn trace(&mut self, query: &TraceQuery, handle: i32) -> TraceRecord {
        let zero = zero_vec3();
        self.transformed_trace(query, handle, zero, zero)
    }

    fn transformed_trace(&mut self, query: &TraceQuery, handle: i32, origin: Vec3, angles: Vec3) -> TraceRecord {
        let resolved = match self.resolve(handle) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.fail(error);
                return empty_trace();
            }
        };
        if !self.scene.has_nodes() {
            let mut empty = empty_trace();
            empty.end = query.end;
            return empty;
        }
        let (curves, player_curve_clip) = self.policy();
        match resolved {
            ClipHandle::Inline(model) => {
                self.scene
                    .geometry_trace(query, model, origin, angles, curves, player_curve_clip)
            }
            ClipHandle::TempBox => {
                let bounds = self.box_bounds;
                self.scene.temp_box_trace(&bounds, query, origin, angles)
            }
            ClipHandle::TempCapsule => {
                let bounds = self.bounds;
                self.scene.temp_capsule_trace(&bounds, query, origin, angles)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_guest::qvm::client_collision_syscalls::TraceShape;
    use qa_guest::qvm::cvar_syscalls::CvarValue;
    use std::collections::HashMap;

    struct StubScene {
        nodes: bool,
        models: i32,
        contents_calls: Vec<(Vec3, i32, bool, bool)>,
        trace_calls: Vec<(i32, i32, bool, bool)>,
        temp_box_calls: usize,
        temp_capsule_calls: usize,
    }

    impl StubScene {
        fn new(nodes: bool, models: i32) -> Self {
            Self {
                nodes,
                models,
                contents_calls: Vec::new(),
                trace_calls: Vec::new(),
                temp_box_calls: 0,
                temp_capsule_calls: 0,
            }
        }
    }

    impl GuestCollisionScene for StubScene {
        fn has_nodes(&self) -> bool {
            self.nodes
        }

        fn model_count(&self) -> i32 {
            self.models
        }

        fn point_contents(
            &mut self,
            point: Vec3,
            model: i32,
            _origin: Vec3,
            _angles: Vec3,
            curves: bool,
            player_curve_clip: bool,
        ) -> i32 {
            self.contents_calls.push((point, model, curves, player_curve_clip));
            3
        }

        fn geometry_trace(
            &mut self,
            query: &TraceQuery,
            model: i32,
            _origin: Vec3,
            _angles: Vec3,
            curves: bool,
            player_curve_clip: bool,
        ) -> TraceRecord {
            self.trace_calls.push((model, query.mask, curves, player_curve_clip));
            let mut record = empty_trace();
            record.fraction = 0.25;
            record
        }

        fn temp_box_contents(&mut self, _bounds: &CollisionBounds, _point: Vec3, _origin: Vec3, _angles: Vec3) -> i32 {
            self.temp_box_calls += 1;
            5
        }

        fn temp_box_trace(
            &mut self,
            _bounds: &CollisionBounds,
            _query: &TraceQuery,
            _origin: Vec3,
            _angles: Vec3,
        ) -> TraceRecord {
            self.temp_box_calls += 1;
            empty_trace()
        }

        fn temp_capsule_contents(
            &mut self,
            _bounds: &CollisionBounds,
            _point: Vec3,
            _origin: Vec3,
            _angles: Vec3,
        ) -> i32 {
            self.temp_capsule_calls += 1;
            7
        }

        fn temp_capsule_trace(
            &mut self,
            _bounds: &CollisionBounds,
            _query: &TraceQuery,
            _origin: Vec3,
            _angles: Vec3,
        ) -> TraceRecord {
            self.temp_capsule_calls += 1;
            empty_trace()
        }
    }

    struct StubCvars {
        values: HashMap<String, i32>,
    }

    impl StubCvars {
        fn new() -> Self {
            Self { values: HashMap::new() }
        }
    }

    impl CvarHost for StubCvars {
        fn bind_vm(&mut self, _name: &str, _default: &str, _flags: i32) -> i32 {
            0
        }
        fn read_vm(&mut self, _handle: i32) -> Option<CvarVmBinding> {
            None
        }
        fn get(&mut self, name: &str) -> Option<CvarValue> {
            self.values.get(name).map(|integer_value| CvarValue {
                value: integer_value.to_string(),
                numeric_value: *integer_value as f32,
                integer_value: *integer_value,
            })
        }
        fn set(&mut self, name: &str, value: &str) {
            if let Ok(parsed) = value.parse::<i32>() {
                self.values.insert(name.to_string(), parsed);
            }
        }
        fn set_value(&mut self, _name: &str, _value: f32) {}
        fn reset(&mut self, _name: &str) {}
        fn register(&mut self, _name: &str, _default: &str, _flags: i32) {}
        fn info_string(&mut self, _flags: i32) -> String {
            String::new()
        }
    }

    use qa_guest::qvm::cvar_syscalls::CvarVmBinding;

    fn clip_models(nodes: bool) -> SharedQvmClientClipModels<StubScene, StubCvars> {
        SharedQvmClientClipModels::new(StubScene::new(nodes, 4), StubCvars::new(), "q3dm1")
    }

    fn query() -> TraceQuery {
        TraceQuery {
            start: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            end: Vec3 { x: 0.0, y: 0.0, z: 8.0 },
            mins: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            maxs: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            shape: TraceShape::Box,
            mask: 1,
        }
    }

    #[test]
    fn inline_model_validates_range() {
        let mut models = clip_models(true);
        assert_eq!(models.inline_model(2), 2);
        assert_eq!(models.inline_model(4), -1);
        assert_eq!(models.take_error(), Some(GuestCollisionError::BadInlineModel(4)));
        assert_eq!(models.inline_model(-1), -1);
        assert_eq!(models.take_error(), Some(GuestCollisionError::BadInlineModel(-1)));
    }

    #[test]
    fn temp_handles_dispatch_to_retained_hulls() {
        let mut models = clip_models(true);
        let point = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        assert_eq!(
            models.temp_box_model(
                Vec3 {
                    x: -1.0,
                    y: -1.0,
                    z: -1.0
                },
                Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                false
            ),
            BOX_MODEL_HANDLE
        );
        assert_eq!(models.point_contents(point, BOX_MODEL_HANDLE), 5);
        assert_eq!(
            models.temp_box_model(
                Vec3 {
                    x: -2.0,
                    y: -2.0,
                    z: -2.0
                },
                Vec3 { x: 2.0, y: 2.0, z: 2.0 },
                true
            ),
            CAPSULE_MODEL_HANDLE
        );
        assert_eq!(models.point_contents(point, CAPSULE_MODEL_HANDLE), 7);
        assert_eq!(models.take_error(), None);
    }

    #[test]
    fn bad_handle_records_error() {
        let mut models = clip_models(true);
        assert_eq!(models.point_contents(zero_vec3(), 9), 0);
        assert_eq!(models.take_error(), Some(GuestCollisionError::BadClipHandle(9)));
        let record = models.trace(&query(), 9);
        assert_eq!(record, empty_trace());
        assert_eq!(models.take_error(), Some(GuestCollisionError::BadClipHandle(9)));
    }

    #[test]
    fn missing_nodes_short_circuits() {
        let mut models = clip_models(false);
        assert_eq!(models.point_contents(zero_vec3(), 1), 0);
        let record = models.trace(&query(), 1);
        assert_eq!(record.end, query().end);
        assert_eq!(record.fraction, 1.0);
        assert!(models.trace_without_nodes(1).is_some());
        let mut loaded = clip_models(true);
        assert!(loaded.trace_without_nodes(1).is_none());
        assert_eq!(models.take_error(), None);
    }

    #[test]
    fn load_map_validates_name() {
        let mut models = clip_models(true);
        models.load_map("q3dm1");
        assert_eq!(models.take_error(), None);
        models.load_map("q3dm2");
        assert_eq!(
            models.take_error(),
            Some(GuestCollisionError::WrongCollisionMap {
                expected: "q3dm1".to_string(),
                got: "q3dm2".to_string(),
            })
        );
    }

    #[test]
    fn cvar_policy_reaches_scene_queries() {
        let mut models = clip_models(true);
        models.cvars.values.insert("cm_noCurves".to_string(), 1);
        models.cvars.values.insert("cm_playerCurveClip".to_string(), 0);
        assert_eq!(models.point_contents(zero_vec3(), 1), 3);
        assert_eq!(models.trace(&query(), 1).fraction, 0.25);
        assert_eq!(models.take_error(), None);
    }

    #[test]
    fn checkpoint_round_trips_hulls() {
        let mut models = clip_models(true);
        models.temp_box_model(
            Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            false,
        );
        models.temp_box_model(
            Vec3 {
                x: -2.0,
                y: -2.0,
                z: -2.0,
            },
            Vec3 { x: 2.0, y: 2.0, z: 2.0 },
            true,
        );
        let checkpoint = models.capture_temporary_checkpoint();
        assert_eq!(checkpoint.box_bounds.min.x, -1.0);
        assert_eq!(checkpoint.bounds.min.x, -2.0);
        let mut fresh = clip_models(true);
        fresh.restore_temporary_checkpoint(&checkpoint).expect("restore");
        assert_eq!(fresh.capture_temporary_checkpoint(), checkpoint);
        let mut bad = checkpoint;
        bad.bounds.max = Vec3 {
            x: f32::NAN,
            y: 0.0,
            z: 0.0,
        };
        assert!(fresh.restore_temporary_checkpoint(&bad).is_err());
    }
}
