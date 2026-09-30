//! Quake III base/game: projectile.
//!
//! Donor provenance: `src/content/q3/base/game/projectile.ts`.

use qa_core::identity::ActorId;
use qa_core::math::add3;
use qa_core::math::length3;
use qa_core::math::normalize3;
use qa_core::math::scale3;
use qa_core::math::vec3;
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;

// ---------------------------------------------------------------------------
// projectile.ts: shared missile simulation
// ---------------------------------------------------------------------------

/// Shared projectile (`Q3Projectile`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Projectile {
    /// Actor.
    pub actor: ActorId,
    /// Owner.
    pub owner: ActorId,
    /// Weapon.
    pub weapon: i32,
    /// Direct damage.
    pub direct: i32,
    /// Splash damage.
    pub splash: i32,
    /// Splash radius.
    pub radius: i32,
    /// Means of death.
    pub method: i32,
    /// Splash means of death.
    pub splash_method: i32,
    /// Damage point.
    pub damage_point: Vec3,
    /// Trajectory.
    pub trajectory: Q3Trajectory,
    /// Flags.
    pub flags: i32,
    /// Pass actor.
    pub pass: Option<ActorId>,
}

/// Projectile target (`Q3ProjectileTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ProjectileTarget {
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Accuracy eligible.
    pub accuracy_eligible: bool,
    /// Invulnerable.
    pub invulnerable: bool,
}

/// Projectile impact event (`Q3ProjectileImpact`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ProjectileImpact {
    /// Bounce.
    Bounce {
        /// Normal.
        normal: Vec3,
    },
    /// Impact.
    Impact {
        /// Normal.
        normal: Vec3,
        /// Target.
        target: Option<ActorId>,
        /// Flesh.
        flesh: bool,
        /// Surface flags.
        surface_flags: i32,
    },
}

/// Projectile phase (`phase()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectilePhase {
    /// Flight.
    Flight,
    /// Event.
    Event,
    /// Attached.
    Attached,
}

/// Reflection outcome (`reflection.impact` return).
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectionOutcome {
    /// Miss.
    Miss,
    /// Hit with a bounce direction.
    Hit {
        /// Bounce direction.
        bounce_direction: Vec3,
    },
}

/// Projectile host (`Q3ProjectileHost`).
pub trait Q3ProjectileHost {
    /// Time.
    fn time(&self) -> i32;
    /// Previous time.
    fn previous_time(&self) -> i32;
    /// Whether the projectile is live (`live()`).
    fn is_live(&mut self) -> bool;
    /// Phase (`phase()`).
    fn phase(&mut self) -> ProjectilePhase;
    /// Event time (`eventTime()`).
    fn event_time(&mut self) -> i32;
    /// Clear the event (`clearEvent?`, default no-op).
    fn clear_event(&mut self) {}
    /// Current origin (`origin()`).
    fn origin(&mut self) -> Vec3;
    /// Move (`move(origin, velocity)`).
    fn move_to(&mut self, origin: Vec3, velocity: Vec3);
    /// Set origin (`setOrigin`).
    fn set_origin(&mut self, origin: Vec3);
    /// Link (`link`).
    fn link(&mut self);
    /// Release (`release`).
    fn release(&mut self);
    /// Trace (`trace`).
    fn trace(&mut self, start: Vec3, end: Vec3, pass: Option<&ActorId>) -> Q3TraceResult;
    /// Resolve a target (`target`).
    fn target(&mut self, actor: &ActorId) -> Option<Q3ProjectileTarget>;
    /// World actor (`worldActor`).
    fn world_actor(&mut self) -> ActorId;
    /// Emit an impact (`emit`).
    fn emit(&mut self, event: &Q3ProjectileImpact);
    /// Retain (`retain`).
    fn retain(&mut self);
    /// Damage (`damage`).
    fn damage(&mut self, target: &ActorId, direction: Vec3, point: Vec3);
    /// Radius damage (`radius`).
    fn radius(&mut self, origin: Vec3, ignore: Option<&ActorId>) -> bool;
    /// Credit accuracy (`accuracy`).
    fn accuracy(&mut self);
    /// Think (`think`).
    fn think(&mut self);
    /// Moved (`moved`).
    fn moved(&mut self);
    /// Reflection impact (`reflection`, default miss for null).
    fn reflection_impact(&mut self, target: &ActorId, direction: Vec3, point: Vec3) -> ReflectionOutcome {
        let _ = (target, direction, point);
        ReflectionOutcome::Miss
    }
    /// Whether reflection applies (`reflection` null check, default false).
    fn has_reflection(&mut self) -> bool {
        false
    }
    /// Special impact (`special.impact`, default false for null).
    fn special_impact(&mut self, trace: &Q3TraceResult, target: &ActorId) -> bool {
        let _ = (trace, target);
        false
    }
    /// Special after-move (`special.afterMove`, default no-op for null).
    fn special_after_move(&mut self) {}
    /// Special no-impact (`special.noImpact`, default no-op for null).
    fn special_no_impact(&mut self) {}
    /// Whether special hooks apply (`special` null check, default false).
    fn has_special(&mut self) -> bool {
        false
    }
}

/// Launch result (`q3LaunchProjectile` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileLaunch {
    /// Expiry time.
    pub expires: i32,
    /// Trajectory.
    pub trajectory: Q3Trajectory,
}

/// Build a launch trajectory (`q3LaunchProjectile`).
#[must_use]
pub fn q3_launch_projectile(
    start: Vec3,
    direction: Vec3,
    speed: f32,
    gravity: bool,
    duration: i32,
    time: i32,
) -> ProjectileLaunch {
    ProjectileLaunch {
        expires: time.wrapping_add(duration),
        trajectory: Q3Trajectory {
            trajectory_type: if gravity {
                TrajectoryType::Gravity
            } else {
                TrajectoryType::Linear
            },
            time: time.wrapping_sub(50),
            duration: 0,
            base: start,
            delta: snap_vector(scale3(direction, speed)),
        },
    }
}

pub(crate) fn trace_normal(trace: &Q3TraceResult) -> Vec3 {
    match trace.contact {
        Q3TraceContact::Plane { normal, .. } => normal,
        Q3TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Bounce a projectile (`q3BounceProjectile`).
pub fn q3_bounce_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost, trace: &Q3TraceResult) {
    let hit_time = q3_missile_hit_time(host.previous_time(), host.time(), trace.fraction);
    let plane = trace_normal(trace);
    let half = projectile.flags & 0x20 != 0;
    let delta = q3_bounce_velocity(evaluate_trajectory_delta(&projectile.trajectory, hit_time), plane, half);
    projectile.trajectory.delta = delta;
    if half && plane.z > 0.2 && length3(delta) < 40.0 {
        host.set_origin(trace.end);
        return;
    }
    let origin = add3(host.origin(), plane);
    host.move_to(origin, delta);
    projectile.trajectory.base = origin;
    projectile.trajectory.time = host.time();
}

/// Explode a projectile (`q3ExplodeProjectile`).
pub fn q3_explode_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost) {
    let origin = snap_vector(evaluate_trajectory(&projectile.trajectory, host.time()));
    host.set_origin(origin);
    host.emit(&Q3ProjectileImpact::Impact {
        normal: vec3(0.0, 0.0, 1.0),
        target: None,
        flesh: false,
        surface_flags: 0,
    });
    if !host.is_live() {
        return;
    }
    host.retain();
    if projectile.splash != 0 && host.radius(origin, Some(&projectile.actor.clone())) {
        host.accuracy();
    }
    if host.is_live() {
        host.link();
    }
}

/// Impact a projectile (`q3ImpactProjectile`).
pub fn q3_impact_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost, trace: &Q3TraceResult) {
    let actor = match &trace.hit {
        Q3TraceHit::Actor(actor) => actor.clone(),
        _ => host.world_actor(),
    };
    let plane = trace_normal(trace);
    let target = host.target(&actor);
    if target.is_none_or(|target| !target.damageable) && projectile.flags & 0x30 != 0 {
        q3_bounce_projectile(projectile, host, trace);
        host.emit(&Q3ProjectileImpact::Bounce { normal: plane });
        return;
    }
    if host.has_reflection()
        && target.is_some_and(|target| target.damageable && target.invulnerable)
        && projectile.weapon != 12
    {
        let effect = host.reflection_impact(
            &actor,
            normalize3(projectile.trajectory.delta),
            projectile.trajectory.base,
        );
        if !host.is_live() {
            return;
        }
        if let ReflectionOutcome::Hit { bounce_direction } = effect {
            let half = projectile.flags & 0x20;
            projectile.flags &= !0x20;
            let reflected = Q3TraceResult {
                contact: Q3TraceContact::Plane {
                    normal: bounce_direction,
                    distance: 0.0,
                },
                ..trace.clone()
            };
            q3_bounce_projectile(projectile, host, &reflected);
            projectile.flags |= half;
        }
        projectile.pass = Some(actor);
        return;
    }
    let mut hit_client = false;
    if target.is_some_and(|target| target.damageable) && projectile.direct != 0 {
        if target.is_some_and(|target| target.accuracy_eligible) {
            host.accuracy();
            hit_client = true;
        }
        let mut velocity = evaluate_trajectory_delta(&projectile.trajectory, host.time());
        if length3(velocity) == 0.0 {
            velocity = vec3(velocity.x, velocity.y, 1.0);
        }
        let point = projectile.damage_point;
        host.damage(&actor, velocity, point);
        if !host.is_live() {
            return;
        }
    }
    if (host.has_special() && host.special_impact(trace, &actor)) || !host.is_live() {
        return;
    }
    let current = host.target(&actor);
    host.emit(&Q3ProjectileImpact::Impact {
        normal: plane,
        target: Some(actor.clone()),
        flesh: current.is_some_and(|target| target.damageable && target.player),
        surface_flags: trace.surface_flags,
    });
    if !host.is_live() {
        return;
    }
    host.retain();
    let base = projectile.trajectory.base;
    let origin = snap_vector_towards(trace.end, base);
    host.set_origin(origin);
    if projectile.splash != 0 && host.radius(origin, Some(&actor)) && !hit_client {
        host.accuracy();
    }
    if host.is_live() {
        host.link();
    }
}

/// Step a projectile (`q3StepProjectile`).
pub fn q3_step_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost) {
    if !host.is_live() {
        return;
    }
    if host.phase() == ProjectilePhase::Event {
        if host.time().wrapping_sub(host.event_time()) > EVENT_VALID_MSEC {
            host.release();
        }
        return;
    }
    if host.time().wrapping_sub(host.event_time()) > EVENT_VALID_MSEC {
        host.clear_event();
    }
    if host.phase() == ProjectilePhase::Attached {
        host.think();
        return;
    }
    let origin = host.origin();
    let destination = evaluate_trajectory(&projectile.trajectory, host.time());
    let pass = projectile.pass.clone();
    let mut trace = host.trace(origin, destination, pass.as_ref());
    if trace.solidity != Q3Solidity::Clear {
        let mut stuck = host.trace(origin, origin, pass.as_ref());
        stuck.fraction = 0.0;
        trace = stuck;
    } else {
        let velocity = evaluate_trajectory_delta(&projectile.trajectory, host.time());
        host.move_to(trace.end, velocity);
    }
    host.link();
    if trace.fraction != 1.0 {
        if trace.surface_flags & 16 != 0 {
            if host.has_special() {
                host.special_no_impact();
            }
            if host.is_live() {
                host.release();
            }
            return;
        }
        q3_impact_projectile(projectile, host, &trace);
        if !host.is_live() || host.phase() != ProjectilePhase::Flight {
            return;
        }
    }
    if host.has_special() {
        host.special_after_move();
    }
    if !host.is_live() {
        return;
    }
    host.moved();
    host.think();
}
