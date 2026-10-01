//! Selected-monster spawn placement.
//!
//! Absolute donor:
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/monster-placement.ts`
//!
//! Q1 teleport-staging detection, authored-placement preservation checks for
//! both families, and the encounter-preserving nearby-placement search. Trace
//! vocabulary reuses the [`TraceQuery`]/[`TraceResult`] mirrors from
//! [`super::physics`]; game state is the real content services.

use std::cmp::Ordering;
use std::collections::HashSet;

use qa_content::bsp::{q1_entity_value, Q1Entity};
use qa_content::contract::{ContentId, MonsterDefinitionReference, ProviderReference};
use qa_content::q1::base::species::{species_by_classname, MonsterMovement};
use qa_content::q1::foundation::entity::parse_vector;
use qa_content::q1::foundation::entity::Q1Actor;
use qa_content::q1::foundation::entity_services::Q1EntityServices;
use qa_content::q1::foundation::gameplay::BodyState as Q1BodyState;
use qa_content::q1::foundation::types::{Q1MoveType, Q1Solid, Q1TraceRequest};
use qa_content::q2::foundation::fields::{integer_field, vector_field};
use qa_content::q2::foundation::host::{
    Q2Edition, Q2Entity, Q2GameServices, Q2MotionKind, Q2SpawnFields, Q2TraceRequest,
};
use qa_content::q2::foundation::monsters::ai::monster_solid_mask;
use qa_content::q2::foundation::monsters::types::{MonsterLocomotion, Q2MonsterDefinition};
use qa_content::q2::support::contracts::{BodyState as Q2BodyState, TraceHit as Q2TraceHit};
use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericProfile;
use qa_world::body::BodyState;
use qa_world::collision::{Q1Move, TracePolicy};
use qa_world::movement::types::{TraceContact, TraceHit, TraceShape};

use super::physics::{TraceQuery, TraceResult, TraceTarget};

/// Locomotion selector for the placement search (donor
/// `"walk" | "fly" | "swim"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementLocomotion {
    /// Walking hull.
    Walk,
    /// Flying hull.
    Fly,
    /// Swimming hull.
    Swim,
}

/// Authored encounter the search must stay reachable from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthoredPlacement {
    /// Authored origin.
    pub origin: Vec3,
    /// Authored hull.
    pub bounds: Bounds,
    /// Authored locomotion.
    pub locomotion: PlacementLocomotion,
}

/// Trace query shared by every placement sweep except start, end, and shape.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementQuery {
    /// Geometry target.
    pub target: TraceTarget,
    /// Gameplay namespace policy.
    pub policy: TracePolicy,
    /// Numeric profile for the sweeps.
    pub numeric: NumericProfile,
    /// Actor the sweeps pass through.
    pub pass_actor: Option<ActorId>,
}

/// Relocated spawn origin plus its ground.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementSpot {
    /// Spawn origin.
    pub origin: Vec3,
    /// Ground actor, if the spot rests on one.
    pub ground: Option<ActorId>,
}

/// Input to [`nearby_monster_placement`].
pub struct NearbyPlacement<'a, F, G, H> {
    /// Selected hull and its current origin.
    pub body: &'a BodyState,
    /// Selected locomotion.
    pub locomotion: PlacementLocomotion,
    /// World ground actor reported when the floor hit is not an actor.
    pub world_actor: Option<ActorId>,
    /// Whether an origin sits in the selected medium.
    pub same_medium: F,
    /// Authored encounter.
    pub authored: AuthoredPlacement,
    /// Shared trace query.
    pub query: PlacementQuery,
    /// Trace implementation.
    pub trace: G,
    /// Observer fed blocking actors, when set.
    pub blocked_by: Option<H>,
}

/// Q1 authored-placement preservation check input.
pub struct Q1PlacementInput<'a> {
    /// Map entity provider (donor reads `map.entities`; the port takes the
    /// two map fields it uses because `ResolvedMap` cannot be built outside
    /// content).
    pub entities: &'a ProviderReference,
    /// Selected map product (donor `map.geometryContent`).
    pub geometry_content: &'a ContentId,
    /// Authored source entity, when the map kept one.
    pub authored: Option<&'a Q1Entity>,
    /// Selected monster definition.
    pub definition: &'a MonsterDefinitionReference,
    /// Live Q1 game.
    pub game: &'a mut Q1EntityServices,
    /// Spawned entity.
    pub entity: &'a Q1Actor,
    /// Spawned body.
    pub body: &'a Q1BodyState,
}

/// Q2 authored-placement preservation check input.
pub struct Q2PlacementInput<'a> {
    /// Map entity provider (donor reads `map.entities`; see
    /// [`Q1PlacementInput`] for why the port narrows the map).
    pub entities: &'a ProviderReference,
    /// Selected map product (donor `map.geometryContent`).
    pub geometry_content: &'a ContentId,
    /// Authored spawn fields, when the map kept them.
    pub authored: Option<&'a Q2SpawnFields>,
    /// Selected monster definition.
    pub definition: &'a MonsterDefinitionReference,
    /// Native monster definition, when registered.
    pub native: Option<&'a Q2MonsterDefinition>,
    /// Live Q2 game.
    pub game: &'a mut Q2GameServices,
    /// Spawned entity.
    pub entity: &'a Q2Entity,
    /// Spawned body.
    pub body: &'a Q2BodyState,
}

/// Q1 maps may store live monsters in solid space inside a target-activated
/// teleport.
#[must_use]
pub fn is_q1_teleport_staging(game: &Q1EntityServices, body: &Q1BodyState) -> bool {
    for trigger in game.entities.values() {
        if trigger.classname != "trigger_teleport"
            || trigger.targetname.is_empty()
            || trigger.solid != Q1Solid::Trigger
            || trigger.touch.is_none()
            || trigger.spawnflags & 1 != 0
        {
            continue;
        }
        let Some(linked) = game.host.bodies.linked(trigger.actor.id()) else {
            continue;
        };
        let bounds = linked.absolute_bounds;
        let overlaps = body.origin.x + body.bounds.max.x >= bounds.min.x
            && body.origin.x + body.bounds.min.x <= bounds.max.x
            && body.origin.y + body.bounds.max.y >= bounds.min.y
            && body.origin.y + body.bounds.min.y <= bounds.max.y
            && body.origin.z + body.bounds.max.z >= bounds.min.z
            && body.origin.z + body.bounds.min.z <= bounds.max.z;
        if overlaps && !game.find(&trigger.target).is_empty() {
            return true;
        }
    }
    false
}

/// Preserve exact native Q1 startup outcomes, including failed floor drops
/// and flying overlap.
pub fn preserves_authored_q1_placement(input: Q1PlacementInput<'_>) -> bool {
    if input.entities.provider != ProviderId::new("q1", "official")
        || (input.definition.source.provider != ProviderId::new("q1", "monsters/classic/id1")
            && input.definition.source.provider != ProviderId::new("q1", "monsters/rerelease/id1"))
        || input.entities.content != input.definition.source.content
        || *input.geometry_content != input.definition.source.content
    {
        return false;
    }
    let Some(authored) = input.authored else { return false };
    let Some(classname) = q1_entity_value(authored, "classname") else {
        return false;
    };
    if classname != input.definition.classname || input.entity.classname != classname {
        return false;
    }
    let Some(species) = species_by_classname(classname) else {
        return false;
    };
    if input.entity.movement != Q1MoveType::Step
        || input.entity.model != format!("progs/{}.mdl", species.model)
        || input.body.bounds.min != species.bounds.min
        || input.body.bounds.max != species.bounds.max
    {
        return false;
    }
    let origin = parse_vector(q1_entity_value(authored, "origin").unwrap_or(""));
    if species.movement == MonsterMovement::Fly {
        return input.entity.movement_flags & 3 == 1 && input.body.origin == origin;
    }
    if species.movement != MonsterMovement::Walk
        || input.entity.movement_flags != 32
        || input.body.ground.is_some()
        || input.entity.solid != Q1Solid::Slidebox
    {
        return false;
    }
    let start = Vec3 {
        x: origin.x,
        y: origin.y,
        z: origin.z + 1.0,
    };
    if input.body.origin != start {
        return false;
    }
    let floor = (input.game.host.trace)(&Q1TraceRequest {
        start,
        end: Vec3 {
            x: start.x,
            y: start.y,
            z: start.z - 256.0,
        },
        bounds: input.body.bounds,
        ignore: Some(input.entity.actor.id().clone()),
        monsters: true,
        missile: false,
    });
    floor.fraction == 1.0 || floor.all_solid
}

/// Preserve exact native Q2 startup outcomes for ordinary walkers.
pub fn preserves_authored_q2_placement(input: Q2PlacementInput<'_>) -> bool {
    if input.entities.provider != ProviderId::new("q2", "official")
        || input.definition.source.provider != ProviderId::new("q2", "monsters/classic/baseq2")
        || input.entities.content != input.definition.source.content
        || *input.geometry_content != input.definition.source.content
        || input.game.options.edition != Q2Edition::Classic
    {
        return false;
    }
    let (Some(authored), Some(native)) = (input.authored, input.native) else {
        return false;
    };
    if authored.classname != input.definition.classname || input.entity.classname != authored.classname {
        return false;
    }
    if native.locomotion.unwrap_or(MonsterLocomotion::Walk) != MonsterLocomotion::Walk
        || input.entity.motion != Q2MotionKind::Step
        || input.entity.flags & 3 != 0
        || input.entity.model != native.model
        || input.entity.scale != 1.0
        || input.entity.gravity != 1.0
        || input.entity.gravity_vector
            != (Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            })
        || integer_field(authored, "spawnflags", 0) & !0x1f00 != input.entity.spawnflags
        || input.entity.spawnflags & 2 != 0
        || input.body.bounds.min != native.bounds.min
        || input.body.bounds.max != native.bounds.max
    {
        return false;
    }
    let mask = monster_solid_mask(input.game);
    let authored_origin = vector_field(authored, "origin");
    let start = Vec3 {
        x: authored_origin.x,
        y: authored_origin.y,
        z: authored_origin.z + 1.0,
    };
    let floor = input.game.host.trace(&Q2TraceRequest {
        start,
        end: Vec3 {
            x: start.x,
            y: start.y,
            z: start.z - 256.0,
        },
        bounds: Some(input.body.bounds),
        ignore: Some(input.entity.actor.id().clone()),
        mask,
        exclude: Vec::new(),
    });
    floor.fraction != 1.0
        && !floor.all_solid
        && matches!(floor.hit, Q2TraceHit::World { .. })
        && input.body.origin == floor.end
}

/// Spiral offsets within a radius, ordered by distance then `y` then `x`.
///
/// The donor caches these per radius in module state; the computation is
/// deterministic, so this port recomputes them per search instead of holding
/// a global cache.
fn offsets_within(radius: f32) -> Vec<(f32, f32)> {
    let extent = (radius / 4.0).ceil() as i32;
    let mut offsets = Vec::new();
    for x in -extent..=extent {
        for y in -extent..=extent {
            if f64::from(x * x + y * y) * 16.0 <= f64::from(radius) * f64::from(radius) {
                offsets.push((x as f32 * 4.0, y as f32 * 4.0));
            }
        }
    }
    offsets.sort_by(|a, b| {
        (a.0 * a.0 + a.1 * a.1)
            .partial_cmp(&(b.0 * b.0 + b.1 * b.1))
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .then_with(|| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal))
    });
    offsets
}

/// Route sweep used by the reachability probes.
type Route<'r> = dyn FnMut(Vec3, Vec3, TraceShape) -> TraceResult + 'r;

/// Whether the authored hull can travel from its origin to a candidate end.
fn reachable(route_trace: &mut Route<'_>, source_origin: Vec3, authored_bounds: Bounds, end: Vec3) -> bool {
    let route = route_trace(source_origin, end, TraceShape::Box(authored_bounds));
    if route.all_solid() || route.fraction() != 1.0 {
        return false;
    }
    if !route.start_solid() {
        return true;
    }
    // Authored hulls can overlap a wall. Require a clear center route and a
    // full-hull exit.
    let center = route_trace(source_origin, end, TraceShape::Point);
    let exit = route_trace(end, end, TraceShape::Box(authored_bounds));
    !center.start_solid() && !center.all_solid() && center.fraction() == 1.0 && !exit.start_solid() && !exit.all_solid()
}

fn offset(origin: Vec3, dx: f32, dy: f32, dz: f32) -> Vec3 {
    Vec3 {
        x: origin.x + dx,
        y: origin.y + dy,
        z: origin.z + dz,
    }
}

fn ground_hit(result: &TraceResult, world_actor: Option<ActorId>) -> Option<ActorId> {
    match result.hit() {
        TraceHit::Actor { actor } => Some(actor.clone()),
        _ => world_actor,
    }
}

/// Keep the authored encounter reachable by its original hull while fitting
/// the selected hull.
pub fn nearby_monster_placement<F, G, H>(input: NearbyPlacement<'_, F, G, H>) -> Option<PlacementSpot>
where
    F: Fn(&Vec3) -> bool,
    G: Fn(TraceQuery) -> TraceResult,
    H: FnMut(&ActorId),
{
    let NearbyPlacement {
        body,
        locomotion,
        world_actor,
        same_medium,
        authored,
        query,
        trace,
        mut blocked_by,
    } = input;
    let mut route_trace = |start: Vec3, end: Vec3, shape: TraceShape| -> TraceResult {
        let policy = match query.policy {
            TracePolicy::Q1 { .. } => TracePolicy::Q1 {
                movement: Q1Move::NoMonsters,
            },
            other => other,
        };
        let result = trace(TraceQuery {
            start,
            end,
            shape,
            target: query.target.clone(),
            policy,
            numeric: query.numeric,
            pass_actor: query.pass_actor.clone(),
        });
        if result.fraction() < 1.0 {
            if let TraceHit::Actor { actor } = result.hit() {
                if let Some(blocked_by) = blocked_by.as_mut() {
                    blocked_by(actor);
                }
            }
        }
        result
    };
    let source_start = offset(authored.origin, 0.0, 0.0, 1.0);
    let source_floor = route_trace(
        source_start,
        offset(source_start, 0.0, 0.0, -256.0),
        TraceShape::Box(authored.bounds),
    );
    let supported = !source_floor.start_solid() && !source_floor.all_solid() && source_floor.fraction() < 1.0;
    let source_origin = if supported && authored.locomotion == PlacementLocomotion::Walk {
        source_floor.end()
    } else {
        authored.origin
    };
    let feet = source_origin.z + authored.bounds.min.z;
    let anchor = Vec3 {
        x: source_floor.end().x,
        y: source_floor.end().y,
        z: feet - body.bounds.min.z,
    };
    let radius = 2.0
        * (body.bounds.max.x - body.bounds.min.x)
            .max(body.bounds.max.y - body.bounds.min.y)
            .max(authored.bounds.max.x - authored.bounds.min.x)
            .max(authored.bounds.max.y - authored.bounds.min.y);
    let offsets = offsets_within(radius);
    if locomotion != PlacementLocomotion::Walk {
        let mut vertical = vec![0.0];
        let mut z = 4.0;
        while z <= body.bounds.max.z - body.bounds.min.z {
            vertical.push(-z);
            vertical.push(z);
            z += 4.0;
        }
        for (dx, dy) in &offsets {
            for lift in &vertical {
                let origin = offset(body.origin, *dx, *dy, *lift);
                if !same_medium(&origin) {
                    continue;
                }
                let fit = trace(TraceQuery {
                    start: origin,
                    end: origin,
                    shape: TraceShape::Box(body.bounds),
                    target: query.target.clone(),
                    policy: query.policy,
                    numeric: query.numeric,
                    pass_actor: query.pass_actor.clone(),
                });
                if fit.start_solid() || fit.all_solid() {
                    continue;
                }
                let source_end = offset(origin, 0.0, 0.0, body.bounds.min.z - authored.bounds.min.z);
                if reachable(&mut route_trace, source_origin, authored.bounds, source_end) {
                    return Some(PlacementSpot { origin, ground: None });
                }
            }
        }
    }
    if locomotion == PlacementLocomotion::Walk {
        for (dx, dy) in &offsets {
            for lift in [1.0, 18.0] {
                let start = offset(anchor, *dx, *dy, lift);
                let drop = if supported && authored.locomotion == PlacementLocomotion::Walk {
                    18.0
                } else {
                    256.0
                };
                let floor = trace(TraceQuery {
                    start,
                    end: Vec3 {
                        x: start.x,
                        y: start.y,
                        z: anchor.z - drop,
                    },
                    shape: TraceShape::Box(body.bounds),
                    target: query.target.clone(),
                    policy: query.policy,
                    numeric: query.numeric,
                    pass_actor: query.pass_actor.clone(),
                });
                let walkable = match floor.contact() {
                    TraceContact::Plane(plane) => f64::from(plane.normal.z) >= 0.7,
                    TraceContact::None => false,
                };
                if floor.start_solid() || floor.all_solid() || floor.fraction() == 1.0 || !walkable {
                    continue;
                }
                if !same_medium(&floor.end()) {
                    continue;
                }
                let fit = trace(TraceQuery {
                    start: floor.end(),
                    end: floor.end(),
                    shape: TraceShape::Box(body.bounds),
                    target: query.target.clone(),
                    policy: query.policy,
                    numeric: query.numeric,
                    pass_actor: query.pass_actor.clone(),
                });
                if fit.start_solid() || fit.all_solid() {
                    continue;
                }
                let source_end = offset(floor.end(), 0.0, 0.0, body.bounds.min.z - authored.bounds.min.z);
                if !reachable(&mut route_trace, source_origin, authored.bounds, source_end) {
                    continue;
                }
                return Some(PlacementSpot {
                    origin: floor.end(),
                    ground: ground_hit(&floor, world_actor.clone()),
                });
            }
        }
    }
    if !supported || authored.locomotion != PlacementLocomotion::Walk {
        return None;
    }
    // Tight authored pockets can open around a corner: follow walkable
    // source-hull edges.
    let mut pending = vec![source_origin];
    let mut visited = HashSet::from([String::from("0,0")]);
    let reach = 512.0f32.max(radius);
    let mut index = 0;
    while index < pending.len() && index < 4096 {
        let current = pending[index];
        for direction in [(8.0, 0.0), (-8.0, 0.0), (0.0, 8.0), (0.0, -8.0)] {
            let x = current.x + direction.0;
            let y = current.y + direction.1;
            let key = format!("{},{}", x - source_origin.x, y - source_origin.y);
            if visited.contains(&key) || (x - source_origin.x).hypot(y - source_origin.y) > reach {
                continue;
            }
            for lift in [0.0, 18.0] {
                let start = offset(current, 0.0, 0.0, lift);
                let raised = route_trace(current, start, TraceShape::Box(authored.bounds));
                if raised.start_solid() || raised.all_solid() || raised.fraction() != 1.0 {
                    continue;
                }
                let across = route_trace(start, Vec3 { x, y, z: start.z }, TraceShape::Box(authored.bounds));
                if across.start_solid() || across.all_solid() || across.fraction() != 1.0 {
                    continue;
                }
                let floor = route_trace(
                    across.end(),
                    Vec3 {
                        x,
                        y,
                        z: current.z - 18.0,
                    },
                    TraceShape::Box(authored.bounds),
                );
                if floor.start_solid()
                    || floor.all_solid()
                    || floor.fraction() == 1.0
                    || !matches!(floor.contact(), TraceContact::Plane(plane) if f64::from(plane.normal.z) >= 0.7)
                {
                    continue;
                }
                visited.insert(key.clone());
                pending.push(floor.end());
                let origin = offset(floor.end(), 0.0, 0.0, authored.bounds.min.z - body.bounds.min.z);
                if same_medium(&origin) {
                    let fit = trace(TraceQuery {
                        start: origin,
                        end: origin,
                        shape: TraceShape::Box(body.bounds),
                        target: query.target.clone(),
                        policy: query.policy,
                        numeric: query.numeric,
                        pass_actor: query.pass_actor.clone(),
                    });
                    if !fit.start_solid() && !fit.all_solid() {
                        let ground = if locomotion != PlacementLocomotion::Walk {
                            None
                        } else {
                            ground_hit(&floor, world_actor.clone())
                        };
                        return Some(PlacementSpot { origin, ground });
                    }
                }
                break;
            }
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use qa_content::contract::{ContentId, ProviderReference};
    use qa_content::q2::base::monsters::registry::q2_classic_base_monster_definitions;
    use qa_content::q2::foundation::host::{Q2Entity, Q2MotionKind};
    use qa_content::q2::support::contracts::{
        Q2BspPlane, Q2TraceFields, TraceContact as Q2Contact, TraceFamily as Q2Family, TraceHit as Q2Hit,
        TraceResult as Q2Result,
    };
    use qa_core::math::Plane;
    use qa_core::numeric::Q1_DONOR_PROFILE;
    use qa_world::movement::q1::types::Q1Trace;

    use super::super::test_hosts::{test_q1_game, test_q2_game};

    fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn bounds(min: Vec3, max: Vec3) -> Bounds {
        Bounds { min, max }
    }

    fn hull(half: f32, down: f32, up: f32) -> Bounds {
        bounds(vec3(-half, -half, -down), vec3(half, half, up))
    }

    fn world_body(origin: Vec3, half: f32, down: f32, up: f32) -> BodyState {
        BodyState {
            origin,
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: hull(half, down, up),
            ground: None,
        }
    }

    fn base_query() -> PlacementQuery {
        PlacementQuery {
            target: TraceTarget::World,
            policy: TracePolicy::Q1 {
                movement: Q1Move::Normal,
            },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: None,
        }
    }

    fn q1_result(end: Vec3, fraction: f64, hit: TraceHit, normal_z: f32) -> TraceResult {
        TraceResult::Q1(Q1Trace {
            fraction,
            end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::Plane(Plane {
                normal: vec3(0.0, 0.0, normal_z),
                distance: end.z,
            }),
            hit,
            in_open: true,
            in_water: false,
            source_plane: Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            surface_flags: None,
        })
    }

    fn q1_miss(end: Vec3) -> TraceResult {
        TraceResult::Q1(Q1Trace {
            fraction: 1.0,
            end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            in_open: true,
            in_water: false,
            source_plane: Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            surface_flags: None,
        })
    }

    fn q1_stuck(point: Vec3) -> TraceResult {
        TraceResult::Q1(Q1Trace {
            fraction: 0.0,
            end: point,
            start_solid: true,
            all_solid: true,
            contact: TraceContact::None,
            hit: TraceHit::None,
            in_open: false,
            in_water: false,
            source_plane: Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            surface_flags: None,
        })
    }

    #[test]
    fn offsets_order_by_distance_then_y_then_x() {
        assert_eq!(
            offsets_within(8.0),
            vec![
                (0.0, 0.0),
                (0.0, -4.0),
                (-4.0, 0.0),
                (4.0, 0.0),
                (0.0, 4.0),
                (-4.0, -4.0),
                (4.0, -4.0),
                (-4.0, 4.0),
                (4.0, 4.0),
                (0.0, -8.0),
                (-8.0, 0.0),
                (8.0, 0.0),
                (0.0, 8.0),
            ]
        );
    }

    #[test]
    fn nearby_walk_finds_supported_floor_spot() {
        let (game, handles) = test_q1_game();
        let world = handles.actors.mint(&ProviderId::new("q1", "world"), "worldspawn");
        drop(game);
        let body = world_body(vec3(0.0, 0.0, 51.0), 16.0, 24.0, 32.0);
        let authored = AuthoredPlacement {
            origin: vec3(0.0, 0.0, 100.0),
            bounds: hull(16.0, 24.0, 32.0),
            locomotion: PlacementLocomotion::Walk,
        };
        let trace = |query: TraceQuery| {
            if query.start.z == 101.0 {
                return q1_result(vec3(0.0, 0.0, 50.0), 0.5, TraceHit::World { model: 0 }, 1.0);
            }
            if query.start == query.end {
                return q1_miss(query.end);
            }
            if query.end.z < query.start.z {
                return q1_result(
                    vec3(query.end.x, query.end.y, 50.0),
                    0.5,
                    TraceHit::World { model: 0 },
                    1.0,
                );
            }
            q1_miss(query.end)
        };
        let spot = nearby_monster_placement(NearbyPlacement {
            body: &body,
            locomotion: PlacementLocomotion::Walk,
            world_actor: Some(world.id().clone()),
            same_medium: |_: &Vec3| true,
            authored,
            query: base_query(),
            trace,
            blocked_by: None::<fn(&ActorId)>,
        })
        .expect("spot");
        assert_eq!(spot.origin, vec3(0.0, 0.0, 50.0));
        assert_eq!(spot.ground, Some(world.id().clone()));
    }

    #[test]
    fn nearby_routes_q1_probes_around_monsters_and_reports_blocks() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let body = world_body(vec3(0.0, 0.0, 51.0), 16.0, 24.0, 32.0);
        let authored = AuthoredPlacement {
            origin: vec3(0.0, 0.0, 100.0),
            bounds: hull(20.0, 24.0, 40.0),
            locomotion: PlacementLocomotion::Walk,
        };
        let policies: Rc<RefCell<Vec<TracePolicy>>> = Rc::new(RefCell::new(Vec::new()));
        let seen = policies.clone();
        let (_game, handles) = test_q1_game();
        let blocker = handles
            .actors
            .mint(&ProviderId::new("q1", "test"), "blocker")
            .id()
            .clone();
        let trace = move |query: TraceQuery| {
            if query.shape == TraceShape::Box(authored.bounds) {
                seen.borrow_mut().push(query.policy);
                return q1_result(query.end, 0.5, TraceHit::Actor { actor: blocker.clone() }, 1.0);
            }
            q1_miss(query.end)
        };
        let blocked: Rc<RefCell<Vec<ActorId>>> = Rc::new(RefCell::new(Vec::new()));
        let reported = blocked.clone();
        let spot = nearby_monster_placement(NearbyPlacement {
            body: &body,
            locomotion: PlacementLocomotion::Walk,
            world_actor: None,
            same_medium: |_: &Vec3| true,
            authored,
            query: base_query(),
            trace,
            blocked_by: Some(move |actor: &ActorId| reported.borrow_mut().push(actor.clone())),
        });
        assert!(spot.is_none());
        assert!(!blocked.borrow().is_empty());
        assert!(policies.borrow().iter().all(|policy| *policy
            == TracePolicy::Q1 {
                movement: Q1Move::NoMonsters
            }));
    }

    #[test]
    fn nearby_fly_returns_unsupported_spot_without_ground() {
        let body = world_body(vec3(0.0, 0.0, 100.0), 16.0, 16.0, 16.0);
        let authored = AuthoredPlacement {
            origin: vec3(64.0, 0.0, 100.0),
            bounds: hull(16.0, 16.0, 16.0),
            locomotion: PlacementLocomotion::Fly,
        };
        let spot = nearby_monster_placement(NearbyPlacement {
            body: &body,
            locomotion: PlacementLocomotion::Fly,
            world_actor: None,
            same_medium: |_: &Vec3| true,
            authored,
            query: base_query(),
            trace: |query: TraceQuery| q1_miss(query.end),
            blocked_by: None::<fn(&ActorId)>,
        })
        .expect("spot");
        assert_eq!(spot.origin, vec3(0.0, 0.0, 100.0));
        assert_eq!(spot.ground, None);
    }

    #[test]
    fn nearby_returns_none_when_enclosed() {
        let body = world_body(vec3(0.0, 0.0, 100.0), 16.0, 24.0, 32.0);
        let authored = AuthoredPlacement {
            origin: vec3(0.0, 0.0, 100.0),
            bounds: hull(16.0, 24.0, 32.0),
            locomotion: PlacementLocomotion::Walk,
        };
        let spot = nearby_monster_placement(NearbyPlacement {
            body: &body,
            locomotion: PlacementLocomotion::Walk,
            world_actor: None,
            same_medium: |_: &Vec3| true,
            authored,
            query: base_query(),
            trace: |query: TraceQuery| q1_stuck(query.start),
            blocked_by: None::<fn(&ActorId)>,
        });
        assert!(spot.is_none());
    }

    #[test]
    fn nearby_edge_walk_finds_pocket_around_corner() {
        let body = world_body(vec3(0.0, 0.0, 51.0), 16.0, 24.0, 32.0);
        let authored = AuthoredPlacement {
            origin: vec3(0.0, 0.0, 100.0),
            bounds: hull(20.0, 24.0, 40.0),
            locomotion: PlacementLocomotion::Walk,
        };
        let trace = move |query: TraceQuery| {
            if query.start.z == 101.0 {
                return q1_result(vec3(0.0, 0.0, 50.0), 0.5, TraceHit::World { model: 0 }, 1.0);
            }
            if query.shape != TraceShape::Box(authored.bounds) {
                return q1_miss(query.end);
            }
            if query.start.z - query.end.z > 17.0 {
                return q1_result(
                    vec3(query.end.x, query.end.y, 40.0),
                    0.5,
                    TraceHit::World { model: 0 },
                    1.0,
                );
            }
            q1_miss(query.end)
        };
        let spot = nearby_monster_placement(NearbyPlacement {
            body: &body,
            locomotion: PlacementLocomotion::Walk,
            world_actor: None,
            same_medium: |_: &Vec3| true,
            authored,
            query: base_query(),
            trace,
            blocked_by: None::<fn(&ActorId)>,
        })
        .expect("pocket");
        assert_eq!(spot.origin, vec3(8.0, 0.0, 40.0));
        assert_eq!(spot.ground, None);
    }

    fn trigger_game() -> (Q1EntityServices, super::super::test_hosts::Q1Handles, ActorId, ActorId) {
        let (mut game, handles) = test_q1_game();
        let trigger = game.create("trigger_teleport", None, None).expect("trigger");
        let entity = game.entities.get_mut(&trigger).expect("trigger entity");
        entity.targetname = "stage".to_string();
        entity.target = "dest".to_string();
        entity.solid = Q1Solid::Trigger;
        entity.touch = Some("teleport_touch".to_string());
        entity.spawnflags = 0;
        let dest = game.create("info_teleport_destination", None, None).expect("dest");
        game.entities.get_mut(&dest).expect("dest entity").targetname = "dest".to_string();
        handles
            .bodies
            .admit_linked(&trigger, vec3(0.0, 0.0, 0.0), hull(8.0, 8.0, 8.0));
        (game, handles, trigger, dest)
    }

    fn monster_body(origin: Vec3) -> Q1BodyState {
        Q1BodyState {
            origin,
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: hull(16.0, 24.0, 32.0),
            ground: None,
        }
    }

    #[test]
    fn staging_detects_monster_inside_active_teleport() {
        let (game, _handles, _trigger, _dest) = trigger_game();
        assert!(is_q1_teleport_staging(&game, &monster_body(vec3(0.0, 0.0, 0.0))));
        assert!(!is_q1_teleport_staging(&game, &monster_body(vec3(512.0, 0.0, 0.0))));
    }

    #[test]
    fn staging_requires_destination_and_link() {
        let (mut game, _handles, trigger, dest) = trigger_game();
        game.entities.get_mut(&trigger).expect("trigger").spawnflags = 1;
        assert!(!is_q1_teleport_staging(&game, &monster_body(vec3(0.0, 0.0, 0.0))));
        game.entities.get_mut(&trigger).expect("trigger").spawnflags = 0;
        game.entities.remove(&dest);
        assert!(!is_q1_teleport_staging(&game, &monster_body(vec3(0.0, 0.0, 0.0))));
        let (fresh, _handles) = test_q1_game();
        assert!(!is_q1_teleport_staging(&fresh, &monster_body(vec3(0.0, 0.0, 0.0))));
    }

    fn test_entities(provider: &str, content: &str) -> (ProviderReference, ContentId) {
        let (namespace, name) = provider.split_once(':').unwrap_or(("", provider));
        (
            ProviderReference {
                provider: ProviderId::new(namespace, name),
                content: ContentId(content.to_string()),
            },
            ContentId(content.to_string()),
        )
    }

    #[test]
    fn preserves_walk_knight_with_failed_floor_drop() {
        let species = species_by_classname("monster_knight").expect("knight");
        let content = "q1:id1:maps:e1m1:1";
        let (entities, geometry) = test_entities("q1:official", content);
        let authored = Q1Entity {
            properties: vec![
                ("classname".to_string(), "monster_knight".to_string()),
                ("origin".to_string(), "0 0 0".to_string()),
            ],
        };
        let definition = MonsterDefinitionReference {
            source: ProviderReference {
                provider: ProviderId::new("q1", "monsters/classic/id1"),
                content: ContentId(content.to_string()),
            },
            classname: "monster_knight".to_string(),
        };
        let (mut game, _handles) = test_q1_game();
        let id = game.create("monster_knight", None, None).expect("knight");
        let entity = game.entities.get_mut(&id).expect("entity");
        entity.movement = Q1MoveType::Step;
        entity.model = format!("progs/{}.mdl", species.model);
        entity.movement_flags = 32;
        entity.solid = Q1Solid::Slidebox;
        let entity = entity.clone();
        let body = Q1BodyState {
            origin: vec3(0.0, 0.0, 1.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: species.bounds,
            ground: None,
        };
        game.host.trace = Box::new(|request| qa_content::q1::foundation::types::Q1Trace {
            fraction: 1.0,
            end: request.end,
            normal: vec3(0.0, 0.0, 1.0),
            actor: None,
            start_solid: false,
            all_solid: false,
            sky: false,
            in_open: true,
            in_water: false,
        });
        assert!(preserves_authored_q1_placement(Q1PlacementInput {
            entities: &entities,
            geometry_content: &geometry,
            authored: Some(&authored),
            definition: &definition,
            game: &mut game,
            entity: &entity,
            body: &body,
        }));
        let (foreign_entities, foreign_geometry) = test_entities("q1:rogue", content);
        assert!(!preserves_authored_q1_placement(Q1PlacementInput {
            entities: &foreign_entities,
            geometry_content: &foreign_geometry,
            authored: Some(&authored),
            definition: &definition,
            game: &mut game,
            entity: &entity,
            body: &body,
        }));
    }

    #[test]
    fn preserves_fly_wizard_at_authored_origin() {
        let species = species_by_classname("monster_wizard").expect("wizard");
        assert_eq!(species.movement, MonsterMovement::Fly);
        let content = "q1:id1:maps:e1m1:1";
        let (entities, geometry) = test_entities("q1:official", content);
        let authored = Q1Entity {
            properties: vec![
                ("classname".to_string(), "monster_wizard".to_string()),
                ("origin".to_string(), "64 128 256".to_string()),
            ],
        };
        let definition = MonsterDefinitionReference {
            source: ProviderReference {
                provider: ProviderId::new("q1", "monsters/rerelease/id1"),
                content: ContentId(content.to_string()),
            },
            classname: "monster_wizard".to_string(),
        };
        let (mut game, _handles) = test_q1_game();
        let id = game.create("monster_wizard", None, None).expect("wizard");
        let entity = game.entities.get_mut(&id).expect("entity");
        entity.movement = Q1MoveType::Step;
        entity.model = format!("progs/{}.mdl", species.model);
        entity.movement_flags = 1;
        let entity = entity.clone();
        let body = Q1BodyState {
            origin: vec3(64.0, 128.0, 256.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: species.bounds,
            ground: None,
        };
        assert!(preserves_authored_q1_placement(Q1PlacementInput {
            entities: &entities,
            geometry_content: &geometry,
            authored: Some(&authored),
            definition: &definition,
            game: &mut game,
            entity: &entity,
            body: &body,
        }));
    }

    #[test]
    fn preserves_q2_berserk_landed_on_world() {
        let content = "q2:baseq2:maps:base1:1";
        let (entities, geometry) = test_entities("q2:official", content);
        let (mut game, handles) = test_q2_game();
        let native = q2_classic_base_monster_definitions()
            .into_iter()
            .find(|definition| definition.classname == "monster_berserk")
            .expect("berserk");
        let mut values = BTreeMap::new();
        values.insert("origin".to_string(), "0 0 0".to_string());
        values.insert("spawnflags".to_string(), "0".to_string());
        let authored = Q2SpawnFields {
            ordinal: 5,
            classname: "monster_berserk".to_string(),
            values,
        };
        let owner = ProviderId::new("q2", "test");
        let owned = handles.actors.mint(&owner, "q2:baseq2/monster_berserk");
        let mut entity = Q2Entity::new(owned.clone(), authored.clone());
        entity.motion = Q2MotionKind::Step;
        entity.flags = 0;
        entity.model.clone_from(&native.model);
        entity.scale = 1.0;
        entity.gravity = 1.0;
        entity.gravity_vector = vec3(0.0, 0.0, -1.0);
        entity.spawnflags = 0;
        game.entities.insert(owned.id().clone(), entity.clone());
        let body = Q2BodyState {
            origin: vec3(0.0, 0.0, 1.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: native.bounds,
            ground: Some(owned.id().clone()),
        };
        let definition = MonsterDefinitionReference {
            source: ProviderReference {
                provider: ProviderId::new("q2", "monsters/classic/baseq2"),
                content: ContentId(content.to_string()),
            },
            classname: "monster_berserk".to_string(),
        };
        let landed = body.origin;
        *handles.trace.borrow_mut() = Box::new(move |_request| Q2Result {
            fraction: 0.5,
            end: landed,
            start_solid: false,
            all_solid: false,
            contact: Q2Contact::None,
            hit: Q2Hit::World { model: 0 },
            family: Q2Family::Q2(Q2TraceFields {
                contents: 0,
                surface: None,
                source_plane: Q2BspPlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                    plane_type: 0,
                    signbits: 0,
                },
                secondary: None,
            }),
        });
        assert!(preserves_authored_q2_placement(Q2PlacementInput {
            entities: &entities,
            geometry_content: &geometry,
            authored: Some(&authored),
            definition: &definition,
            native: Some(&native),
            game: &mut game,
            entity: &entity,
            body: &body,
        }));
        game.options.edition = qa_content::q2::foundation::host::Q2Edition::Rerelease;
        assert!(!preserves_authored_q2_placement(Q2PlacementInput {
            entities: &entities,
            geometry_content: &geometry,
            authored: Some(&authored),
            definition: &definition,
            native: Some(&native),
            game: &mut game,
            entity: &entity,
            body: &body,
        }));
    }
}
