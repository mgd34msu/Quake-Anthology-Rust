//! Q3 guest spatial operations over shared collision.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/guest-spatial.ts`
//! (`Q3GuestSpatial`).
//!
//! SV_Trace port credited to id Software's sv_world.c. Copyright (C)
//! 1999-2005 Id Software, Inc. GPL-2.0-or-later.
//!
//! Temporary box/capsule clip models (donor `createBoxModel`/
//! `createCapsuleModel` from `src/world/collision/q3/model.ts`) have no Rust
//! home, so callers inject them through [`Q3GuestClipFactory`]; the donor's
//! native-world performance counters are dropped because no Rust equivalent
//! exists.

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::q3::base::world::{ActorTraceHit, TraceContact, TraceSolidity};
use qa_core::cvar::CvarRegistry;
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::native_atoi;
use qa_guest::qvm::client_collision_syscalls::{TraceRecord, TraceShape as GuestTraceShape};
use qa_guest::qvm::server_game_syscalls::{Bounds as GuestBounds, ServerSpatialHost, ServerTraceQuery};
use qa_guest::qvm::shared_entity_record::QvmEntityCollisionModel;
use qa_world::movement::types::TraceShape;

use super::guest_records::{
    Q3GuestModelTraceQuery, Q3GuestPointTarget, Q3GuestRecords, Q3GuestScene, Q3GuestTraceQuery, Q3GuestVisibilityLink,
};
use super::guest_world::Q3GuestWorld;

/// World model index (donor `0`).
const WORLD_MODEL: i32 = 0;

/// Temporary clip-model seam: donor `createBoxModel`/`createCapsuleModel`
/// products from `src/world/collision/q3/model.ts` (missing value, injected
/// by the caller; never duplicated here).
pub trait Q3GuestClipModel {
    /// Contents at a transformed point.
    fn transformed_point_contents(&self, point: Vec3, origin: Vec3, angles: Vec3) -> i32;
    /// Whether a transformed sweep starts or ends solid.
    fn transformed_trace_solid(
        &self,
        start: Vec3,
        end: Vec3,
        mask: i32,
        shape: TraceShape,
        origin: Vec3,
        angles: Vec3,
    ) -> bool;
}

/// Temporary clip-model kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GuestClipKind {
    /// Box model.
    Box,
    /// Capsule model.
    Capsule,
}

/// Temporary clip-model factory seam (see [`Q3GuestClipModel`]).
pub type Q3GuestClipFactory = Rc<dyn Fn(Q3GuestClipKind, &Bounds) -> Rc<dyn Q3GuestClipModel>>;

fn plane_type(normal: Vec3) -> (u8, u8) {
    let plane_type = if normal.x == 1.0 {
        0
    } else if normal.y == 1.0 {
        1
    } else if normal.z == 1.0 {
        2
    } else {
        3
    };
    let signbits = u8::from(normal.x < 0.0) | (u8::from(normal.y < 0.0) << 1) | (u8::from(normal.z < 0.0) << 2);
    (plane_type, signbits)
}

/// Guest spatial operations over the record-owner scene.
pub struct Q3GuestSpatial {
    /// Guest records.
    pub records: Rc<Q3GuestRecords>,
    /// Shared scene (must be the record-owner scene).
    pub scene: Rc<dyn Q3GuestScene>,
    /// Cvar registry.
    pub cvars: Rc<RefCell<CvarRegistry>>,
    /// Area-portal ownership.
    pub world: Q3GuestWorld<dyn Q3GuestScene>,
    clip: Q3GuestClipFactory,
}

impl Q3GuestSpatial {
    /// Bind records, scene, cvars, and the clip-model factory.
    #[must_use]
    pub fn new(
        records: Rc<Q3GuestRecords>,
        scene: Rc<dyn Q3GuestScene>,
        cvars: Rc<RefCell<CvarRegistry>>,
        clip: Q3GuestClipFactory,
    ) -> Self {
        if !Rc::ptr_eq(&records.host.scene, &scene) {
            panic!("Q3 guest spatial operations must use the record owner scene");
        }
        Self {
            records,
            scene: scene.clone(),
            cvars,
            world: Q3GuestWorld::new(scene),
            clip,
        }
    }

    fn temporary(&self, capsule: bool, bounds: &Bounds) -> Rc<dyn Q3GuestClipModel> {
        (self.clip)(
            if capsule {
                Q3GuestClipKind::Capsule
            } else {
                Q3GuestClipKind::Box
            },
            bounds,
        )
    }

    fn curve_policy(&self) -> (bool, bool) {
        let cvars = self.cvars.borrow();
        let curves = cvars.variable_value("cm_noCurves") == 0.0;
        let player_curve_clip = cvars
            .get("cm_playerCurveClip")
            .map_or(1, |variable| variable.integer_value)
            != 0;
        (curves, player_curve_clip)
    }

    /// Visibility link for a VM slot, if linked.
    #[must_use]
    pub fn visibility(&self, slot: i32) -> Option<Q3GuestVisibilityLink> {
        self.records.visibility(slot)
    }
}

impl ServerSpatialHost for Q3GuestSpatial {
    fn trace(&mut self, query: &ServerTraceQuery) -> TraceRecord {
        let shape = if query.mins == vec3(0.0, 0.0, 0.0) && query.maxs == vec3(0.0, 0.0, 0.0) {
            TraceShape::Point
        } else {
            let bounds = Bounds {
                min: query.mins,
                max: query.maxs,
            };
            match query.shape {
                GuestTraceShape::Capsule => TraceShape::Capsule(bounds),
                GuestTraceShape::Box => TraceShape::Box(bounds),
            }
        };
        let pass_entity = query.pass_entity_num;
        let (curves, player_curve_clip) = self.curve_policy();
        let result = self.scene.trace(&Q3GuestTraceQuery {
            start: query.start,
            end: query.end,
            shape,
            pass_actor: self.records.reference(pass_entity),
            mask: query.mask,
            curves,
            player_curve_clip,
        });
        let entity_num = match &result.hit {
            ActorTraceHit::Actor { actor } => self.records.require_slot(actor),
            _ if result.fraction == 1.0 => 1023,
            _ => 1022,
        };
        let (plane_normal, plane_distance) = match &result.contact {
            TraceContact::Plane { plane } => (plane.normal, plane.distance),
            TraceContact::None => (vec3(0.0, 0.0, 0.0), 0.0),
        };
        let (plane_type, plane_signbits) = plane_type(plane_normal);
        TraceRecord {
            all_solid: result.solidity == TraceSolidity::AllSolid,
            start_solid: matches!(result.solidity, TraceSolidity::StartSolid | TraceSolidity::AllSolid),
            fraction: result.fraction,
            end: result.end,
            plane_normal,
            plane_distance,
            plane_type,
            plane_signbits,
            surface_flags: result.surface_flags,
            contents: result.contents,
            entity_num,
        }
    }

    fn point_contents(&mut self, point: Vec3, pass_entity_num: i32) -> i32 {
        let zero = vec3(0.0, 0.0, 0.0);
        let mut contents = self.scene.point_contents(
            point,
            &Q3GuestPointTarget::Model {
                model: WORLD_MODEL,
                origin: zero,
                angles: zero,
            },
            None,
        );
        let actors = self.scene.query_actors(&Bounds { min: point, max: point });
        for actor in &actors {
            let slot = self.records.require_slot(actor);
            if slot == pass_entity_num {
                continue;
            }
            let entity = self.records.entity(slot);
            match entity.r.model {
                QvmEntityCollisionModel::Inline { index } => {
                    contents |= self.scene.point_contents(
                        point,
                        &Q3GuestPointTarget::Model {
                            model: index,
                            origin: entity.r.current_origin,
                            angles: entity.r.current_angles,
                        },
                        None,
                    );
                }
                model => {
                    let bounds = Bounds {
                        min: entity.r.mins,
                        max: entity.r.maxs,
                    };
                    let temporary = self.temporary(matches!(model, QvmEntityCollisionModel::Capsule), &bounds);
                    contents |=
                        temporary.transformed_point_contents(point, entity.r.current_origin, entity.r.current_angles);
                }
            }
        }
        contents
    }

    fn area_entities(&mut self, bounds: GuestBounds, maximum: i32) -> Vec<i32> {
        let bounds = Bounds {
            min: bounds.min,
            max: bounds.max,
        };
        let candidates = self.scene.query_actors(&bounds);
        let candidates = if maximum < 0 {
            candidates
        } else {
            candidates.into_iter().take(maximum as usize).collect()
        };
        candidates
            .iter()
            .map(|actor| self.records.require_slot(actor))
            .collect()
    }

    fn entity_contact(&mut self, bounds: GuestBounds, slot: i32, capsule: bool) -> bool {
        let entity = self.records.entity(slot);
        let shared = &entity.r;
        let bounds = Bounds {
            min: bounds.min,
            max: bounds.max,
        };
        if let QvmEntityCollisionModel::Inline { index } = shared.model {
            let (curves, player_curve_clip) = self.curve_policy();
            let result = self.scene.geometry_trace(&Q3GuestModelTraceQuery {
                start: vec3(0.0, 0.0, 0.0),
                end: vec3(0.0, 0.0, 0.0),
                shape: if capsule {
                    TraceShape::Capsule(bounds)
                } else {
                    TraceShape::Box(bounds)
                },
                pass_actor: self.records.reference(1023),
                model: index,
                origin: shared.current_origin,
                angles: shared.current_angles,
                mask: -1,
                curves,
                player_curve_clip,
            });
            return matches!(result.solidity, TraceSolidity::StartSolid | TraceSolidity::AllSolid);
        }
        let target_bounds = Bounds {
            min: shared.mins,
            max: shared.maxs,
        };
        let temporary = self.temporary(matches!(shared.model, QvmEntityCollisionModel::Capsule), &target_bounds);
        let origin = vec3(0.0, 0.0, 0.0);
        temporary.transformed_trace_solid(
            origin,
            origin,
            -1,
            if capsule {
                TraceShape::Capsule(bounds)
            } else {
                TraceShape::Box(bounds)
            },
            shared.current_origin,
            shared.current_angles,
        )
    }

    fn set_brush_model(&mut self, slot: i32, name: &str) {
        if !name.starts_with('*') {
            panic!("SV_SetBrushModel: {name} is not a brush model");
        }
        let index = native_atoi(&name[1..]).unwrap_or_else(|_| panic!("SV_SetBrushModel: {name} is not a brush model"));
        let bounds = self.scene.model_bounds(index);
        let mut entity = self.records.entity(slot);
        entity.r.model = QvmEntityCollisionModel::Inline { index };
        entity.r.mins = bounds.min;
        entity.r.maxs = bounds.max;
        entity.r.contents = -1;
        self.records.write_entity(slot, &entity);
        self.records.link(slot);
    }

    fn adjust_area_portal_state(&mut self, slot: i32, open: bool) {
        let link = self.records.visibility(slot);
        if link.is_none() {
            self.world.adjust_area_portal_state(0, 0, open);
            return;
        }
        let link = link.expect("checked link");
        if link.areanum2 != -1 {
            self.world.adjust_area_portal_state(link.areanum, link.areanum2, open);
        }
    }

    fn in_pvs(&mut self, first: Vec3, second: Vec3, ignore_portals: bool) -> bool {
        let first_leaf = self.scene.point_leaf(first);
        let second_leaf = self.scene.point_leaf(second);
        self.scene.cluster_visible(
            self.scene.leaf_cluster(first_leaf),
            self.scene.leaf_cluster(second_leaf),
        ) && (ignore_portals
            || self
                .scene
                .areas_connected(self.scene.leaf_area(first_leaf), self.scene.leaf_area(second_leaf)))
    }

    fn areas_connected(&mut self, first: i32, second: i32) -> bool {
        self.scene.areas_connected(first, second)
    }

    fn link(&mut self, slot: i32) {
        self.records.link(slot);
    }

    fn unlink(&mut self, slot: i32) {
        self.records.unlink(slot);
    }
}

#[cfg(test)]
mod tests {
    use qa_content::q3::base::world::{ActorTraceResult, TraceSolidity};
    use qa_core::cmd::Dialect;
    use qa_core::identity::ActorId;
    use qa_core::identity::{IdentityOwner, OwnedActor, ProviderId, SavedActorId, SessionId};
    use qa_guest::qvm::client_collision_syscalls::TraceShape as GuestTraceShape;
    use qa_guest::qvm::game_data::{AbiProfile, QvmSharedMemory};

    use super::super::guest_records::{
        Q3GuestActorCollision, Q3GuestActorSource, Q3GuestBodyBinding, Q3GuestLeafQuery, Q3GuestRecordActors,
        Q3GuestRecordBodies, Q3GuestRecordHost,
    };
    use super::super::guest_world::{Q3GuestGeometry, Q3GuestGeometryKind, Q3GuestNativeClipWorld, Q3GuestTopology};
    use super::*;

    struct FakeActors {
        owner: IdentityOwner,
    }

    impl Q3GuestRecordActors for FakeActors {
        fn assert_owned(&self, _actor: &OwnedActor) {}

        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            self.owner
                .owned_actor(&self.owner.actor(slot as u32, 1), provider.clone())
                .unwrap()
        }

        fn source_of(&self, _actor: &ActorId) -> Option<Q3GuestActorSource> {
            None
        }

        fn at_source(&self, _provider: &ProviderId, _slot: usize) -> Option<OwnedActor> {
            None
        }

        fn owned_by(&self, _provider: &ProviderId) -> Vec<OwnedActor> {
            Vec::new()
        }

        fn resolve_saved(&self, _saved: &SavedActorId) -> Option<OwnedActor> {
            None
        }

        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }

        fn release(&self, _actor: &OwnedActor) {}

        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
    }

    struct FakeBodies;
    impl Q3GuestRecordBodies for FakeBodies {
        fn bind(&self, _actor: &OwnedActor, _binding: Q3GuestBodyBinding) {}

        fn unlink(&self, _actor: &OwnedActor) {}

        fn link(&self, _actor: &OwnedActor) {}
    }

    struct FakeScene {
        contents: i32,
        connected: bool,
    }

    impl Q3GuestTopology for FakeScene {
        fn geometry(&self) -> Q3GuestGeometry {
            Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q3Bsp,
                areas: Vec::new(),
                area_portals: Vec::new(),
                leaf_count: 1,
            }
        }

        fn adjust_area_portal_state(&self, _first: i32, _second: i32, _open: bool) {}

        fn adjust_area_portal_contribution(&self, _portal: i32, _delta: i32) {}

        fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>> {
            None
        }
    }

    impl Q3GuestScene for FakeScene {
        fn bind_actor_collision(&self, _read: Rc<dyn Fn(&ActorId) -> Option<Q3GuestActorCollision>>) {}

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> Q3GuestLeafQuery {
            Q3GuestLeafQuery {
                leaves: Vec::new(),
                topnode: None,
                overflow: false,
            }
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }

        fn trace(&self, _query: &Q3GuestTraceQuery) -> ActorTraceResult {
            ActorTraceResult {
                fraction: 1.0,
                end: vec3(1.0, 2.0, 3.0),
                hit: ActorTraceHit::None,
                contact: TraceContact::None,
                solidity: TraceSolidity::Clear,
                contents: 0,
                surface_flags: 0,
            }
        }

        fn point_contents(&self, _point: Vec3, _target: &Q3GuestPointTarget, _pass_actor: Option<&ActorId>) -> i32 {
            self.contents
        }

        fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
            Vec::new()
        }

        fn geometry_trace(&self, _query: &Q3GuestModelTraceQuery) -> ActorTraceResult {
            ActorTraceResult {
                fraction: 0.0,
                end: vec3(0.0, 0.0, 0.0),
                hit: ActorTraceHit::World,
                contact: TraceContact::None,
                solidity: TraceSolidity::StartSolid,
                contents: 1,
                surface_flags: 0,
            }
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            }
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            self.connected
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }

        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            true
        }
    }

    struct FakeClip;
    impl Q3GuestClipModel for FakeClip {
        fn transformed_point_contents(&self, _point: Vec3, _origin: Vec3, _angles: Vec3) -> i32 {
            0
        }

        fn transformed_trace_solid(
            &self,
            _start: Vec3,
            _end: Vec3,
            _mask: i32,
            _shape: TraceShape,
            _origin: Vec3,
            _angles: Vec3,
        ) -> bool {
            false
        }
    }

    fn spatial_with(scene: Rc<dyn Q3GuestScene>) -> Q3GuestSpatial {
        let memory = QvmSharedMemory::new(65536).unwrap();
        let data = qa_guest::qvm::game_data::QvmGameData::new(memory, AbiProfile::Modern);
        data.set_client_count(2).unwrap();
        data.locate(64, 8, 560, 8192, 560).unwrap();
        let records = Q3GuestRecords::open(
            data,
            Q3GuestRecordHost {
                actors: Rc::new(FakeActors {
                    owner: IdentityOwner::create("q3-guest-spatial").unwrap(),
                }),
                bodies: Rc::new(FakeBodies),
                scene: scene.clone(),
                provider: ProviderId::new("q3", "guest-test"),
                collision: Rc::new(|_, _| {}),
                admit: None,
            },
        );
        let cvars = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        cvars.borrow_mut().register("cm_noCurves", "0", 0).unwrap();
        cvars.borrow_mut().register("cm_playerCurveClip", "1", 0).unwrap();
        Q3GuestSpatial::new(records, scene, cvars, Rc::new(|_, _| Rc::new(FakeClip)))
    }

    fn spatial() -> Q3GuestSpatial {
        spatial_with(Rc::new(FakeScene {
            contents: 5,
            connected: true,
        }))
    }

    #[test]
    fn trace_maps_miss_and_plane() {
        let mut spatial = spatial();
        let record = ServerSpatialHost::trace(
            &mut spatial,
            &ServerTraceQuery {
                start: vec3(0.0, 0.0, 0.0),
                end: vec3(1.0, 2.0, 3.0),
                mins: vec3(0.0, 0.0, 0.0),
                maxs: vec3(0.0, 0.0, 0.0),
                shape: GuestTraceShape::Box,
                pass_entity_num: 1023,
                mask: -1,
            },
        );
        assert_eq!(record.fraction, 1.0);
        assert_eq!(record.entity_num, 1023);
        assert!(!record.all_solid && !record.start_solid);
        assert_eq!((record.plane_type, record.plane_signbits), (3, 0));
    }

    #[test]
    fn capsule_shape_selects_capsule() {
        struct ShapeScene {
            seen: Rc<RefCell<Vec<TraceShape>>>,
        }
        impl Q3GuestTopology for ShapeScene {
            fn geometry(&self) -> Q3GuestGeometry {
                Q3GuestGeometry {
                    kind: Q3GuestGeometryKind::Q3Bsp,
                    areas: Vec::new(),
                    area_portals: Vec::new(),
                    leaf_count: 1,
                }
            }

            fn adjust_area_portal_state(&self, _first: i32, _second: i32, _open: bool) {}

            fn adjust_area_portal_contribution(&self, _portal: i32, _delta: i32) {}

            fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>> {
                None
            }
        }
        impl Q3GuestScene for ShapeScene {
            fn bind_actor_collision(&self, _read: Rc<dyn Fn(&ActorId) -> Option<Q3GuestActorCollision>>) {}

            fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> Q3GuestLeafQuery {
                Q3GuestLeafQuery {
                    leaves: Vec::new(),
                    topnode: None,
                    overflow: false,
                }
            }

            fn leaf_cluster(&self, _leaf: i32) -> i32 {
                0
            }

            fn leaf_area(&self, _leaf: i32) -> i32 {
                0
            }

            fn trace(&self, query: &Q3GuestTraceQuery) -> ActorTraceResult {
                self.seen.borrow_mut().push(query.shape);
                FakeScene {
                    contents: 0,
                    connected: true,
                }
                .trace(query)
            }

            fn point_contents(&self, _point: Vec3, _target: &Q3GuestPointTarget, _pass_actor: Option<&ActorId>) -> i32 {
                0
            }

            fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
                Vec::new()
            }

            fn geometry_trace(&self, query: &Q3GuestModelTraceQuery) -> ActorTraceResult {
                FakeScene {
                    contents: 0,
                    connected: true,
                }
                .geometry_trace(query)
            }

            fn model_bounds(&self, _model: i32) -> Bounds {
                Bounds {
                    min: vec3(-1.0, -1.0, -1.0),
                    max: vec3(1.0, 1.0, 1.0),
                }
            }

            fn areas_connected(&self, _first: i32, _second: i32) -> bool {
                true
            }

            fn point_leaf(&self, _point: Vec3) -> i32 {
                0
            }

            fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
                true
            }
        }
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut spatial = spatial_with(Rc::new(ShapeScene { seen: seen.clone() }));
        ServerSpatialHost::trace(
            &mut spatial,
            &ServerTraceQuery {
                start: vec3(0.0, 0.0, 0.0),
                end: vec3(0.0, 0.0, 0.0),
                mins: vec3(-15.0, -15.0, -15.0),
                maxs: vec3(15.0, 15.0, 15.0),
                shape: GuestTraceShape::Capsule,
                pass_entity_num: 1023,
                mask: -1,
            },
        );
        assert!(matches!(seen.borrow().as_slice(), [TraceShape::Capsule(_)]));
    }

    #[test]
    fn point_contents_accumulates_world() {
        let mut spatial = spatial();
        assert_eq!(
            ServerSpatialHost::point_contents(&mut spatial, vec3(0.0, 0.0, 0.0), 1023),
            5
        );
    }

    #[test]
    fn area_entities_and_contact() {
        let mut spatial = spatial();
        let bounds = GuestBounds {
            min: vec3(-1.0, -1.0, -1.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        assert!(ServerSpatialHost::area_entities(&mut spatial, bounds, -1).is_empty());
        let _ = spatial.records.actor(0);
        assert!(!ServerSpatialHost::entity_contact(&mut spatial, bounds, 0, false));
    }

    #[test]
    fn brush_model_and_pvs() {
        let mut spatial = spatial();
        let _ = spatial.records.actor(0);
        ServerSpatialHost::set_brush_model(&mut spatial, 0, "*3");
        let entity = spatial.records.entity(0);
        assert!(matches!(entity.r.model, QvmEntityCollisionModel::Inline { index: 3 }));
        assert_eq!(entity.r.contents, -1);
        assert!(ServerSpatialHost::in_pvs(
            &mut spatial,
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 1.0, 1.0),
            false
        ));
        assert!(ServerSpatialHost::areas_connected(&mut spatial, 0, 1));
        ServerSpatialHost::link(&mut spatial, 0);
        ServerSpatialHost::unlink(&mut spatial, 0);
        assert!(spatial.visibility(0).is_some());
    }

    #[test]
    #[should_panic(expected = "SV_SetBrushModel: bad is not a brush model")]
    fn brush_model_rejects_names() {
        let mut spatial = spatial();
        let _ = spatial.records.actor(0);
        ServerSpatialHost::set_brush_model(&mut spatial, 0, "bad");
    }
}
