//! `setorigin` / `setsize` / `setmodel` / `pointcontents` / `traceline` /
//! `findradius` / `droptofloor` spatial builtins.
//!
//! Ported from donor `src/compat/qc/spatial-host.ts`
//! (`createQcSpatialBindings`).
//!
//! Local mirrors: [`Scene`] mirrors the `trace`/`pointContents` subset of
//! `SceneQueries` from `src/contracts/scene.ts`; [`TraceResult`] and
//! [`TraceHit`] mirror the Q1 trace shapes; [`TraceGlobals`] mirrors the
//! `trace_*` globals. Entity words live in
//! [`FieldTable`](crate::fields::FieldTable); slot/reference conversion
//! reuses `super::entity_host`.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Plane, Vec3};

use super::actor_state::FLAG_ONGROUND;
use super::entity_host::{reference_for_slot, slot_for_reference};
use crate::error::GuestError;
use crate::fields::{FieldTable, FieldValue};

/// Trace shape: point or swept box.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Swept box trace.
    Box {
        /// Box bounds.
        bounds: Bounds,
    },
}

/// Q1 monster interaction mode for a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MoveKind {
    /// Normal trace.
    Normal,
    /// Ignore monsters.
    NoMonsters,
    /// Missile trace.
    Missile,
}

/// Trace query parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceParams {
    /// Trace start.
    pub start: Vec3,
    /// Trace end.
    pub end: Vec3,
    /// Swept shape.
    pub shape: TraceShape,
    /// Monster interaction mode.
    pub move_kind: Q1MoveKind,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
}

/// Trace hit record.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceHit {
    /// No hit.
    None,
    /// Hit the world.
    World {
        /// Brush model index.
        model: u32,
    },
    /// Hit an actor.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Q1 trace result.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceResult {
    /// Fraction of the trace completed.
    pub fraction: f32,
    /// Entire trace inside solid.
    pub all_solid: bool,
    /// Trace started inside solid.
    pub start_solid: bool,
    /// Trace ended in water.
    pub in_water: bool,
    /// Trace ended in open space.
    pub in_open: bool,
    /// Trace end position.
    pub end: Vec3,
    /// Impact plane.
    pub plane: Plane,
    /// Hit record.
    pub hit: TraceHit,
}

/// Scene queries behind the spatial builtins.
pub trait Scene {
    /// Run a Q1 trace.
    fn trace(&self, params: &TraceParams) -> Result<TraceResult, GuestError>;
    /// Read Q1 contents at a point.
    fn point_contents(&self, point: Vec3) -> Result<f32, GuestError>;
}

/// Precached model record used by `setmodel`.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    /// Model index.
    pub index: i32,
    /// Model bounds.
    pub bounds: Bounds,
}

/// Model lookup behind `setmodel`; lookup only, so unprecached models fail.
pub trait ModelTable {
    /// Look up a precached model by name.
    fn model(&self, name: &str) -> Option<ModelInfo>;
}

/// World services behind the spatial builtins.
pub trait SpatialWorld {
    /// Current entity row count.
    fn entity_count(&self) -> usize;
    /// Whether a slot is free.
    fn is_free(&self, slot: usize) -> bool;
    /// Actor bound to a slot.
    fn slot_actor(&self, slot: usize) -> Result<ActorId, GuestError>;
    /// Encode an actor as an entity reference.
    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError>;
    /// Refresh entity snapshots before a radius scan.
    fn prepare_entities(&mut self) {}
    /// Link one slot into the collision world.
    fn link(&mut self, slot: usize);
}

/// `trace_*` global words written by `traceline`.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceGlobals {
    /// `trace_allsolid`.
    pub all_solid: f32,
    /// `trace_startsolid`.
    pub start_solid: f32,
    /// `trace_fraction`.
    pub fraction: f32,
    /// `trace_inwater`.
    pub in_water: f32,
    /// `trace_inopen`.
    pub in_open: f32,
    /// `trace_plane_dist`.
    pub plane_dist: f32,
    /// `trace_endpos`.
    pub end_pos: Vec3,
    /// `trace_plane_normal`.
    pub plane_normal: Vec3,
    /// `trace_ent` entity reference.
    pub ent_reference: i32,
}

impl Default for TraceGlobals {
    fn default() -> Self {
        Self {
            all_solid: 0.0,
            start_solid: 0.0,
            fraction: 1.0,
            in_water: 0.0,
            in_open: 0.0,
            plane_dist: 0.0,
            end_pos: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            plane_normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            ent_reference: 0,
        }
    }
}

/// Spatial builtins bound to one world, scene, and model table.
pub struct SpatialBindings<W, S, M> {
    world: W,
    scene: S,
    models: M,
}

impl<W: SpatialWorld, S: Scene, M: ModelTable> SpatialBindings<W, S, M> {
    /// Bind the builtins to their services.
    pub fn new(world: W, scene: S, models: M) -> Self {
        Self { world, scene, models }
    }

    /// Borrow the world (for test inspection).
    #[must_use]
    pub fn world(&self) -> &W {
        &self.world
    }

    /// `setorigin`: move an entity and relink it.
    pub fn set_origin(&mut self, fields: &mut FieldTable, reference: i32, origin: Vec3) -> Result<(), GuestError> {
        let slot = slot_for_reference(reference, self.world.entity_count())?;
        let actor = self.world.slot_actor(slot)?;
        fields.set(&actor, "origin", FieldValue::Vector(origin))?;
        self.world.link(slot);
        Ok(())
    }

    /// `setsize`: resize an entity, derive `size`, and relink it.
    pub fn set_size(&mut self, fields: &mut FieldTable, reference: i32, bounds: Bounds) -> Result<(), GuestError> {
        if bounds.min.x > bounds.max.x || bounds.min.y > bounds.max.y || bounds.min.z > bounds.max.z {
            return Err(GuestError::invalid("backwards mins/maxs"));
        }
        let slot = slot_for_reference(reference, self.world.entity_count())?;
        let actor = self.world.slot_actor(slot)?;
        fields.set(&actor, "mins", FieldValue::Vector(bounds.min))?;
        fields.set(&actor, "maxs", FieldValue::Vector(bounds.max))?;
        fields.set(
            &actor,
            "size",
            FieldValue::Vector(Vec3 {
                x: bounds.max.x - bounds.min.x,
                y: bounds.max.y - bounds.min.y,
                z: bounds.max.z - bounds.min.z,
            }),
        )?;
        self.world.link(slot);
        Ok(())
    }

    /// `setmodel`: attach a precached model and adopt its bounds.
    pub fn set_model(&mut self, fields: &mut FieldTable, reference: i32, name: &str) -> Result<(), GuestError> {
        let Some(model) = self.models.model(name) else {
            return Err(GuestError::invalid(format!("no precache: {name}")));
        };
        let slot = slot_for_reference(reference, self.world.entity_count())?;
        let actor = self.world.slot_actor(slot)?;
        fields.set(&actor, "model", FieldValue::Text(name.to_string()))?;
        fields.set(&actor, "modelindex", FieldValue::Float(model.index as f32))?;
        self.set_size(fields, reference_for_slot(slot), model.bounds)?;
        Ok(())
    }

    /// `pointcontents`: read Q1 contents at a point.
    pub fn point_contents(&self, point: Vec3) -> Result<f32, GuestError> {
        self.scene.point_contents(point)
    }

    /// `traceline`: trace a line and write the `trace_*` globals.
    pub fn traceline(
        &mut self,
        globals: &mut TraceGlobals,
        start: Vec3,
        end: Vec3,
        mode: i32,
        pass_reference: i32,
    ) -> Result<(), GuestError> {
        let move_kind = match mode {
            1 => Q1MoveKind::NoMonsters,
            2 => Q1MoveKind::Missile,
            _ => Q1MoveKind::Normal,
        };
        let pass_actor = if pass_reference == 0 {
            None
        } else {
            let slot = slot_for_reference(pass_reference, self.world.entity_count())?;
            Some(self.world.slot_actor(slot)?)
        };
        let trace = self.scene.trace(&TraceParams {
            start,
            end,
            shape: TraceShape::Point,
            move_kind,
            pass_actor,
        })?;
        self.write_trace(globals, &trace)
    }

    /// `findradius`: chain solid entities whose center is within `radius`
    /// of `center`; returns the head reference.
    pub fn find_radius(&mut self, fields: &mut FieldTable, center: Vec3, radius: f32) -> Result<i32, GuestError> {
        self.world.prepare_entities();
        let mut chain = 0;
        for slot in 1..self.world.entity_count() {
            if self.world.is_free(slot) {
                continue;
            }
            let reference = reference_for_slot(slot);
            let actor = self.world.slot_actor(slot)?;
            if fields.get(&actor, "solid")?.as_float("solid")? == 0.0 {
                continue;
            }
            let origin = fields.get(&actor, "origin")?.as_vector("origin")?;
            let min = fields.get(&actor, "mins")?.as_vector("mins")?;
            let max = fields.get(&actor, "maxs")?.as_vector("maxs")?;
            let dx = center.x - (origin.x + (min.x + max.x) * 0.5);
            let dy = center.y - (origin.y + (min.y + max.y) * 0.5);
            let dz = center.z - (origin.z + (min.z + max.z) * 0.5);
            if dx.hypot(dy).hypot(dz) > radius {
                continue;
            }
            fields.set(&actor, "chain", FieldValue::Float(chain as f32))?;
            chain = reference;
        }
        Ok(chain)
    }

    /// `droptofloor`: drop `self_reference` to the floor; returns 1 on
    /// landing, 0 when the trace runs free or starts solid.
    pub fn drop_to_floor(&mut self, fields: &mut FieldTable, self_reference: i32) -> Result<f32, GuestError> {
        let slot = slot_for_reference(self_reference, self.world.entity_count())?;
        let actor = self.world.slot_actor(slot)?;
        let origin = fields.get(&actor, "origin")?.as_vector("origin")?;
        let trace = self.scene.trace(&TraceParams {
            start: origin,
            end: Vec3 { x: origin.x, y: origin.y, z: origin.z - 256.0 },
            shape: TraceShape::Box {
                bounds: Bounds {
                    min: fields.get(&actor, "mins")?.as_vector("mins")?,
                    max: fields.get(&actor, "maxs")?.as_vector("maxs")?,
                },
            },
            move_kind: Q1MoveKind::Normal,
            pass_actor: Some(actor.clone()),
        })?;
        if trace.fraction == 1.0 || trace.all_solid {
            return Ok(0.0);
        }
        fields.set(&actor, "origin", FieldValue::Vector(trace.end))?;
        self.world.link(slot);
        let flags = fields.get(&actor, "flags")?.as_float("flags")? as i32;
        fields.set(&actor, "flags", FieldValue::Float((flags | FLAG_ONGROUND) as f32))?;
        let hit = self.hit_reference(&trace.hit)?;
        fields.set(&actor, "groundentity", FieldValue::Float(hit as f32))?;
        Ok(1.0)
    }

    fn hit_reference(&self, hit: &TraceHit) -> Result<i32, GuestError> {
        match hit {
            TraceHit::Actor { actor } => self.world.reference(actor),
            TraceHit::None | TraceHit::World { .. } => Ok(0),
        }
    }

    fn write_trace(&self, globals: &mut TraceGlobals, trace: &TraceResult) -> Result<(), GuestError> {
        globals.all_solid = f32::from(trace.all_solid);
        globals.start_solid = f32::from(trace.start_solid);
        globals.fraction = trace.fraction;
        globals.in_water = f32::from(trace.in_water);
        globals.in_open = f32::from(trace.in_open);
        globals.plane_dist = trace.plane.distance;
        globals.end_pos = trace.end;
        globals.plane_normal = trace.plane.normal;
        globals.ent_reference = self.hit_reference(&trace.hit)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::FieldLayout;
    use qa_core::identity::{IdentityOwner, SavedActorId};
    use qa_core::math::vec3;
    use std::collections::HashMap;

    struct FakeWorld {
        actors: Vec<Option<ActorId>>,
        links: Vec<usize>,
        prepared: usize,
    }

    impl SpatialWorld for FakeWorld {
        fn entity_count(&self) -> usize {
            self.actors.len()
        }

        fn is_free(&self, slot: usize) -> bool {
            self.actors.get(slot).is_none_or(Option::is_none)
        }

        fn slot_actor(&self, slot: usize) -> Result<ActorId, GuestError> {
            self.actors
                .get(slot)
                .and_then(Clone::clone)
                .ok_or_else(|| GuestError::invalid(format!("free slot {slot}")))
        }

        fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
            self.actors
                .iter()
                .position(|entry| entry.as_ref() == Some(actor))
                .map(|slot| slot as i32)
                .ok_or_else(|| GuestError::invalid("stale actor"))
        }

        fn prepare_entities(&mut self) {
            self.prepared += 1;
        }

        fn link(&mut self, slot: usize) {
            self.links.push(slot);
        }
    }

    struct FakeScene {
        contents: f32,
        hit: Option<ActorId>,
    }

    impl Scene for FakeScene {
        fn trace(&self, params: &TraceParams) -> Result<TraceResult, GuestError> {
            let hit = match &self.hit {
                Some(actor) if Some(actor) != params.pass_actor.as_ref() => {
                    TraceHit::Actor { actor: actor.clone() }
                }
                _ => TraceHit::None,
            };
            let fraction = if matches!(hit, TraceHit::None) { 1.0 } else { 0.5 };
            Ok(TraceResult {
                fraction,
                all_solid: false,
                start_solid: false,
                in_water: false,
                in_open: true,
                end: params.end,
                plane: Plane { normal: vec3(0.0, 0.0, 1.0), distance: params.end.z },
                hit,
            })
        }

        fn point_contents(&self, _point: Vec3) -> Result<f32, GuestError> {
            Ok(self.contents)
        }
    }

    struct FakeModels {
        table: HashMap<String, ModelInfo>,
    }

    impl ModelTable for FakeModels {
        fn model(&self, name: &str) -> Option<ModelInfo> {
            self.table.get(name).cloned()
        }
    }

    fn layout() -> FieldLayout {
        FieldLayout::qc_entity()
            .field("flags", "float")
            .field("mins", "vector")
            .field("maxs", "vector")
            .field("size", "vector")
            .field("chain", "float")
            .field("groundentity", "float")
    }

    fn harness(hit: Option<ActorId>) -> (IdentityOwner, FieldTable, FakeWorld, FakeScene, FakeModels) {
        let owner = IdentityOwner::create("spatial-host").unwrap();
        let mut fields = FieldTable::new();
        let world_actor = owner.actor(0, 1);
        let player = owner.actor(1, 1);
        let target = owner.actor(2, 1);
        for actor in [&world_actor, &player, &target] {
            fields.allocate(actor, &layout()).unwrap();
        }
        fields.set(&player, "solid", FieldValue::Float(3.0)).unwrap();
        fields.set(&player, "origin", FieldValue::Vector(vec3(0.0, 0.0, 0.0))).unwrap();
        fields.set(&target, "solid", FieldValue::Float(3.0)).unwrap();
        fields.set(&target, "origin", FieldValue::Vector(vec3(40.0, 0.0, 0.0))).unwrap();
        let world = FakeWorld { actors: vec![Some(world_actor), Some(player), Some(target)], links: Vec::new(), prepared: 0 };
        let scene = FakeScene { contents: -16.0, hit };
        let mut table = HashMap::new();
        table.insert(
            "progs/player.mdl".to_string(),
            ModelInfo {
                index: 2,
                bounds: Bounds { min: vec3(-16.0, -16.0, -24.0), max: vec3(16.0, 16.0, 32.0) },
            },
        );
        (owner, fields, world, scene, FakeModels { table })
    }

    #[test]
    fn set_origin_size_and_model_link() {
        let (_owner, mut fields, world, scene, models) = harness(None);
        let mut bindings = SpatialBindings::new(world, scene, models);
        bindings.set_origin(&mut fields, 1, vec3(8.0, 0.0, 0.0)).unwrap();
        bindings
            .set_size(
                &mut fields,
                1,
                Bounds { min: vec3(-8.0, -8.0, -8.0), max: vec3(8.0, 8.0, 8.0) },
            )
            .unwrap();
        bindings.set_model(&mut fields, 1, "progs/player.mdl").unwrap();
        let actor = bindings.world().slot_actor(1).unwrap();
        assert_eq!(fields.get(&actor, "modelindex").unwrap(), &FieldValue::Float(2.0));
        assert_eq!(
            fields.get(&actor, "size").unwrap(),
            &FieldValue::Vector(vec3(32.0, 32.0, 56.0))
        );
        assert_eq!(bindings.world().links, vec![1, 1, 1]);
        assert!(bindings.set_model(&mut fields, 1, "missing.mdl").is_err());
        assert!(bindings
            .set_size(&mut fields, 1, Bounds { min: vec3(1.0, 0.0, 0.0), max: vec3(0.0, 0.0, 0.0) })
            .is_err());
    }

    #[test]
    fn traceline_writes_globals_with_hit_reference() {
        let (owner, mut fields, world, _, models) = harness(Some(owner.actor(2, 1)));
        let hit = owner.actor(2, 1);
        let _ = &mut fields;
        let scene = FakeScene { contents: 0.0, hit: Some(hit) };
        let mut bindings = SpatialBindings::new(world, scene, models);
        let mut globals = TraceGlobals::default();
        bindings.traceline(&mut globals, vec3(0.0, 0.0, 0.0), vec3(64.0, 0.0, 0.0), 1, 1).unwrap();
        assert_eq!(globals.fraction, 0.5);
        assert_eq!(globals.ent_reference, 2);
        assert_eq!(globals.plane_normal, vec3(0.0, 0.0, 1.0));
        assert_eq!(SavedActorId::from(&bindings.world().slot_actor(2).unwrap()).slot, 2);
    }

    #[test]
    fn find_radius_chains_solids_in_range() {
        let (_owner, mut fields, world, scene, models) = harness(None);
        let mut bindings = SpatialBindings::new(world, scene, models);
        let head = bindings.find_radius(&mut fields, vec3(0.0, 0.0, 0.0), 24.0).unwrap();
        assert_eq!(head, 1);
        assert_eq!(bindings.world().prepared, 1);
        let near = bindings.world().slot_actor(1).unwrap();
        let far = bindings.world().slot_actor(2).unwrap();
        assert_eq!(fields.get(&near, "chain").unwrap(), &FieldValue::Float(0.0));
        assert_eq!(fields.get(&far, "chain").unwrap(), &FieldValue::Float(0.0));
    }

    #[test]
    fn point_contents_queries_scene() {
        let (_owner, _fields, world, scene, models) = harness(None);
        let bindings = SpatialBindings::new(world, scene, models);
        assert_eq!(bindings.point_contents(vec3(0.0, 0.0, 0.0)).unwrap(), -16.0);
    }

    #[test]
    fn drop_to_floor_lands_and_sets_ground() {
        let (owner, mut fields, world, _, models) = harness(None);
        let floor = owner.actor(0, 1);
        let scene = FakeScene { contents: 0.0, hit: Some(floor) };
        let mut bindings = SpatialBindings::new(world, scene, models);
        let actor = bindings.world().slot_actor(1).unwrap();
        fields.set(&actor, "origin", FieldValue::Vector(vec3(0.0, 0.0, 64.0))).unwrap();
        let landed = bindings.drop_to_floor(&mut fields, 1).unwrap();
        assert_eq!(landed, 1.0);
        let flags = fields.get(&actor, "flags").unwrap().as_float("flags").unwrap() as i32;
        assert_ne!(flags & FLAG_ONGROUND, 0);
        assert_eq!(fields.get(&actor, "groundentity").unwrap(), &FieldValue::Float(0.0));
        assert_eq!(bindings.world().links, vec![1]);
    }

    #[test]
    fn drop_to_floor_misses_when_trace_runs_free() {
        let (_owner, mut fields, world, scene, models) = harness(None);
        let mut bindings = SpatialBindings::new(world, scene, models);
        assert_eq!(bindings.drop_to_floor(&mut fields, 1).unwrap(), 0.0);
        assert!(bindings.world().links.is_empty());
    }
}
