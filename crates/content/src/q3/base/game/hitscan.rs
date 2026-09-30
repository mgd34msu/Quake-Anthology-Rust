//! Quake III base/game: hitscan.
//!
//! Donor provenance: `src/content/q3/base/game/hitscan.ts`.

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, normalize3, scale3, sub3, vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::ballistics_math::*;
use crate::q3::base::game::entities::GameRandomMirror;
use crate::q3::base::game::missile::{snap_vector, snap_vector_towards};
use crate::q3::base::world::{ActorTraceHit, ActorTraceResult, TraceContact};

// ---------------------------------------------------------------------------
// Hitscan (hitscan.ts).
// ---------------------------------------------------------------------------

/// Surface flag for no-damage surfaces (`SURF_NODAMAGE`).
pub(crate) const SURF_NODAMAGE: i32 = 0x10;

/// Solid contents bit (`CONTENTS_SOLID`).
pub(crate) const CONTENTS_SOLID: i32 = 0x1;

/// Accuracy subject record (`Q3AccuracySubject`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AccuracySubject {
    /// Actor handle.
    pub actor: ActorId,
    /// Can take damage.
    pub damageable: bool,
    /// Is a player.
    pub player: bool,
    /// Health.
    pub health: i32,
    /// Team identity.
    pub team: Option<String>,
}

/// Accuracy-hit test (`q3AccuracyHit`).
#[must_use]
pub fn q3_accuracy_hit(team_game: bool, target: &Q3AccuracySubject, attacker: &Q3AccuracySubject) -> bool {
    target.damageable
        && target.actor != attacker.actor
        && target.player
        && attacker.player
        && target.health > 0
        && (!team_game || target.team != attacker.team)
}

/// Bullet attack frame (`Q3BulletAttack`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BulletAttack {
    /// Forward direction.
    pub forward: Vec3,
    /// Right direction.
    pub right: Vec3,
    /// Up direction.
    pub up: Vec3,
    /// Muzzle origin.
    pub muzzle: Vec3,
    /// Quad damage multiplier.
    pub quad: f32,
}

/// Bullet target record (`Q3BulletTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3BulletTarget {
    /// Can take damage.
    pub damageable: bool,
    /// Is a player.
    pub player: bool,
    /// Eligible for accuracy credit.
    pub accuracy_eligible: bool,
    /// Invulnerable.
    pub invulnerable: bool,
}

/// Bullet impact event.
#[derive(Debug, Clone, PartialEq)]
pub struct BulletEmitEvent {
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Target actor.
    pub target: Option<ActorId>,
    /// Hit flesh.
    pub flesh: bool,
}

/// Invulnerability impact result.
#[derive(Debug, Clone, PartialEq)]
pub enum InvulnerabilityImpact {
    /// Pass through.
    Miss,
    /// Bounce with a new impact point.
    Hit {
        /// Impact point.
        impact_point: Vec3,
        /// Bounce direction.
        bounce_direction: Vec3,
    },
}

/// Hitscan product services.
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub enum HitscanProduct {
    /// Base game.
    Baseq3,
    /// Mission pack with invulnerability impacts.
    Missionpack {
        /// Invulnerability impact resolver.
        invulnerability_impact: Rc<dyn Fn(&ActorId, Vec3, Vec3) -> InvulnerabilityImpact>,
    },
}

/// Bullet host services (`Q3BulletHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Q3BulletHost {
    /// Game random.
    pub random: Rc<RefCell<GameRandomMirror>>,
    /// Trace helper.
    pub trace: Rc<dyn Fn(Vec3, Vec3, Option<ActorId>) -> ActorTraceResult>,
    /// Target record by actor.
    pub target: Rc<dyn Fn(&ActorId) -> Option<Q3BulletTarget>>,
    /// Impact mark helper.
    pub impact: Option<Rc<dyn Fn(Vec3)>>,
    /// Impact event emitter.
    pub emit: Rc<dyn Fn(BulletEmitEvent)>,
    /// Damage application.
    pub damage: Rc<dyn Fn(&ActorId, Vec3, Vec3, i32)>,
    /// Accuracy credit.
    pub credit_accuracy_hit: Rc<dyn Fn()>,
    /// Product services.
    pub product: HitscanProduct,
}

/// Fire a bullet (`q3BulletFire`, `Bullet_Fire`).
pub fn q3_bullet_fire(host: &Q3BulletHost, shooter: &ActorId, attack: &mut Q3BulletAttack, spread: f32, amount: f32) {
    let mut end = q3_bullet_endpoint(
        &BallisticAttack {
            muzzle: attack.muzzle,
            forward: attack.forward,
            right: attack.right,
            up: attack.up,
        },
        spread,
        &mut *host.random.borrow_mut(),
    );
    let mut pass: Option<ActorId> = Some(shooter.clone());
    for _ in 0..10 {
        let trace = (host.trace)(attack.muzzle, end, pass.clone());
        if (trace.surface_flags & SURF_NODAMAGE) != 0 {
            return;
        }
        if !matches!(trace.hit, ActorTraceHit::None) {
            if let Some(impact) = &host.impact {
                impact(trace.end);
            }
        }
        let actor = match &trace.hit {
            ActorTraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        let target = actor.as_ref().and_then(|actor| (host.target)(actor));
        let point = snap_vector_towards(trace.end, attack.muzzle);
        let flesh = target.is_some_and(|record| record.damageable && record.player);
        (host.emit)(BulletEmitEvent {
            point,
            normal: match &trace.contact {
                TraceContact::Plane { plane } => plane.normal,
                TraceContact::None => vec3(0.0, 0.0, 0.0),
            },
            target: actor.clone(),
            flesh,
        });
        if flesh && target.is_some_and(|record| record.accuracy_eligible) {
            (host.credit_accuracy_hit)();
        }
        if let Some(actor) = actor {
            if target.is_some_and(|record| record.damageable) {
                let record = target.unwrap_or(Q3BulletTarget {
                    damageable: true,
                    player: false,
                    accuracy_eligible: false,
                    invulnerable: false,
                });
                if matches!(host.product, HitscanProduct::Missionpack { .. }) && record.player && record.invulnerable {
                    let HitscanProduct::Missionpack { invulnerability_impact } = &host.product else {
                        break;
                    };
                    match invulnerability_impact(&actor, attack.forward, point) {
                        InvulnerabilityImpact::Hit {
                            impact_point,
                            bounce_direction,
                        } => {
                            let incoming = sub3(impact_point, attack.muzzle);
                            let reflection = add3(
                                incoming,
                                scale3(bounce_direction, -2.0 * dot3(incoming, bounce_direction)),
                            );
                            end = add3(impact_point, scale3(normalize3(reflection), 8192.0));
                            attack.muzzle = impact_point;
                            pass = None;
                        }
                        InvulnerabilityImpact::Miss => {
                            attack.muzzle = point;
                            pass = Some(actor);
                        }
                    }
                    continue;
                }
                (host.damage)(&actor, attack.forward, point, qvm_float_to_int(amount * attack.quad));
            }
        }
        break;
    }
}

/// Contact weapon event (`Q3ContactEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ContactEvent {
    /// Flesh hit.
    Hit {
        /// Impact point.
        point: Vec3,
        /// Impact normal.
        normal: Vec3,
        /// Target actor.
        target: ActorId,
    },
    /// Wall miss.
    Miss {
        /// Impact point.
        point: Vec3,
        /// Impact normal.
        normal: Vec3,
    },
    /// Lightning reflection.
    LightningReflection {
        /// Reflection start.
        start: Vec3,
        /// Reflection end.
        end: Vec3,
    },
    /// Quad-amplified gauntlet hit.
    GauntletQuad,
}

/// Contact host services (`Q3ContactHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Q3ContactHost {
    /// Trace helper.
    pub trace: Rc<dyn Fn(Vec3, Vec3, Option<ActorId>) -> ActorTraceResult>,
    /// Target record by actor.
    pub target: Rc<dyn Fn(&ActorId) -> Option<Q3BulletTarget>>,
    /// Impact mark helper.
    pub impact: Option<Rc<dyn Fn(Vec3)>>,
    /// Event emitter.
    pub emit: Rc<dyn Fn(Q3ContactEvent)>,
    /// Damage application.
    pub damage: Rc<dyn Fn(&ActorId, Vec3, Vec3, i32)>,
    /// Accuracy credit.
    pub credit_accuracy_hit: Rc<dyn Fn()>,
    /// Product services.
    pub product: HitscanProduct,
}

/// Trace contact normal (`traceNormal`).
pub(crate) fn trace_normal(trace: &ActorTraceResult) -> Vec3 {
    match &trace.contact {
        TraceContact::Plane { plane } => plane.normal,
        TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Quad-scaled damage (`scaledDamage`).
pub(crate) fn scaled_damage(amount: f32, attack: &Q3BulletAttack) -> i32 {
    qvm_float_to_int(amount * attack.quad)
}

/// Reflected ray endpoint (`reflectedEnd`).
pub(crate) fn reflected_end(start: Vec3, point: Vec3, direction: Vec3) -> Vec3 {
    let incoming = sub3(point, start);
    add3(
        point,
        scale3(
            normalize3(add3(incoming, scale3(direction, -2.0 * dot3(incoming, direction)))),
            8192.0,
        ),
    )
}

/// Gauntlet attack (`q3GauntletAttack`).
pub fn q3_gauntlet_attack(host: &Q3ContactHost, shooter: &ActorId, attack: &Q3BulletAttack, quad_active: bool) -> bool {
    let trace = (host.trace)(
        attack.muzzle,
        add3(attack.muzzle, scale3(attack.forward, 32.0)),
        Some(shooter.clone()),
    );
    if (trace.surface_flags & SURF_NODAMAGE) != 0 {
        return false;
    }
    let ActorTraceHit::Actor { actor } = &trace.hit else {
        return false;
    };
    let target = (host.target)(actor);
    if !target.is_some_and(|record| record.damageable) {
        return false;
    }
    if let Some(impact) = &host.impact {
        impact(trace.end);
    }
    if target.is_some_and(|record| record.player) {
        (host.emit)(Q3ContactEvent::Hit {
            point: trace.end,
            normal: trace_normal(&trace),
            target: actor.clone(),
        });
    }
    if quad_active {
        (host.emit)(Q3ContactEvent::GauntletQuad);
    }
    (host.damage)(actor, attack.forward, trace.end, scaled_damage(50.0, attack));
    true
}

/// Lightning fire (`q3LightningFire`).
pub fn q3_lightning_fire(host: &Q3ContactHost, shooter: &ActorId, attack: &mut Q3BulletAttack) {
    let mut pass: Option<ActorId> = Some(shooter.clone());
    for count in 0..10 {
        let trace = (host.trace)(
            attack.muzzle,
            add3(attack.muzzle, scale3(attack.forward, 768.0)),
            pass.clone(),
        );
        if matches!(host.product, HitscanProduct::Missionpack { .. }) && count != 0 {
            (host.emit)(Q3ContactEvent::LightningReflection {
                start: attack.muzzle,
                end: snap_vector(trace.end),
            });
        }
        if matches!(trace.hit, ActorTraceHit::None) {
            return;
        }
        if (trace.surface_flags & SURF_NODAMAGE) == 0 {
            if let Some(impact) = &host.impact {
                impact(trace.end);
            }
        }
        let actor = match &trace.hit {
            ActorTraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        let target = actor.as_ref().and_then(|actor| (host.target)(actor));
        if let Some(actor) = &actor {
            if target.is_some_and(|record| record.damageable) {
                let record = target.unwrap_or(Q3BulletTarget {
                    damageable: true,
                    player: false,
                    accuracy_eligible: false,
                    invulnerable: false,
                });
                if matches!(host.product, HitscanProduct::Missionpack { .. }) && record.player && record.invulnerable {
                    let HitscanProduct::Missionpack { invulnerability_impact } = &host.product else {
                        break;
                    };
                    match invulnerability_impact(actor, attack.forward, trace.end) {
                        InvulnerabilityImpact::Hit {
                            impact_point,
                            bounce_direction,
                        } => {
                            let end = reflected_end(attack.muzzle, impact_point, bounce_direction);
                            attack.muzzle = impact_point;
                            attack.forward = normalize3(sub3(end, impact_point));
                            pass = None;
                        }
                        InvulnerabilityImpact::Miss => {
                            attack.muzzle = trace.end;
                            pass = Some(actor.clone());
                        }
                    }
                    continue;
                }
                (host.damage)(actor, attack.forward, trace.end, scaled_damage(8.0, attack));
            }
        }
        let after = actor.as_ref().and_then(|actor| (host.target)(actor));
        if actor
            .as_ref()
            .is_some_and(|_| after.is_some_and(|record| record.damageable && record.player))
        {
            let actor = actor.unwrap_or_else(|| shooter.clone());
            (host.emit)(Q3ContactEvent::Hit {
                point: trace.end,
                normal: trace_normal(&trace),
                target: actor,
            });
            if after.is_some_and(|record| record.accuracy_eligible) {
                (host.credit_accuracy_hit)();
            }
        } else if (trace.surface_flags & SURF_NODAMAGE) == 0 {
            (host.emit)(Q3ContactEvent::Miss {
                point: trace.end,
                normal: trace_normal(&trace),
            });
        }
        break;
    }
}

/// Shotgun network event (`Q3ShotgunEvent`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ShotgunEvent {
    /// Muzzle origin.
    pub muzzle: Vec3,
    /// Fire direction.
    pub direction: Vec3,
    /// Pellet seed.
    pub seed: i32,
}

/// Shotgun host services (`Q3ShotgunHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Q3ShotgunHost {
    /// Trace helper.
    pub trace: Rc<dyn Fn(Vec3, Vec3, Option<ActorId>) -> ActorTraceResult>,
    /// Target record by actor.
    pub target: Rc<dyn Fn(&ActorId) -> Option<Q3BulletTarget>>,
    /// Impact mark helper.
    pub impact: Option<Rc<dyn Fn(Vec3)>>,
    /// Damage application.
    pub damage: Rc<dyn Fn(&ActorId, Vec3, Vec3, i32)>,
    /// Accuracy credit.
    pub credit_accuracy_hit: Rc<dyn Fn()>,
    /// Game random.
    pub random: Rc<RefCell<GameRandomMirror>>,
    /// Event publisher factory.
    pub begin: Rc<dyn Fn(Vec3, Vec3) -> Rc<dyn Fn(i32)>>,
    /// Shooter still alive.
    pub alive: Rc<dyn Fn() -> bool>,
    /// Product services.
    pub product: HitscanProduct,
}

/// One shotgun pellet (`shotgunPellet`).
pub(crate) fn shotgun_pellet(
    host: &Q3ShotgunHost,
    shooter: &ActorId,
    attack: &Q3BulletAttack,
    start: Vec3,
    end: Vec3,
) -> bool {
    let mut start = start;
    let mut end = end;
    let mut pass: Option<ActorId> = Some(shooter.clone());
    for _ in 0..10 {
        let trace = (host.trace)(start, end, pass.clone());
        if (trace.surface_flags & SURF_NODAMAGE) != 0 {
            return false;
        }
        if !matches!(trace.hit, ActorTraceHit::None) {
            if let Some(impact) = &host.impact {
                impact(trace.end);
            }
        }
        let ActorTraceHit::Actor { actor } = &trace.hit else {
            return false;
        };
        let target = (host.target)(actor);
        if !target.is_some_and(|record| record.damageable) {
            return false;
        }
        let record = target.unwrap_or(Q3BulletTarget {
            damageable: true,
            player: false,
            accuracy_eligible: false,
            invulnerable: false,
        });
        if matches!(host.product, HitscanProduct::Missionpack { .. }) && record.player && record.invulnerable {
            let HitscanProduct::Missionpack { invulnerability_impact } = &host.product else {
                return false;
            };
            match invulnerability_impact(actor, attack.forward, trace.end) {
                InvulnerabilityImpact::Hit {
                    impact_point,
                    bounce_direction,
                } => {
                    end = reflected_end(start, impact_point, bounce_direction);
                    start = impact_point;
                    pass = None;
                }
                InvulnerabilityImpact::Miss => {
                    start = trace.end;
                    pass = Some(actor.clone());
                }
            }
            continue;
        }
        (host.damage)(actor, attack.forward, trace.end, scaled_damage(10.0, attack));
        return (host.target)(actor).is_some_and(|record| record.accuracy_eligible);
    }
    false
}

/// Shotgun fire (`q3ShotgunFire`).
pub fn q3_shotgun_fire(host: &Q3ShotgunHost, shooter: &ActorId, attack: &Q3BulletAttack) {
    let muzzle = attack.muzzle;
    let direction = snap_vector(scale3(attack.forward, 4096.0));
    let publish = (host.begin)(muzzle, direction);
    let seed = host.random.borrow_mut().rand_value() & 255;
    publish(seed);
    let mut hit_client = false;
    for end in q3_shotgun_endpoints(muzzle, direction, seed) {
        if !(host.alive)() {
            break;
        }
        if shotgun_pellet(host, shooter, attack, muzzle, end) && !hit_client {
            hit_client = true;
            (host.credit_accuracy_hit)();
        }
    }
}

/// Rail trail impact (`Q3RailTrail.impact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RailImpact {
    /// No surface impact.
    None,
    /// Surface impact.
    Surface {
        /// Surface normal.
        normal: Vec3,
    },
}

/// Rail trail event (`Q3RailTrail`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3RailTrail {
    /// Trail start.
    pub start: Vec3,
    /// Trail end.
    pub end: Vec3,
    /// Impact record.
    pub impact: RailImpact,
}

/// Unlink restore callback.
pub type RailRestore = Box<dyn FnOnce()>;

/// Rail host services (`Q3RailHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Q3RailHost {
    /// Trace helper.
    pub trace: Rc<dyn Fn(Vec3, Vec3, Option<ActorId>) -> ActorTraceResult>,
    /// Target record by actor.
    pub target: Rc<dyn Fn(&ActorId) -> Option<Q3BulletTarget>>,
    /// Impact mark helper.
    pub impact: Option<Rc<dyn Fn(Vec3)>>,
    /// Damage application.
    pub damage: Rc<dyn Fn(&ActorId, Vec3, Vec3, i32)>,
    /// Shooter still alive.
    pub alive: Rc<dyn Fn() -> bool>,
    /// Temporarily unlink a penetrated actor.
    pub unlink: Rc<dyn Fn(&ActorId) -> Option<RailRestore>>,
    /// Trail emitter.
    pub trail: Rc<dyn Fn(Q3RailTrail)>,
    /// Product services.
    pub product: HitscanProduct,
}

/// Emit a rail trail from the current attack frame.
pub(crate) fn emit_rail_trail(host: &Q3RailHost, attack: &Q3BulletAttack, point: Vec3, impact: RailImpact) {
    (host.trail)(Q3RailTrail {
        start: add3(add3(attack.muzzle, scale3(attack.right, 4.0)), scale3(attack.up, -1.0)),
        end: point,
        impact,
    });
}

/// Railgun fire (`q3RailFire`).
pub fn q3_rail_fire(host: &Q3RailHost, shooter: &ActorId, attack: &mut Q3BulletAttack) -> i32 {
    let mut end = add3(attack.muzzle, scale3(attack.forward, 8192.0));
    let mut pass: Option<ActorId> = Some(shooter.clone());
    let mut hits = 0;
    let mut penetrated = 0;
    let mut restores: Vec<RailRestore> = Vec::new();
    let mut trace: Option<ActorTraceResult> = None;
    loop {
        if !(host.alive)() {
            break;
        }
        let current = (host.trace)(attack.muzzle, end, pass.clone());
        if !matches!(current.hit, ActorTraceHit::None) && (current.surface_flags & SURF_NODAMAGE) == 0 {
            if let Some(impact) = &host.impact {
                impact(current.end);
            }
        }
        let ActorTraceHit::Actor { actor } = &current.hit else {
            trace = Some(current);
            break;
        };
        let actor = actor.clone();
        let target = (host.target)(&actor);
        let mut recorded = false;
        if target.is_some_and(|record| record.damageable) {
            let record = target.unwrap_or(Q3BulletTarget {
                damageable: true,
                player: false,
                accuracy_eligible: false,
                invulnerable: false,
            });
            if matches!(host.product, HitscanProduct::Missionpack { .. }) && record.player && record.invulnerable {
                let HitscanProduct::Missionpack { invulnerability_impact } = &host.product else {
                    trace = Some(current);
                    break;
                };
                if let InvulnerabilityImpact::Hit {
                    impact_point,
                    bounce_direction,
                } = invulnerability_impact(&actor, attack.forward, current.end)
                {
                    end = reflected_end(attack.muzzle, impact_point, bounce_direction);
                    let snapped = ActorTraceResult {
                        end: snap_vector_towards(current.end, attack.muzzle),
                        ..current.clone()
                    };
                    trace = Some(snapped.clone());
                    emit_rail_trail(host, attack, snapped.end, RailImpact::None);
                    attack.muzzle = impact_point;
                    pass = None;
                    recorded = true;
                }
            } else {
                if target.is_some_and(|record| record.accuracy_eligible) {
                    hits += 1;
                }
                (host.damage)(&actor, attack.forward, current.end, scaled_damage(100.0, attack));
            }
        }
        if (current.contents & CONTENTS_SOLID) != 0 {
            trace = Some(current);
            break;
        }
        if !recorded {
            trace = Some(current.clone());
        }
        if let Some(restore) = (host.unlink)(&actor) {
            restores.push(restore);
        }
        penetrated += 1;
        if penetrated >= 4 {
            break;
        }
    }
    for restore in restores {
        restore();
    }
    if let Some(trace) = trace {
        let impact = if (trace.surface_flags & SURF_NODAMAGE) != 0 {
            RailImpact::None
        } else {
            RailImpact::Surface {
                normal: trace_normal(&trace),
            }
        };
        emit_rail_trail(host, attack, snap_vector_towards(trace.end, attack.muzzle), impact);
    }
    hits
}

/// Rail statistics state (`Q3RailStatistics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3RailStatistics {
    /// Consecutive multi-hit streak.
    pub streak: i32,
    /// Total hits.
    pub hits: i32,
    /// Impressive award count.
    pub impressive_count: i32,
    /// Reward expiry time.
    pub reward_until: i32,
}

/// Rail statistics update with award flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3RailStatisticsUpdate {
    /// Updated statistics.
    pub statistics: Q3RailStatistics,
    /// Impressive awarded.
    pub awarded: bool,
}

/// Rail statistics update (`q3RailStatistics`).
#[must_use]
pub fn q3_rail_statistics(state: &Q3RailStatistics, hits: i32, time: i32) -> Q3RailStatisticsUpdate {
    if hits == 0 {
        return Q3RailStatisticsUpdate {
            statistics: Q3RailStatistics { streak: 0, ..*state },
            awarded: false,
        };
    }
    let streak = state.streak.wrapping_add(hits);
    let awarded = streak >= 2;
    Q3RailStatisticsUpdate {
        statistics: Q3RailStatistics {
            streak: if awarded { streak.wrapping_sub(2) } else { streak },
            hits: state.hits.wrapping_add(1),
            impressive_count: if awarded {
                state.impressive_count.wrapping_add(1)
            } else {
                state.impressive_count
            },
            reward_until: if awarded {
                time.wrapping_add(2000)
            } else {
                state.reward_until
            },
        },
        awarded,
    }
}
