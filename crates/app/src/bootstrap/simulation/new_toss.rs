//! Rogue `NewToss` projectile physics.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/new-toss.ts`
//! (`stepQ2NewToss`; original Rogue `g_phys.c` `SV_Physics_NewToss` and
//! rerelease `rogue/g_rogue_phys.cpp`).
//!
//! The host supplies the actor tables, traces, and source hooks through
//! [`NewTossServices`]; only the operations the donor calls are modeled, so
//! the engine adapter stays a thin dispatch layer.

use qa_content::q2::foundation::host::Q2Motion;
use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;
use qa_core::numeric::NumericOps;
use qa_world::body::BodyState;

use super::physics::TraceResult;

/// Water reading backing one toss step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewTossWater {
    /// Water level.
    pub level: i32,
    /// Water contents type.
    pub water_type: i32,
}

/// Engine edition selecting the velocity clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewTossEdition {
    /// Classic per-component clamp.
    Classic,
    /// Rerelease vector clamp.
    Rerelease,
}

/// Outcome of one toss step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewTossOutcome {
    /// The actor moved and stays live.
    Moved,
    /// The actor rests on flat ground.
    Stopped,
    /// A team slave the caller moves with its master.
    TeamSlave,
    /// The actor is gone.
    Removed,
}

/// Source hooks and tables backing [`step_q2_new_toss`].
///
/// Mirrors the donor `Q2NewTossServices` interface (`new-toss.ts`); the
/// `actors`/`bodies` members surface as the operations the step calls so the
/// engine adapter can borrow its tables without re-exports.
pub trait NewTossServices {
    /// Numeric operations for this step.
    fn numeric(&self) -> NumericOps;
    /// Engine edition selecting the velocity clamp.
    fn edition(&self) -> NewTossEdition;
    /// World gravity scale.
    fn world_gravity(&self) -> f64;
    /// Maximum velocity magnitude.
    fn max_velocity(&self) -> f64;
    /// Stop speed feeding angular friction.
    fn stop_speed(&self) -> f64;
    /// Whether the actor rides a team master.
    fn team_slave(&self) -> bool;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Read a body record.
    fn read_body(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body record.
    fn write_body(&mut self, actor: &OwnedActor, state: BodyState);
    /// Re-link a body after it moves.
    fn link_body(&mut self, actor: &OwnedActor);
    /// Current water reading for the actor.
    fn water(&self) -> NewTossWater;
    /// Trace with the actor's exact source clipmask, including zero.
    fn trace(&mut self, start: Vec3, end: Vec3) -> TraceResult;
    /// Actor hit by a trace, if any.
    fn hit_actor(&self, trace: &TraceResult) -> Option<ActorId>;
    /// Run the shared fly-move for a frame.
    fn fly_move(&mut self, elapsed: f64);
    /// Persist angular velocity back to the source.
    fn write_angular_velocity(&mut self, velocity: Vec3);
    /// Touch triggers overlapped by the actor.
    fn touch_triggers(&mut self);
    /// Stored contents at a point.
    fn point_contents(&self, origin: Vec3) -> i32;
    /// Persist a water reading back to the source.
    fn write_water(&mut self, level: i32, water_type: i32);
    /// Play the water transition sound at an origin.
    fn water_sound(&mut self, origin: Vec3);
}

/// Step Rogue toss physics for one actor.
///
/// Source thinking precedes this call. The caller moves team followers only
/// after a [`NewTossOutcome::Moved`] result.
pub fn step_q2_new_toss(
    actor: &OwnedActor,
    elapsed: f64,
    motion: &Q2Motion,
    host: &mut impl NewTossServices,
) -> NewTossOutcome {
    let id = actor.id().clone();
    let Some(mut body) = host.read_body(&id) else {
        return NewTossOutcome::Removed;
    };
    if host.team_slave() {
        return NewTossOutcome::TeamSlave;
    }
    let n = host.numeric();
    let vector = |x: f64, y: f64, z: f64| Vec3 {
        x: n.store(x),
        y: n.store(y),
        z: n.store(z),
    };
    let scale = |value: Vec3, amount: f64| {
        vector(
            n.mul(f64::from(value.x), amount),
            n.mul(f64::from(value.y), amount),
            n.mul(f64::from(value.z), amount),
        )
    };
    let add = |left: Vec3, right: Vec3| {
        vector(
            n.add(f64::from(left.x), f64::from(right.x)),
            n.add(f64::from(left.y), f64::from(right.y)),
            n.add(f64::from(left.z), f64::from(right.z)),
        )
    };
    let length = |value: Vec3| {
        n.store(n.sqrt(n.add(
            n.add(
                n.mul(f64::from(value.x), f64::from(value.x)),
                n.mul(f64::from(value.y), f64::from(value.y)),
            ),
            n.mul(f64::from(value.z), f64::from(value.z)),
        )))
    };
    let moving = |value: Vec3| value.x != 0.0 || value.y != 0.0 || value.z != 0.0;

    let trace = host.trace(
        body.origin,
        vector(
            f64::from(body.origin.x),
            f64::from(body.origin.y),
            n.sub(f64::from(body.origin.z), 0.25),
        ),
    );
    let ground = if body.ground.as_ref().is_some_and(|ground| host.is_live(ground)) {
        host.hit_actor(&trace)
    } else {
        None
    };
    body.ground = ground.clone();
    host.write_body(actor, body.clone());
    let normal = trace.contact_normal().unwrap_or_else(|| trace.source_normal());
    if ground.is_some() && normal.z == 1.0 && !moving(body.velocity) {
        return NewTossOutcome::Stopped;
    }

    let old_origin = body.origin;
    let mut velocity = body.velocity;
    if host.edition() == NewTossEdition::Rerelease {
        let speed = f64::from(length(velocity));
        if speed > host.max_velocity() {
            velocity = scale(
                vector(
                    n.div(f64::from(velocity.x), speed),
                    n.div(f64::from(velocity.y), speed),
                    n.div(f64::from(velocity.z), speed),
                ),
                host.max_velocity(),
            );
        }
    } else {
        let max = host.max_velocity();
        let clamp = |value: f32| {
            if f64::from(value) > max {
                max
            } else if f64::from(value) < -max {
                -max
            } else {
                f64::from(value)
            }
        };
        velocity = vector(clamp(velocity.x), clamp(velocity.y), clamp(velocity.z));
    }
    let gravity = n.mul(n.mul(motion.gravity, host.world_gravity()), elapsed);
    velocity = if host.edition() == NewTossEdition::Rerelease || motion.gravity_vector.z > 0.0 {
        add(velocity, scale(motion.gravity_vector, gravity))
    } else {
        vector(
            f64::from(velocity.x),
            f64::from(velocity.y),
            n.sub(f64::from(velocity.z), gravity),
        )
    };
    body.velocity = velocity;
    host.write_body(actor, body.clone());

    if moving(motion.angular_velocity) {
        let angles = add(body.angles, scale(motion.angular_velocity, elapsed));
        let adjustment = n.store(n.mul(n.mul(elapsed, host.stop_speed()), 6.0));
        let friction = |value: f32| {
            if value > 0.0 {
                (n.store(n.sub(f64::from(value), f64::from(adjustment))) as f64).max(0.0)
            } else {
                (n.store(n.add(f64::from(value), f64::from(adjustment))) as f64).min(0.0)
            }
        };
        body.angles = angles;
        host.write_body(actor, body.clone());
        host.write_angular_velocity(vector(
            friction(motion.angular_velocity.x),
            friction(motion.angular_velocity.y),
            friction(motion.angular_velocity.z),
        ));
    }
    let speed = f64::from(length(velocity));
    let water_level = host.water().level;
    let loss = if water_level != 0 {
        n.mul(6.0, f64::from(water_level))
    } else if ground.is_none() {
        6.0
    } else {
        36.0
    };
    let newspeed = n.store(n.div(f64::from(n.store(n.sub(speed, loss))).max(0.0), speed));
    let Some(mut body) = host.read_body(&id) else {
        return NewTossOutcome::Removed;
    };
    body.velocity = scale(velocity, f64::from(newspeed));
    host.write_body(actor, body);
    host.fly_move(elapsed);
    if !host.is_live(&id) {
        return NewTossOutcome::Removed;
    }
    host.link_body(actor);
    host.touch_triggers();
    let Some(body) = host.read_body(&id) else {
        return NewTossOutcome::Removed;
    };
    let was_in_water = (host.water().water_type & 56) != 0;
    let water_type = host.point_contents(body.origin);
    let in_water = (water_type & 56) != 0;
    host.write_water(i32::from(in_water), water_type);
    if was_in_water != in_water {
        host.water_sound(if in_water { old_origin } else { body.origin });
    }
    if host.is_live(&id) {
        NewTossOutcome::Moved
    } else {
        NewTossOutcome::Removed
    }
}

#[cfg(test)]
mod tests {
    use qa_content::q2::foundation::host::Q2MotionKind;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Bounds};
    use qa_core::numeric::Q2_DONOR_PROFILE;
    use qa_world::movement::q2::types::{Q2Trace, Q2TracePlane};
    use qa_world::movement::types::{TraceContact, TraceHit};

    use super::*;

    struct FakeHost {
        numeric: NumericOps,
        edition: NewTossEdition,
        team_slave: bool,
        live: bool,
        body: Option<BodyState>,
        water: NewTossWater,
        ground_trace: TraceResult,
        contents: i32,
        angular: Vec3,
        sounds: Vec<Vec3>,
        touched: bool,
        flew: f64,
    }

    impl FakeHost {
        fn new(owner: &IdentityOwner, actor: &ActorId) -> (Self, OwnedActor) {
            let owned = owner
                .owned_actor(actor, qa_core::identity::ProviderId::new("q2", "test"))
                .expect("owned");
            let host = Self {
                numeric: NumericOps::select(Q2_DONOR_PROFILE).expect("numeric"),
                edition: NewTossEdition::Classic,
                team_slave: false,
                live: true,
                body: Some(BodyState {
                    origin: vec3(0.0, 0.0, 10.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-8.0, -8.0, -8.0),
                        max: vec3(8.0, 8.0, 8.0),
                    },
                    ground: None,
                }),
                water: NewTossWater {
                    level: 0,
                    water_type: 0,
                },
                ground_trace: TraceResult::Q2(Q2Trace {
                    fraction: 1.0,
                    end: vec3(0.0, 0.0, 9.75),
                    start_solid: false,
                    all_solid: false,
                    contact: TraceContact::None,
                    hit: TraceHit::None,
                    contents: 0,
                    surface: None,
                    source_plane: Q2TracePlane {
                        normal: vec3(0.0, 0.0, 1.0),
                        dist: 9.75,
                        plane_type: 2,
                        signbits: 0,
                    },
                    secondary: None,
                }),
                contents: 0,
                angular: vec3(0.0, 0.0, 0.0),
                sounds: Vec::new(),
                touched: false,
                flew: 0.0,
            };
            (host, owned)
        }

        fn motion(&self, actor: &OwnedActor) -> Q2Motion {
            Q2Motion {
                actor: actor.clone(),
                velocity: vec3(0.0, 0.0, 0.0),
                angular_velocity: vec3(0.0, 0.0, 0.0),
                kind: Q2MotionKind::NewToss,
                gravity: 1.0,
                gravity_vector: vec3(0.0, 0.0, -1.0),
                clip_mask: 0,
                owner: None,
            }
        }
    }

    impl NewTossServices for FakeHost {
        fn numeric(&self) -> NumericOps {
            self.numeric
        }
        fn edition(&self) -> NewTossEdition {
            self.edition
        }
        fn world_gravity(&self) -> f64 {
            800.0
        }
        fn max_velocity(&self) -> f64 {
            2000.0
        }
        fn stop_speed(&self) -> f64 {
            100.0
        }
        fn team_slave(&self) -> bool {
            self.team_slave
        }
        fn is_live(&self, _actor: &ActorId) -> bool {
            self.live
        }
        fn read_body(&self, _actor: &ActorId) -> Option<BodyState> {
            self.body.clone()
        }
        fn write_body(&mut self, _actor: &OwnedActor, state: BodyState) {
            self.body = Some(state);
        }
        fn link_body(&mut self, _actor: &OwnedActor) {}
        fn water(&self) -> NewTossWater {
            self.water
        }
        fn trace(&mut self, _start: Vec3, _end: Vec3) -> TraceResult {
            self.ground_trace.clone()
        }
        fn hit_actor(&self, _trace: &TraceResult) -> Option<ActorId> {
            None
        }
        fn fly_move(&mut self, elapsed: f64) {
            self.flew += elapsed;
        }
        fn write_angular_velocity(&mut self, velocity: Vec3) {
            self.angular = velocity;
        }
        fn touch_triggers(&mut self) {
            self.touched = true;
        }
        fn point_contents(&self, _origin: Vec3) -> i32 {
            self.contents
        }
        fn write_water(&mut self, level: i32, water_type: i32) {
            self.water = NewTossWater { level, water_type };
        }
        fn water_sound(&mut self, origin: Vec3) {
            self.sounds.push(origin);
        }
    }

    #[test]
    fn removed_without_body() {
        let owner = IdentityOwner::create("toss").expect("owner");
        let id = owner.actor(1, 0);
        let (mut host, actor) = FakeHost::new(&owner, &id);
        host.body = None;
        let motion = host.motion(&actor);
        assert_eq!(
            step_q2_new_toss(&actor, 0.1, &motion, &mut host),
            NewTossOutcome::Removed
        );
    }

    #[test]
    fn team_slave_short_circuits() {
        let owner = IdentityOwner::create("toss").expect("owner");
        let id = owner.actor(1, 0);
        let (mut host, actor) = FakeHost::new(&owner, &id);
        host.team_slave = true;
        let motion = host.motion(&actor);
        assert_eq!(
            step_q2_new_toss(&actor, 0.1, &motion, &mut host),
            NewTossOutcome::TeamSlave
        );
    }

    #[test]
    fn resting_on_flat_ground_stops() {
        let owner = IdentityOwner::create("toss").expect("owner");
        let id = owner.actor(1, 0);
        let ground = owner.actor(2, 0);
        let (mut host, actor) = FakeHost::new(&owner, &id);
        host.body.as_mut().expect("body").ground = Some(ground.clone());
        host.ground_trace = TraceResult::Q2(Q2Trace {
            fraction: 0.0,
            end: vec3(0.0, 0.0, 10.0),
            start_solid: false,
            all_solid: false,
            contact: TraceContact::Plane(qa_core::math::Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 10.0,
            }),
            hit: TraceHit::World { model: 0 },
            contents: 1,
            surface: None,
            source_plane: Q2TracePlane {
                normal: vec3(0.0, 0.0, 1.0),
                dist: 10.0,
                plane_type: 2,
                signbits: 0,
            },
            secondary: None,
        });
        // Ground resolves through the trace hit.
        struct GroundHost {
            inner: FakeHost,
            ground: ActorId,
        }
        impl NewTossServices for GroundHost {
            fn numeric(&self) -> NumericOps {
                self.inner.numeric()
            }
            fn edition(&self) -> NewTossEdition {
                self.inner.edition()
            }
            fn world_gravity(&self) -> f64 {
                self.inner.world_gravity()
            }
            fn max_velocity(&self) -> f64 {
                self.inner.max_velocity()
            }
            fn stop_speed(&self) -> f64 {
                self.inner.stop_speed()
            }
            fn team_slave(&self) -> bool {
                self.inner.team_slave()
            }
            fn is_live(&self, actor: &ActorId) -> bool {
                self.inner.is_live(actor)
            }
            fn read_body(&self, actor: &ActorId) -> Option<BodyState> {
                self.inner.read_body(actor)
            }
            fn write_body(&mut self, actor: &OwnedActor, state: BodyState) {
                self.inner.write_body(actor, state);
            }
            fn link_body(&mut self, actor: &OwnedActor) {
                self.inner.link_body(actor);
            }
            fn water(&self) -> NewTossWater {
                self.inner.water()
            }
            fn trace(&mut self, start: Vec3, end: Vec3) -> TraceResult {
                self.inner.trace(start, end)
            }
            fn hit_actor(&self, _trace: &TraceResult) -> Option<ActorId> {
                Some(self.ground.clone())
            }
            fn fly_move(&mut self, elapsed: f64) {
                self.inner.fly_move(elapsed);
            }
            fn write_angular_velocity(&mut self, velocity: Vec3) {
                self.inner.write_angular_velocity(velocity);
            }
            fn touch_triggers(&mut self) {
                self.inner.touch_triggers();
            }
            fn point_contents(&self, origin: Vec3) -> i32 {
                self.inner.point_contents(origin)
            }
            fn write_water(&mut self, level: i32, water_type: i32) {
                self.inner.write_water(level, water_type);
            }
            fn water_sound(&mut self, origin: Vec3) {
                self.inner.water_sound(origin);
            }
        }
        let mut host = GroundHost { inner: host, ground };
        let motion = host.inner.motion(&actor);
        assert_eq!(
            step_q2_new_toss(&actor, 0.1, &motion, &mut host),
            NewTossOutcome::Stopped
        );
    }

    #[test]
    fn falling_gains_gravity_and_moves() {
        let owner = IdentityOwner::create("toss").expect("owner");
        let id = owner.actor(1, 0);
        let (mut host, actor) = FakeHost::new(&owner, &id);
        let motion = host.motion(&actor);
        assert_eq!(step_q2_new_toss(&actor, 0.1, &motion, &mut host), NewTossOutcome::Moved);
        let body = host.body.expect("body");
        assert!(body.velocity.z < 0.0);
        assert!(host.touched);
        assert_eq!(host.flew, 0.1);
        assert!(host.sounds.is_empty());
    }

    #[test]
    fn water_entry_sounds_at_old_origin() {
        let owner = IdentityOwner::create("toss").expect("owner");
        let id = owner.actor(1, 0);
        let (mut host, actor) = FakeHost::new(&owner, &id);
        host.contents = 32;
        let motion = host.motion(&actor);
        assert_eq!(step_q2_new_toss(&actor, 0.1, &motion, &mut host), NewTossOutcome::Moved);
        assert_eq!(host.sounds, vec![vec3(0.0, 0.0, 10.0)]);
        assert_eq!(host.water.level, 1);
    }

    #[test]
    fn rerelease_clamps_speed_vector() {
        let owner = IdentityOwner::create("toss").expect("owner");
        let id = owner.actor(1, 0);
        let (mut host, actor) = FakeHost::new(&owner, &id);
        host.edition = NewTossEdition::Rerelease;
        host.body.as_mut().expect("body").velocity = vec3(3000.0, 0.0, 0.0);
        let motion = host.motion(&actor);
        assert_eq!(step_q2_new_toss(&actor, 0.1, &motion, &mut host), NewTossOutcome::Moved);
        let body = host.body.expect("body");
        let speed = (f64::from(body.velocity.x).powi(2) + f64::from(body.velocity.z).powi(2)).sqrt();
        assert!(speed <= 2000.0 + 80.0);
    }
}
