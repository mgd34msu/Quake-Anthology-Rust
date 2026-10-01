//! Clip handles over submodels and the temporary box hull, translated from
//! id Software's `cm_trace.c` and `sv_world.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/clip-models.ts`.
//!
//! [`SourceClipModels`] borrows its world; the mutable temporary-box state
//! it would own lives in the world's clip cell instead, so traces that
//! construct clip models on demand still observe `tempBoxModel` calls.

use qa_core::math::{add3, scale3, sub3, vec3, Bounds, Vec3};

use super::counters::CollisionCounters;
use super::model::{create_box_model, create_capsule_model, TempHull, TemporaryCollisionModel};
use super::world::{empty_source_trace, source_trace_end, CollisionWorld, SourceTraceResult, TraceQuery, TraceShape};
use crate::error::WorldError;
use crate::save::shared::{read_bounds, write_bounds};
use crate::save::value::{obj, SaveJson, SaveReader};

/// Clip handle for the temporary box model.
pub const SOURCE_BOX_MODEL_HANDLE: i32 = 255;
/// Clip handle for the temporary capsule model.
pub const SOURCE_CAPSULE_MODEL_HANDLE: i32 = 254;

/// Temporary storage mode for clip models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporaryStorage {
    /// Borrow the world's initialized box hull.
    World,
    /// Keep owned bounds without map storage.
    Private,
}

/// Mutable temporary-box state retained by the world.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipState {
    /// Retained temporary bounds (`tempBoxModel` input).
    pub bounds: Bounds,
    /// Owned temporary box bounds (storage-less worlds).
    pub box_bounds: Bounds,
}

impl ClipState {
    /// Zeroed temporary state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
            box_bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
        }
    }
}

impl Default for ClipState {
    fn default() -> Self {
        Self::new()
    }
}

enum ClipModelRef {
    Inline(i32),
    Box,
}

/// Clip handles over a world's submodels and temporary hull.
#[derive(Debug)]
pub struct SourceClipModels<'w> {
    world: &'w CollisionWorld,
    storage_mode: TemporaryStorage,
}

impl<'w> SourceClipModels<'w> {
    /// Borrow a world with a storage mode.
    #[must_use]
    pub fn new(world: &'w CollisionWorld, storage_mode: TemporaryStorage) -> Self {
        Self { world, storage_mode }
    }

    fn storage(&self) -> Option<TempHull<'w>> {
        match self.storage_mode {
            TemporaryStorage::Private => None,
            TemporaryStorage::World => self.world.box_storage().map(|hull| TempHull {
                hull,
                map_check_count: &self.world.map().check_count,
            }),
        }
    }

    /// Submodel count.
    #[must_use]
    pub fn model_count(&self) -> usize {
        self.world.model_count()
    }

    fn transient_box(&self) -> Result<TemporaryCollisionModel<'w>, WorldError> {
        create_box_model(
            self.world.clip_state().borrow().box_bounds,
            self.world.counters(),
            self.storage(),
        )
    }

    /// Capture the temporary-box checkpoint.
    pub fn capture_temporary_checkpoint(&self) -> SaveJson {
        let state = self.world.clip_state().borrow();
        let bounds = match self.storage() {
            Some(storage) => storage.hull.bounds.get(),
            None => state.bounds,
        };
        let owned = match self.storage() {
            Some(storage) => storage.hull.brush_bounds.get(),
            None => state.box_bounds,
        };
        obj(vec![("bounds", write_bounds(bounds)), ("box", write_bounds(owned))])
    }

    /// Restore a temporary-box checkpoint.
    pub fn restore_temporary_checkpoint(&self, value: &SaveJson) -> Result<(), WorldError> {
        let reader = SaveReader::at(value, "cgame-temporary-collision");
        let owned = read_bounds(reader.field("box"))?;
        let retained = read_bounds(reader.field("bounds"))?;
        self.temp_box_model(owned.min, owned.max, false)?;
        self.temp_box_model(retained.min, retained.max, true)?;
        Ok(())
    }

    /// Validate an inline model number.
    pub fn inline_model(&self, index: i32) -> Result<i32, WorldError> {
        if index < 0 || index as usize >= self.model_count() {
            return Err(WorldError::BadCollisionRecord("CM_InlineModel: bad number".to_string()));
        }
        Ok(index)
    }

    /// Replace the retained temporary box; returns the clip handle.
    pub fn temp_box_model(&self, mins: Vec3, maxs: Vec3, capsule: bool) -> Result<i32, WorldError> {
        self.world.clip_state().borrow_mut().bounds = Bounds {
            min: vec3(mins.x, mins.y, mins.z),
            max: vec3(maxs.x, maxs.y, maxs.z),
        };
        if let Some(storage) = self.storage() {
            storage.hull.set_bounds(mins, maxs, capsule);
            return Ok(if capsule {
                SOURCE_CAPSULE_MODEL_HANDLE
            } else {
                SOURCE_BOX_MODEL_HANDLE
            });
        }
        // The capsule call changes box_model bounds but leaves box_brush
        // planes intact.
        if capsule {
            return Ok(SOURCE_CAPSULE_MODEL_HANDLE);
        }
        // Rebuilding validates immediately, exactly when the donor's
        // replacement constructor would throw.
        let bounds = self.world.clip_state().borrow().bounds;
        create_box_model(bounds, self.world.counters(), None)?;
        self.world.clip_state().borrow_mut().box_bounds = bounds;
        Ok(SOURCE_BOX_MODEL_HANDLE)
    }

    /// Bounds of a clip handle.
    pub fn model_bounds(&self, handle: i32) -> Result<Bounds, WorldError> {
        match self.resolve(handle)? {
            ClipModelRef::Inline(index) => self.world.model_bounds(index),
            ClipModelRef::Box => Ok(match self.storage() {
                Some(storage) => storage.hull.bounds.get(),
                None => self.world.clip_state().borrow().bounds,
            }),
        }
    }

    /// Contents at a point inside a clip handle.
    pub fn point_contents(&self, point: Vec3, handle: i32) -> Result<i32, WorldError> {
        if !self.world.has_nodes() {
            return Ok(0);
        }
        match self.resolve(handle)? {
            ClipModelRef::Inline(index) => self.world.point_contents(point, index),
            ClipModelRef::Box => self.transient_box()?.point_contents(point),
        }
    }

    /// Contents at a point inside a transformed clip handle.
    pub fn transformed_point_contents(
        &self,
        point: Vec3,
        handle: i32,
        origin: Vec3,
        angles: Vec3,
    ) -> Result<i32, WorldError> {
        if !self.world.has_nodes() {
            return Ok(0);
        }
        let resolved = self.resolve(handle)?;
        // Even a real submodel occupying index 255 takes the source
        // no-rotation branch.
        let rotation = if handle == SOURCE_BOX_MODEL_HANDLE {
            vec3(0.0, 0.0, 0.0)
        } else {
            angles
        };
        match resolved {
            ClipModelRef::Inline(index) => self.world.transformed_point_contents(point, index, origin, rotation),
            ClipModelRef::Box => self
                .transient_box()?
                .transformed_point_contents(point, origin, rotation),
        }
    }

    /// Raw source trace against a clip handle.
    pub fn trace(&self, query: &TraceQuery, handle: i32) -> Result<SourceTraceResult, WorldError> {
        let resolved = self.resolve(handle)?;
        if !self.world.has_nodes() {
            self.world.advance_check_count();
            CollisionCounters::bump(&self.world.counters().c_traces);
            return Ok(empty_source_trace());
        }
        if handle == SOURCE_CAPSULE_MODEL_HANDLE {
            self.world.advance_check_count();
            let capsule = self.capsule(query, handle, false)?;
            return if !matches!(query.shape, TraceShape::Capsule { .. })
                && self.model_count() > SOURCE_BOX_MODEL_HANDLE as usize
            {
                capsule.trace_capsule_replacement_source(query, self.world, None)
            } else {
                capsule.trace_source(query)
            };
        }
        match resolved {
            ClipModelRef::Inline(index) => self.world.trace_source(&TraceQuery {
                model_index: Some(index),
                ..*query
            }),
            ClipModelRef::Box => {
                self.world.advance_check_count();
                self.transient_box()?.trace_source(query)
            }
        }
    }

    /// `CM_Trace` checks its handle before the no-node return, without
    /// reading vectors.
    pub fn trace_without_nodes(&self, handle: i32) -> Result<Option<SourceTraceResult>, WorldError> {
        if self.world.has_nodes() {
            return Ok(None);
        }
        self.resolve(handle)?;
        self.world.advance_check_count();
        CollisionCounters::bump(&self.world.counters().c_traces);
        Ok(Some(empty_source_trace()))
    }

    /// Raw source trace against a transformed clip handle.
    pub fn transformed_trace(
        &self,
        query: &TraceQuery,
        handle: i32,
        origin: Vec3,
        angles: Vec3,
    ) -> Result<SourceTraceResult, WorldError> {
        let resolved = self.resolve(handle)?;
        if !self.world.has_nodes() {
            self.world.advance_check_count();
            CollisionCounters::bump(&self.world.counters().c_traces);
            return Ok(SourceTraceResult {
                end: source_trace_end(query.start, query.end, 1.0),
                ..empty_source_trace()
            });
        }
        if handle == SOURCE_CAPSULE_MODEL_HANDLE {
            self.world.advance_check_count();
            let capsule = self.capsule(query, handle, true)?;
            return if !matches!(query.shape, TraceShape::Capsule { .. })
                && self.model_count() > SOURCE_BOX_MODEL_HANDLE as usize
            {
                capsule.trace_capsule_replacement_source(
                    query,
                    self.world,
                    Some(&super::world::ModelTransform { origin, angles }),
                )
            } else {
                capsule.transformed_trace_source(query, origin, angles)
            };
        }
        let rotation = if handle == SOURCE_BOX_MODEL_HANDLE {
            vec3(0.0, 0.0, 0.0)
        } else {
            angles
        };
        match resolved {
            ClipModelRef::Inline(index) => self.world.transformed_trace_source(
                &TraceQuery {
                    model_index: Some(index),
                    ..*query
                },
                origin,
                rotation,
            ),
            ClipModelRef::Box => {
                self.world.advance_check_count();
                self.transient_box()?.transformed_trace_source(query, origin, rotation)
            }
        }
    }

    fn capsule(
        &self,
        query: &TraceQuery,
        handle: i32,
        transformed: bool,
    ) -> Result<TemporaryCollisionModel<'w>, WorldError> {
        // In the pinned source 254 succeeds only when it names an actual submodel.
        let capsule = create_capsule_model(self.model_bounds(handle)?, self.world.counters(), self.storage())?;
        if !matches!(query.shape, TraceShape::Capsule { .. }) {
            // Both box-versus-capsule branches replace the retained temporary box.
            let (mins, maxs) = match &query.shape {
                TraceShape::Point => (vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
                TraceShape::Box { mins, maxs } | TraceShape::Capsule { mins, maxs } => (*mins, *maxs),
            };
            let center = scale3(add3(mins, maxs), 0.5);
            let (mut size_min, mut size_max) = (sub3(mins, center), sub3(maxs, center));
            if transformed {
                let trace_center = scale3(add3(size_min, size_max), 0.5);
                size_min = sub3(size_min, trace_center);
                size_max = sub3(size_max, trace_center);
            }
            self.temp_box_model(size_min, size_max, false)?;
        }
        Ok(capsule)
    }

    fn resolve(&self, handle: i32) -> Result<ClipModelRef, WorldError> {
        if handle < 0 {
            return Err(WorldError::BadCollisionRecord(format!(
                "CM_ClipHandleToModel: bad handle {handle}"
            )));
        }
        if (handle as usize) < self.model_count() {
            return Ok(ClipModelRef::Inline(handle));
        }
        if handle == SOURCE_BOX_MODEL_HANDLE {
            return Ok(ClipModelRef::Box);
        }
        if handle < 256 {
            return Err(WorldError::BadCollisionRecord(format!(
                "CM_ClipHandleToModel: bad handle {} < {handle} < 256",
                self.model_count()
            )));
        }
        Err(WorldError::BadCollisionRecord(format!(
            "CM_ClipHandleToModel: bad handle {}",
            handle.wrapping_add(256)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    use qa_core::math::{vec3, Plane};

    use super::super::counters::CollisionCounters;
    use super::super::map_resource::BODY_CONTENTS;
    use super::super::map_resource::{decoded_collision_map, CollisionBoxHull};
    use super::super::map_resource::{
        CollisionBrushSide, CollisionPlane, CollisionShader, IndexRange, Q3BspChild, Q3CollisionBrushInput,
        Q3CollisionBrushSideInput, Q3CollisionGeometry, Q3CollisionLeafInput, Q3CollisionModelInput,
        Q3CollisionNodeInput,
    };
    use super::super::world::{CollisionWorldProfile, TraceShape};

    const BODY: i32 = BODY_CONTENTS;

    /// Cube brush plus a z=0 split; mirrors the world-trace fixture.
    fn clip_geometry() -> Q3CollisionGeometry {
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
        Q3CollisionGeometry {
            entities: String::new(),
            shaders: vec![CollisionShader {
                name: "solid".to_string(),
                surface_flags: 1,
                content_flags: 1,
            }],
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
                    surfaces: IndexRange { first: 0, count: 0 },
                },
                Q3CollisionLeafInput {
                    cluster: 1,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 0 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
            ],
            leaf_brushes: vec![0],
            leaf_surfaces: Vec::new(),
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
            vertices: Vec::new(),
            surfaces: Vec::new(),
            visibility: None,
        }
    }

    fn clip_world() -> CollisionWorld {
        let map = decoded_collision_map(&clip_geometry(), None).expect("decode");
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
    fn clip_handles_match_donor() {
        let world = clip_world();
        let clip = SourceClipModels::new(&world, TemporaryStorage::Private);
        assert_eq!(clip.model_count(), 1);
        assert_eq!(clip.inline_model(0).expect("inline"), 0);
        let error = clip.inline_model(1).expect_err("model 1 is missing");
        assert_eq!(error.to_string(), "CM_InlineModel: bad number");
        assert_eq!(
            clip.temp_box_model(vec3(-16.0, -16.0, -16.0), vec3(16.0, 16.0, 16.0), false)
                .expect("temp box"),
            SOURCE_BOX_MODEL_HANDLE
        );
        assert_eq!(
            clip.model_bounds(SOURCE_BOX_MODEL_HANDLE).expect("bounds"),
            Bounds {
                min: vec3(-16.0, -16.0, -16.0),
                max: vec3(16.0, 16.0, 16.0),
            }
        );
        let hit = clip
            .trace(
                &point_query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), BODY),
                SOURCE_BOX_MODEL_HANDLE,
            )
            .expect("trace");
        assert_eq!(hit.fraction, 0.3387500047683716f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 16.125));
        assert_eq!(hit.contents, BODY);
        let inline = clip
            .trace(&point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1), 0)
            .expect("trace");
        assert_eq!(inline.fraction, 0.17937499284744263f64 as f32);
        assert_eq!(inline.end, vec3(0.0, 0.0, 64.125));
        assert_eq!(inline.contents, 1);
        assert_eq!(
            clip.point_contents(vec3(0.0, 0.0, 0.0), SOURCE_BOX_MODEL_HANDLE)
                .expect("contents"),
            BODY
        );
        assert_eq!(clip.point_contents(vec3(0.0, 0.0, 0.0), 0).expect("contents"), 1);
        // The capsule call retargets the retained bounds but leaves the box
        // brush planes intact.
        assert_eq!(
            clip.temp_box_model(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0), true)
                .expect("temp capsule"),
            SOURCE_CAPSULE_MODEL_HANDLE
        );
        assert_eq!(
            clip.model_bounds(SOURCE_BOX_MODEL_HANDLE).expect("bounds"),
            Bounds {
                min: vec3(-4.0, -4.0, -4.0),
                max: vec3(4.0, 4.0, 4.0),
            }
        );
        let intact = clip
            .trace(
                &point_query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), BODY),
                SOURCE_BOX_MODEL_HANDLE,
            )
            .expect("trace");
        assert_eq!(intact.fraction, 0.3387500047683716f64 as f32);
        let checkpoint = clip.capture_temporary_checkpoint();
        clip.temp_box_model(vec3(-2.0, -2.0, -2.0), vec3(2.0, 2.0, 2.0), false)
            .expect("shrink");
        clip.restore_temporary_checkpoint(&checkpoint).expect("restore");
        assert_eq!(
            clip.model_bounds(SOURCE_BOX_MODEL_HANDLE).expect("bounds"),
            Bounds {
                min: vec3(-4.0, -4.0, -4.0),
                max: vec3(4.0, 4.0, 4.0),
            }
        );
        for handle in [SOURCE_CAPSULE_MODEL_HANDLE, 7] {
            let error = clip
                .trace(&point_query(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), 1), handle)
                .expect_err("bad handle must fail");
            assert_eq!(
                error.to_string(),
                format!("CM_ClipHandleToModel: bad handle 1 < {handle} < 256")
            );
        }
        let error = clip
            .trace(&point_query(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), 1), 300)
            .expect_err("handle 300 must fail");
        assert_eq!(error.to_string(), "CM_ClipHandleToModel: bad handle 556");
    }

    #[test]
    fn clip_transformed_traces_match_donor() {
        let world = clip_world();
        let clip = SourceClipModels::new(&world, TemporaryStorage::Private);
        clip.temp_box_model(vec3(-4.0, -4.0, -4.0), vec3(4.0, 4.0, 4.0), false)
            .expect("temp box");
        // Handle 255 drops the rotation, keeping the origin shift.
        let hit = clip
            .transformed_trace(
                &point_query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), BODY),
                SOURCE_BOX_MODEL_HANDLE,
                vec3(0.0, 0.0, 8.0),
                vec3(0.0, 30.0, 0.0),
            )
            .expect("trace");
        assert_eq!(hit.fraction, 0.3787499964237213f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 12.125));
        let rotated = clip
            .transformed_trace(
                &point_query(vec3(100.0, 0.0, 0.0), vec3(-100.0, 0.0, 0.0), 1),
                0,
                vec3(0.0, 0.0, 0.0),
                vec3(0.0, 45.0, 0.0),
            )
            .expect("trace");
        assert_eq!(rotated.fraction, 0.046567775309085846f64 as f32);
        assert_eq!(
            clip.transformed_point_contents(vec3(0.0, 0.0, 0.0), 0, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))
                .expect("contents"),
            1
        );
    }

    #[test]
    fn clip_covers_missing_nodes_and_world_storage() {
        let mut geometry = clip_geometry();
        geometry.nodes.clear();
        let map = decoded_collision_map(&geometry, None).expect("decode");
        let world = CollisionWorld::new(
            Rc::new(map),
            CollisionWorldProfile::Disabled,
            Rc::new(CollisionCounters::new()),
        );
        let clip = SourceClipModels::new(&world, TemporaryStorage::Private);
        assert_eq!(clip.point_contents(vec3(0.0, 0.0, 0.0), 0).expect("contents"), 0);
        assert!(clip.trace_without_nodes(0).expect("empty").is_some());
        assert!(clip
            .trace_without_nodes(SOURCE_BOX_MODEL_HANDLE)
            .expect("empty")
            .is_some());

        let world = clip_world();
        let clip = SourceClipModels::new(&world, TemporaryStorage::Private);
        assert!(clip.trace_without_nodes(0).expect("nodes").is_none());
        assert!(clip.trace_without_nodes(7).expect("nodes").is_none());

        let mut map = decoded_collision_map(&clip_geometry(), None).expect("decode");
        map.box_hull = Some(test_hull());
        let world = CollisionWorld::new(
            Rc::new(map),
            CollisionWorldProfile::Disabled,
            Rc::new(CollisionCounters::new()),
        );
        let clip = SourceClipModels::new(&world, TemporaryStorage::World);
        assert_eq!(
            clip.temp_box_model(vec3(-16.0, -16.0, -16.0), vec3(16.0, 16.0, 16.0), false)
                .expect("temp box"),
            SOURCE_BOX_MODEL_HANDLE
        );
        let hit = clip
            .trace(
                &point_query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), BODY),
                SOURCE_BOX_MODEL_HANDLE,
            )
            .expect("trace");
        assert_eq!(hit.fraction, 0.3387500047683716f64 as f32);
        assert_eq!(
            clip.point_contents(vec3(0.0, 0.0, 0.0), SOURCE_BOX_MODEL_HANDLE)
                .expect("contents"),
            BODY
        );
    }

    /// Box hull mirroring `initialize_box_hull` for storage-path coverage.
    fn test_hull() -> CollisionBoxHull {
        let mut planes = [CollisionPlane {
            normal: vec3(0.0, 0.0, 0.0),
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        }; 12];
        for index in 0..6 {
            for opposite in 0..2 {
                let axis = index >> 1;
                let sign = if opposite == 0 { 1.0 } else { -1.0 };
                planes[index * 2 + opposite] = CollisionPlane {
                    normal: vec3(
                        if axis == 0 { sign } else { 0.0 },
                        if axis == 1 { sign } else { 0.0 },
                        if axis == 2 { sign } else { 0.0 },
                    ),
                    distance: 0.0,
                    plane_type: axis as i32 + opposite as i32 * 3,
                    signbits: if opposite == 0 { 0 } else { 1 << axis },
                };
            }
        }
        let mut sides = [CollisionBrushSide {
            plane: 0,
            shader: 0,
            surface_flags: 0,
        }; 6];
        for (index, side) in sides.iter_mut().enumerate() {
            side.plane = index * 2 + (index & 1);
        }
        CollisionBoxHull {
            bounds: Rc::new(Cell::new(Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            })),
            brush_bounds: Cell::new(Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            }),
            brush_check_count: Cell::new(0),
            first_side: 0,
            sides,
            planes: Cell::new(planes),
        }
    }
}
