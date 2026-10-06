//! Native source fields over the shared Q1 pusher transaction.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/native-q1-pusher.ts`.
//!
//! Native source fields join the same pusher transaction used by raw QC. The
//! donor spreads `physics.q1PusherServices(projection)` and adds `think`;
//! Rust cannot spread trait implementations, so [`NativeQ1Pusher`] implements
//! [`Q1PusherServices`](qa_world::movement::q1::types::Q1PusherServices)
//! directly, delegating physics-owned behavior to [`SharedPhysics`] and
//! joining source fields through [`Q1PusherGame`].

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;
use qa_core::numeric::NumericOps;
use qa_core::time::FrameContext;
use qa_world::movement::q1::types::{Q1PhysicsEntity, Q1PusherServices, Q1Trace};
use qa_world::movement::types::TraceHit;

/// Mirror of `SharedPhysics` from donor
/// `src/app/bootstrap/simulation/physics.ts` (canonical home:
/// `crate::bootstrap::simulation::physics`); unify post-merge.
///
/// Only the pusher-transaction surface used by
/// `createNativeQ1PusherServices` is mirrored. `push_entity` and `unlink`
/// name the private donor primitives behind `Q1PusherServices::push` and
/// `collisionEnabled(false)` (`SharedPhysics.pushEntity` and the scene
/// unlink); everything else maps one-to-one onto public donor methods.
pub trait SharedPhysics {
    /// Numeric operations (donor `movement.numeric`).
    fn numeric(&self) -> NumericOps;
    /// Candidate actors in source slot order.
    fn candidates(&mut self) -> Vec<ActorId>;
    /// Test an entity position.
    fn test_position(&mut self, entity: &Q1PhysicsEntity) -> TraceHit;
    /// Push an entity by a displacement (donor `pushEntity`).
    fn push_entity(&mut self, actor: &OwnedActor, displacement: Vec3) -> Q1Trace;
    /// Read a pusher snapshot (donor `readQ1Pusher`).
    fn read_q1_pusher(&mut self, actor: &ActorId) -> Option<Q1PhysicsEntity>;
    /// Write a pusher snapshot (donor `writeQ1Pusher`).
    fn write_q1_pusher(&mut self, entity: &Q1PhysicsEntity);
    /// Unlink one actor from traces (donor scene unlink).
    fn unlink(&mut self, actor: &ActorId);
    /// Touch triggers for one actor (donor `touchTriggers`).
    fn touch_triggers(&mut self, actor: &OwnedActor);
}

/// Native source fields read for one pusher participant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1PusherSourceView {
    /// Local pusher time in seconds (`ltime`).
    pub ltime: f64,
    /// Next think time in seconds.
    pub next_think: f64,
    /// Movement flags.
    pub movement_flags: i32,
    /// Whether a think callback is bound.
    pub has_think: bool,
}

/// Native game access for the pusher bridge (seam).
///
/// This is the exact game surface used by donor
/// `createNativeQ1PusherServices` (`entity`, `fields`, `nextThink`,
/// `movementFlags`, link, `blocked`, `think`). The native game binding
/// lands when the live path wires the pusher; tests inject fakes. Donor
/// null guards live in the [`NativeQ1Pusher`] core, so every adapter method
/// expects a live entity.
pub trait Q1PusherGame {
    /// Read native source fields, or [`None`] for a missing entity.
    fn pusher_source(&self, id: &ActorId) -> Option<Q1PusherSourceView>;
    /// Write native source fields.
    fn write_pusher_source(&mut self, id: &ActorId, ltime: f64, next_think: f64, flags: i32);
    /// Link an actor body.
    fn link_pusher_body(&mut self, actor: &OwnedActor);
    /// Run the blocked callback, if bound.
    fn fire_pusher_blocked(&mut self, id: &ActorId, other: &ActorId);
    /// Run the think callback with the current think frame.
    fn fire_pusher_think(&mut self, id: &ActorId, frame: &FrameContext);
}

/// Native Q1 pusher services joining source fields to shared physics.
pub struct NativeQ1Pusher<'g, 'p, G: Q1PusherGame + ?Sized, P: SharedPhysics + ?Sized> {
    game: &'g mut G,
    physics: &'p mut P,
    think_frame: FrameContext,
}

impl<'g, 'p, G: Q1PusherGame + ?Sized, P: SharedPhysics + ?Sized> NativeQ1Pusher<'g, 'p, G, P> {
    /// Borrow the native game and shared physics for one transaction.
    ///
    /// The think frame supplies the engine time donor `think()` reads from
    /// the game clock; update it every frame with [`set_think_frame`](Self::set_think_frame).
    pub fn new(game: &'g mut G, physics: &'p mut P, think_frame: FrameContext) -> Self {
        Self {
            game,
            physics,
            think_frame,
        }
    }

    /// Replace the think frame (donor game clock).
    pub fn set_think_frame(&mut self, frame: FrameContext) {
        self.think_frame = frame;
    }
}

/// Native source fields join the same pusher transaction used by raw QC.
pub fn create_native_q1_pusher_services<'g, 'p, G: Q1PusherGame, P: SharedPhysics>(
    game: &'g mut G,
    physics: &'p mut P,
    think_frame: FrameContext,
) -> NativeQ1Pusher<'g, 'p, G, P> {
    NativeQ1Pusher::new(game, physics, think_frame)
}

impl<G: Q1PusherGame + ?Sized, P: SharedPhysics + ?Sized> Q1PusherServices for NativeQ1Pusher<'_, '_, G, P> {
    fn numeric(&self) -> NumericOps {
        self.physics.numeric()
    }

    fn read(&mut self, actor: &ActorId) -> Option<Q1PhysicsEntity> {
        let mut physical = self.physics.read_q1_pusher(actor)?;
        if let Some(source) = self.game.pusher_source(actor) {
            physical.local_time_seconds = source.ltime;
            physical.next_think_seconds = source.next_think;
            physical.state.flags = source.movement_flags;
        }
        Some(physical)
    }

    fn candidates(&mut self) -> Vec<ActorId> {
        self.physics.candidates()
    }

    fn write(&mut self, entity: Q1PhysicsEntity) {
        self.physics.write_q1_pusher(&entity);
        let id = entity.actor.id().clone();
        if self.game.pusher_source(&id).is_some() {
            self.game.write_pusher_source(
                &id,
                entity.local_time_seconds,
                entity.next_think_seconds,
                entity.state.flags,
            );
        }
    }

    fn link(&mut self, actor: &OwnedActor, touch_triggers: bool) {
        self.game.link_pusher_body(actor);
        if touch_triggers {
            self.physics.touch_triggers(actor);
        }
    }

    fn collision_enabled(&mut self, actor: &OwnedActor, enabled: bool) {
        if enabled {
            self.link(actor, false);
        } else {
            let id = actor.id().clone();
            self.physics.unlink(&id);
        }
    }

    fn test_position(&mut self, entity: &Q1PhysicsEntity) -> TraceHit {
        self.physics.test_position(entity)
    }

    fn push(&mut self, entity: &Q1PhysicsEntity, displacement: Vec3) -> (Option<Q1PhysicsEntity>, Q1Trace) {
        // Publish cleared onground before the synchronous source touch callback.
        self.write(entity.clone());
        let trace = self.physics.push_entity(&entity.actor, displacement);
        let id = entity.actor.id().clone();
        (self.read(&id), trace)
    }

    fn blocked(&mut self, pusher: &OwnedActor, obstacle: &ActorId) {
        let id = pusher.id().clone();
        if self.game.pusher_source(&id).is_some() {
            self.game.fire_pusher_blocked(&id, obstacle);
        }
    }

    fn think(&mut self, pusher: &OwnedActor) {
        let id = pusher.id().clone();
        // Donor `?.think?.()`: without a bound callback the game clock is
        // untouched. Gating here also avoids `fire_think` clearing an
        // unbound think slot.
        if self.game.pusher_source(&id).is_some_and(|source| source.has_think) {
            let frame = self.think_frame;
            self.game.fire_pusher_think(&id, &frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{vec3, Bounds, Plane};
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use qa_core::time::{FramePhase, SourceTime};
    use qa_world::movement::q1::types::{Q1MovementState, Q1Solid};

    use super::*;

    struct FakeGame {
        sources: HashMap<ActorId, Q1PusherSourceView>,
        blocked: Vec<(ActorId, ActorId)>,
        thinks: Vec<ActorId>,
        links: Vec<ActorId>,
    }

    impl Q1PusherGame for FakeGame {
        fn pusher_source(&self, id: &ActorId) -> Option<Q1PusherSourceView> {
            self.sources.get(id).copied()
        }

        fn write_pusher_source(&mut self, id: &ActorId, ltime: f64, next_think: f64, flags: i32) {
            let source = self.sources.get_mut(id).expect("live test entity");
            source.ltime = ltime;
            source.next_think = next_think;
            source.movement_flags = flags;
        }

        fn link_pusher_body(&mut self, actor: &OwnedActor) {
            self.links.push(actor.id().clone());
        }

        fn fire_pusher_blocked(&mut self, id: &ActorId, other: &ActorId) {
            self.blocked.push((id.clone(), other.clone()));
        }

        fn fire_pusher_think(&mut self, id: &ActorId, _frame: &FrameContext) {
            self.thinks.push(id.clone());
        }
    }

    struct FakePhysics {
        snapshots: HashMap<ActorId, Q1PhysicsEntity>,
        writes: Vec<ActorId>,
        touches: Vec<ActorId>,
        unlinks: Vec<ActorId>,
    }

    impl SharedPhysics for FakePhysics {
        fn numeric(&self) -> NumericOps {
            NumericOps::select(Q1_DONOR_PROFILE).unwrap()
        }

        fn candidates(&mut self) -> Vec<ActorId> {
            let mut actors: Vec<ActorId> = self.snapshots.keys().cloned().collect();
            actors.sort_by_key(|actor| (actor.slot(), actor.generation()));
            actors
        }

        fn test_position(&mut self, _entity: &Q1PhysicsEntity) -> TraceHit {
            TraceHit::None
        }

        fn push_entity(&mut self, actor: &OwnedActor, _displacement: Vec3) -> Q1Trace {
            let id = actor.id().clone();
            self.touches.push(id);
            Q1Trace {
                fraction: 1.0,
                end: vec3(1.0, 0.0, 0.0),
                start_solid: false,
                all_solid: false,
                contact: qa_world::movement::types::TraceContact::None,
                hit: TraceHit::None,
                in_open: true,
                in_water: false,
                source_plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
            }
        }

        fn read_q1_pusher(&mut self, actor: &ActorId) -> Option<Q1PhysicsEntity> {
            self.snapshots.get(actor).cloned()
        }

        fn write_q1_pusher(&mut self, entity: &Q1PhysicsEntity) {
            let id = entity.actor.id().clone();
            self.writes.push(id.clone());
            self.snapshots.insert(id, entity.clone());
        }

        fn unlink(&mut self, actor: &ActorId) {
            self.unlinks.push(actor.clone());
        }

        fn touch_triggers(&mut self, actor: &OwnedActor) {
            self.touches.push(actor.id().clone());
        }
    }

    fn state() -> Q1MovementState {
        Q1MovementState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            old_origin: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 0.0),
            view_angles: vec3(0.0, 0.0, 0.0),
            punch_angles: vec3(0.0, 0.0, 0.0),
            move_type: 7,
            flags: 0,
            ground: TraceHit::None,
            water_level: 0,
            water_type: -1,
            teleport_time_seconds: 0.0,
            water_jump_direction: vec3(0.0, 0.0, 0.0),
            ideal_pitch: 0.0,
            fix_angle: false,
            health: 0.0,
        }
    }

    fn bounds() -> Bounds {
        Bounds {
            min: vec3(-16.0, -16.0, -16.0),
            max: vec3(16.0, 16.0, 16.0),
        }
    }

    fn frame() -> FrameContext {
        FrameContext {
            frame: 30,
            time: SourceTime::Seconds(3.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::EntityThink,
        }
    }

    fn harness() -> (IdentityOwner, OwnedActor, FakeGame, FakePhysics) {
        let owner = IdentityOwner::create("pusher-test").unwrap();
        let id = owner.actor(1, 1);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "game")).unwrap();
        let game = FakeGame {
            sources: HashMap::from([(
                actor.id().clone(),
                Q1PusherSourceView {
                    ltime: 4.0,
                    next_think: 5.0,
                    movement_flags: 512,
                    has_think: true,
                },
            )]),
            blocked: Vec::new(),
            thinks: Vec::new(),
            links: Vec::new(),
        };
        let physics = FakePhysics {
            snapshots: HashMap::from([(
                actor.id().clone(),
                Q1PhysicsEntity {
                    actor: actor.clone(),
                    state: state(),
                    bounds: bounds(),
                    absolute_bounds: bounds(),
                    solid: Q1Solid::Bsp,
                    local_time_seconds: 0.0,
                    next_think_seconds: 0.0,
                },
            )]),
            writes: Vec::new(),
            touches: Vec::new(),
            unlinks: Vec::new(),
        };
        (owner, actor, game, physics)
    }

    #[test]
    fn read_joins_source_fields() {
        let (_owner, actor, mut game, mut physics) = harness();
        let entity = NativeQ1Pusher::new(&mut game, &mut physics, frame())
            .read(actor.id())
            .unwrap();
        assert_eq!(entity.local_time_seconds, 4.0);
        assert_eq!(entity.next_think_seconds, 5.0);
        assert_eq!(entity.state.flags, 512);
    }

    #[test]
    fn read_without_source_returns_physical() {
        let (_owner, actor, mut game, mut physics) = harness();
        game.sources.remove(actor.id());
        let entity = NativeQ1Pusher::new(&mut game, &mut physics, frame())
            .read(actor.id())
            .unwrap();
        assert_eq!(entity.local_time_seconds, 0.0);
        assert_eq!(entity.state.flags, 0);
    }

    #[test]
    fn write_updates_physics_and_source() {
        let (_owner, actor, mut game, mut physics) = harness();
        let mut next = state();
        next.flags = 513;
        NativeQ1Pusher::new(&mut game, &mut physics, frame()).write(Q1PhysicsEntity {
            actor: actor.clone(),
            state: next,
            bounds: bounds(),
            absolute_bounds: bounds(),
            solid: Q1Solid::Bsp,
            local_time_seconds: 7.0,
            next_think_seconds: 8.0,
        });
        assert_eq!(physics.writes, vec![actor.id().clone()]);
        let source = game.sources[actor.id()];
        assert_eq!(source.ltime, 7.0);
        assert_eq!(source.next_think, 8.0);
        assert_eq!(source.movement_flags, 513);
    }

    #[test]
    fn link_touches_triggers_and_collision_toggles() {
        let (_owner, actor, mut game, mut physics) = harness();
        {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            pusher.link(&actor, true);
            pusher.link(&actor, false);
        }
        assert_eq!(game.links.len(), 2);
        assert_eq!(physics.touches, vec![actor.id().clone()]);
        {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            pusher.collision_enabled(&actor, false);
        }
        assert_eq!(physics.unlinks, vec![actor.id().clone()]);
        {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            pusher.collision_enabled(&actor, true);
        }
        assert_eq!(game.links.len(), 3);
        assert_eq!(physics.touches.len(), 1);
    }

    #[test]
    fn push_writes_then_reads() {
        let (_owner, actor, mut game, mut physics) = harness();
        let mut before = physics.snapshots[actor.id()].clone();
        before.local_time_seconds = 7.0;
        let (entity, trace, candidates, position) = {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            let (entity, trace) = pusher.push(&before, vec3(8.0, 0.0, 0.0));
            let candidates = pusher.candidates();
            let position = pusher.test_position(&before);
            (entity, trace, candidates, position)
        };
        assert_eq!(physics.writes, vec![actor.id().clone()]);
        assert_eq!(trace.fraction, 1.0);
        // The re-read joins the written source fields over the snapshot.
        assert_eq!(entity.unwrap().local_time_seconds, 7.0);
        assert_eq!(candidates, vec![actor.id().clone()]);
        assert_eq!(position, TraceHit::None);
    }

    #[test]
    fn blocked_and_think_gate_on_source() {
        let (owner, actor, mut game, mut physics) = harness();
        let other_id = owner.actor(2, 1);
        let other = owner.owned_actor(&other_id, ProviderId::new("q1", "game")).unwrap();
        {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            pusher.blocked(&actor, other.id());
            pusher.think(&actor);
        }
        assert_eq!(game.blocked, vec![(actor.id().clone(), other.id().clone())]);
        assert_eq!(game.thinks, vec![actor.id().clone()]);

        // Missing entities and unbound thinks are silent.
        game.sources.remove(actor.id());
        {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            pusher.blocked(&actor, other.id());
            pusher.think(&actor);
        }
        assert_eq!(game.blocked.len(), 1);
        assert_eq!(game.thinks.len(), 1);
        game.sources.insert(
            actor.id().clone(),
            Q1PusherSourceView {
                ltime: 0.0,
                next_think: -1.0,
                movement_flags: 0,
                has_think: false,
            },
        );
        {
            let mut pusher = NativeQ1Pusher::new(&mut game, &mut physics, frame());
            pusher.think(&actor);
            pusher.set_think_frame(FrameContext {
                frame: 90,
                time: SourceTime::Seconds(9.0),
                elapsed: SourceTime::Seconds(0.1),
                phase: FramePhase::EntityThink,
            });
        }
        assert_eq!(game.thinks.len(), 1);
    }
}
