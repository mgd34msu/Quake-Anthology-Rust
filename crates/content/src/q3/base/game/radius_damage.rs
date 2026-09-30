//! Quake III base/game: radius damage.
//!
//! Donor provenance: `src/content/q3/base/game/radius-damage.ts`.

use qa_core::identity::ActorId;
use qa_core::math::add3;
use qa_core::math::length3;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::Bounds;
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::SpatialQueries;
use crate::q3::base::world::{ActorTraceHit, ActorTraceQuery, TraceShape};

// ---------------------------------------------------------------------------
// radius-damage.ts: radius falloff and visibility
// ---------------------------------------------------------------------------

/// Radius target (`Q3RadiusTarget`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3RadiusTarget {
    /// Origin.
    pub origin: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Accuracy eligible.
    pub accuracy_eligible: bool,
}

/// Radius host (`Q3RadiusHost`).
pub trait Q3RadiusHost {
    /// Spatial queries.
    fn spatial(&mut self) -> &mut dyn SpatialQueries;
    /// Resolve a target.
    fn target(&mut self, actor: &ActorId) -> Option<Q3RadiusTarget>;
    /// Apply damage.
    fn damage(&mut self, actor: &ActorId, direction: Vec3, point: Vec3, amount: i32);
}

/// Visibility check (`q3CanDamage`).
pub fn q3_can_damage(spatial: &dyn SpatialQueries, actor: &ActorId, bounds: &Bounds, origin: Vec3) -> bool {
    let midpoint = scale3(add3(bounds.min, bounds.max), 0.5);
    let trace = |end: Vec3| {
        spatial.trace_actor(&ActorTraceQuery {
            start: origin,
            end,
            shape: TraceShape::Point,
            pass_actor: None,
            mask: 1,
        })
    };
    let center = trace(midpoint);
    if center.fraction == 1.0 || matches!(&center.hit, ActorTraceHit::Actor { actor: hit } if hit == actor) {
        return true;
    }
    for (x, y) in [(15.0, 15.0), (15.0, -15.0), (-15.0, 15.0), (-15.0, -15.0)] {
        if trace(vec3(midpoint.x + x, midpoint.y + y, midpoint.z)).fraction == 1.0 {
            return true;
        }
    }
    false
}

/// Radius damage (`q3RadiusDamage`).
pub fn q3_radius_damage(
    host: &mut dyn Q3RadiusHost,
    origin: Vec3,
    amount: f32,
    radius: f32,
    ignore: Option<&ActorId>,
) -> bool {
    let radius = radius.max(1.0);
    let extent = vec3(radius, radius, radius);
    let candidates = host.spatial().area_actors(
        &Bounds {
            min: sub3(origin, extent),
            max: add3(origin, extent),
        },
        1024,
    );
    let mut hit_client = false;
    for actor in candidates {
        if ignore.is_some_and(|ignored| ignored == &actor) {
            continue;
        }
        let Some(target) = host.target(&actor) else { continue };
        let axis = |value: f32, min: f32, max: f32| -> f32 {
            if value < min {
                min - value
            } else if value > max {
                value - max
            } else {
                0.0
            }
        };
        let distance = length3(vec3(
            axis(origin.x, target.bounds.min.x, target.bounds.max.x),
            axis(origin.y, target.bounds.min.y, target.bounds.max.y),
            axis(origin.z, target.bounds.min.z, target.bounds.max.z),
        ));
        if distance >= radius {
            continue;
        }
        let points = amount * (1.0 - distance / radius);
        if !q3_can_damage(host.spatial(), &actor, &target.bounds, origin) {
            continue;
        }
        if target.accuracy_eligible {
            hit_client = true;
        }
        let direction = add3(sub3(target.origin, origin), vec3(0.0, 0.0, 24.0));
        host.damage(&actor, direction, origin, points.trunc() as i32);
    }
    hit_client
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::q3::base::game::state::test_support::*;

    use qa_core::identity::ActorId;

    use qa_core::math::vec3;
    use qa_core::math::Bounds;
    use qa_core::math::Vec3;

    #[test]
    fn radius_damage_falloff_and_visibility() {
        struct Host {
            spatial: StubSpatial,
            damages: Vec<(ActorId, i32)>,
        }
        impl Q3RadiusHost for Host {
            fn spatial(&mut self) -> &mut dyn SpatialQueries {
                &mut self.spatial
            }
            fn target(&mut self, actor: &ActorId) -> Option<Q3RadiusTarget> {
                Some(Q3RadiusTarget {
                    origin: vec3(51.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(50.0, -1.0, -1.0),
                        max: vec3(52.0, 1.0, 1.0),
                    },
                    accuracy_eligible: actor.slot() == 1,
                })
            }
            fn damage(&mut self, actor: &ActorId, _direction: Vec3, _point: Vec3, amount: i32) {
                self.damages.push((actor.clone(), amount));
            }
        }
        let owner = test_owner();
        let one = owner.actor(1, 1);
        let two = owner.actor(2, 1);
        let mut host = Host {
            spatial: StubSpatial {
                trace_result: clear_trace(vec3(10.0, 0.0, 0.0)),
                area: vec![one.clone(), two.clone()],
            },
            damages: Vec::new(),
        };
        assert!(q3_radius_damage(&mut host, vec3(0.0, 0.0, 0.0), 100.0, 100.0, None));
        assert_eq!(host.damages.len(), 2);
        assert_eq!(host.damages[0].1, 50);
        host.damages.clear();
        assert!(!q3_radius_damage(
            &mut host,
            vec3(0.0, 0.0, 0.0),
            100.0,
            100.0,
            Some(&one)
        ));
        assert_eq!(host.damages.len(), 1);
    }
}
