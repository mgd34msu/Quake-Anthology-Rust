//! Actor-body traces: sweep a query against one linked actor's body.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/body.ts`
//! (`traceActorBody`, `traceQ2Box`; the box hull is `qa_world::hull::axis_box_hull`).

use qa_core::math::{vec3, Bounds, Vec3};
use qa_world::collision::actor_contents;
use qa_world::collision::q3::TraceShape as RuntimeTraceShape;
use qa_world::collision::q3::{create_box_model, create_capsule_model, source_trace_view};
use qa_world::collision::q3::{CollisionCounters, TraceQuery as RuntimeTraceQuery};
use qa_world::hull::{axis_box_hull, trace_hull_solid};
use qa_world::spatial::{ActorCollision, CollisionFamily, CollisionShape, SpatialActor};
use qa_world::WorldError;

use crate::collision_support::{adapt_trace_result, select_numeric};
use crate::scene::{
    BspPlane, LeafContents, Q2SecondaryImpact, TraceContact, TraceDetail, TraceHit, TracePolicy, TraceQuery,
    TraceResult, TraceShape,
};

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    vec3(a.x - b.x, a.y - b.y, a.z - b.z)
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    vec3(a.x + b.x, a.y + b.y, a.z + b.z)
}

fn contents_of(collision: &ActorCollision, to: CollisionFamily) -> i32 {
    actor_contents(
        collision.family,
        collision.shape,
        collision.contents,
        collision.dead_monster,
        to,
    )
}

fn is_capsule(shape: &TraceShape) -> bool {
    matches!(shape, TraceShape::Capsule { .. })
}

fn shape_bounds(shape: &TraceShape) -> Bounds {
    match shape {
        TraceShape::Point => Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        },
        TraceShape::Box { bounds } | TraceShape::Capsule { bounds } => *bounds,
    }
}

/// Sweep a query against one actor's body (`traceActorBody`).
pub fn trace_actor_body(query: &TraceQuery, actor: &SpatialActor) -> Result<TraceResult, WorldError> {
    let body = &actor.body;
    let collision = &actor.collision;
    let bounds = body.state.bounds;
    if matches!(query.policy, TracePolicy::Q1 { .. })
        && !is_capsule(&query.shape)
        && collision.shape == CollisionShape::Box
    {
        return trace_q1_body(query, actor, &bounds);
    }
    if matches!(query.policy, TracePolicy::Q2 { .. })
        && !is_capsule(&query.shape)
        && collision.shape == CollisionShape::Box
    {
        return trace_q2_body(query, actor, &bounds);
    }
    trace_temp_model(query, actor, &bounds)
}

/// Sweep a box body through a Quake I hull.
fn trace_q1_body(query: &TraceQuery, actor: &SpatialActor, bounds: &Bounds) -> Result<TraceResult, WorldError> {
    let numeric = select_numeric(&query.numeric)?;
    let moving = shape_bounds(&query.shape);
    let expanded = Bounds {
        min: vec3(
            bounds.min.x - moving.max.x,
            bounds.min.y - moving.max.y,
            bounds.min.z - moving.max.z,
        ),
        max: vec3(
            bounds.max.x - moving.min.x,
            bounds.max.y - moving.min.y,
            bounds.max.z - moving.min.z,
        ),
    };
    let origin = actor.body.state.origin;
    let result = trace_hull_solid(
        &axis_box_hull(&expanded),
        sub(query.start, origin),
        sub(query.end, origin),
        &numeric,
    )?;
    let end = if result.fraction == 1.0 {
        query.end
    } else {
        add(result.end, origin)
    };
    Ok(TraceResult {
        fraction: result.fraction,
        end,
        start_solid: result.start_solid,
        all_solid: result.all_solid,
        contact: if result.fraction < 1.0 {
            TraceContact::Plane { plane: result.plane }
        } else {
            TraceContact::None
        },
        hit: if result.fraction < 1.0 || result.start_solid {
            TraceHit::Actor {
                actor: actor.body.actor.clone(),
            }
        } else {
            TraceHit::None
        },
        detail: TraceDetail::Q1 {
            in_open: result.in_open,
            in_water: result.in_water,
            source_plane: result.plane,
            surface_flags: None,
            contents: None,
        },
    })
}

/// Sweep a box body with Quake II epsilon planes (`traceQ2Box`).
fn trace_q2_body(query: &TraceQuery, actor: &SpatialActor, bounds: &Bounds) -> Result<TraceResult, WorldError> {
    const EPSILON: f64 = 0.03125;
    let n = select_numeric(&query.numeric)?;
    let origin = actor.body.state.origin;
    let moving = shape_bounds(&query.shape);
    let expanded = Bounds {
        min: vec3(
            n.sub(f64::from(bounds.min.x), f64::from(moving.max.x)) as f32,
            n.sub(f64::from(bounds.min.y), f64::from(moving.max.y)) as f32,
            n.sub(f64::from(bounds.min.z), f64::from(moving.max.z)) as f32,
        ),
        max: vec3(
            n.sub(f64::from(bounds.max.x), f64::from(moving.min.x)) as f32,
            n.sub(f64::from(bounds.max.y), f64::from(moving.min.y)) as f32,
            n.sub(f64::from(bounds.max.z), f64::from(moving.min.z)) as f32,
        ),
    };
    let planes = axis_box_hull(&expanded).planes;
    let mut enter = -1.0f64;
    let mut enter2 = -1.0f64;
    let mut leave = 1.0f64;
    let mut start_out = false;
    let mut get_out = false;
    let mut plane = BspPlane {
        normal: vec3(0.0, 0.0, 0.0),
        distance: 0.0,
        plane_type: 0,
        signbits: 0,
    };
    let mut secondary: Option<BspPlane> = None;
    let stationary = query.start.x == query.end.x && query.start.y == query.end.y && query.start.z == query.end.z;
    let mut misses = false;
    for (index, current) in planes.iter().enumerate() {
        let axis = match current.plane_type {
            0 => 0,
            1 => 1,
            _ => 2,
        };
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        let start_component = [query.start.x, query.start.y, query.start.z][axis];
        let end_component = [query.end.x, query.end.y, query.end.z][axis];
        let origin_component = [origin.x, origin.y, origin.z][axis];
        let d1 = n.mul(
            n.sub(
                n.sub(f64::from(start_component), f64::from(origin_component)),
                f64::from(current.distance),
            ),
            sign,
        );
        let d2 = n.mul(
            n.sub(
                n.sub(f64::from(end_component), f64::from(origin_component)),
                f64::from(current.distance),
            ),
            sign,
        );
        let distance = if sign > 0.0 {
            [bounds.max.x, bounds.max.y, bounds.max.z][axis]
        } else {
            -[bounds.min.x, bounds.min.y, bounds.min.z][axis]
        };
        if d1 > 0.0 {
            start_out = true;
        }
        if d2 > 0.0 {
            get_out = true;
        }
        if d1 > 0.0 && (d2 >= EPSILON || d2 >= d1) {
            misses = true;
            break;
        }
        if d1 <= 0.0 && d2 <= 0.0 {
            continue;
        }
        if d1 > d2 {
            let crossed = n.div(n.sub(d1, EPSILON), n.sub(d1, d2)).max(0.0);
            let kind = i32::from(current.plane_type);
            let source = BspPlane {
                normal: vec3(
                    if axis == 0 { sign as f32 } else { 0.0 },
                    if axis == 1 { sign as f32 } else { 0.0 },
                    if axis == 2 { sign as f32 } else { 0.0 },
                ),
                distance,
                plane_type: if sign < 0.0 { kind + 3 } else { kind },
                signbits: if sign < 0.0 { 1 << kind } else { 0 },
            };
            if crossed > enter {
                enter = crossed;
                plane = source;
            } else if crossed > enter2 {
                enter2 = crossed;
                secondary = Some(source);
            }
        } else {
            leave = leave.min(1.0f64.min(n.div(n.add(d1, EPSILON), n.sub(d1, d2))));
        }
    }
    let start_solid = !misses && !start_out;
    let all_solid = start_solid && !get_out;
    let merged = matches!(
        query.policy,
        TracePolicy::Q2 {
            leaf_contents: LeafContents::Merged,
            ..
        }
    );
    let fraction = if misses {
        1.0
    } else if all_solid && (stationary || merged) {
        0.0
    } else if start_solid {
        1.0
    } else if enter < leave && enter >= 0.0 {
        enter
    } else {
        1.0
    };
    let contents = if fraction < 1.0 {
        contents_of(&actor.collision, CollisionFamily::Q2)
    } else {
        0
    };
    let selected = if misses || start_solid || fraction == 1.0 {
        BspPlane {
            normal: vec3(0.0, 0.0, 0.0),
            distance: 0.0,
            plane_type: 0,
            signbits: 0,
        }
    } else {
        plane
    };
    let end = vec3(
        n.add(
            f64::from(query.start.x),
            n.mul(fraction, n.sub(f64::from(query.end.x), f64::from(query.start.x))),
        ) as f32,
        n.add(
            f64::from(query.start.y),
            n.mul(fraction, n.sub(f64::from(query.end.y), f64::from(query.start.y))),
        ) as f32,
        n.add(
            f64::from(query.start.z),
            n.mul(fraction, n.sub(f64::from(query.end.z), f64::from(query.start.z))),
        ) as f32,
    );
    Ok(TraceResult {
        fraction,
        end,
        start_solid,
        all_solid,
        contact: if fraction < 1.0 && !all_solid {
            TraceContact::Plane {
                plane: qa_core::math::Plane {
                    normal: selected.normal,
                    distance: selected.distance,
                },
            }
        } else {
            TraceContact::None
        },
        hit: if fraction < 1.0 || start_solid {
            TraceHit::Actor {
                actor: actor.body.actor.clone(),
            }
        } else {
            TraceHit::None
        },
        detail: TraceDetail::Q2 {
            contents,
            surface: None,
            source_plane: selected,
            secondary: match secondary {
                Some(runner) if fraction != 1.0 && !start_solid => Some(Q2SecondaryImpact {
                    plane: runner,
                    surface: None,
                }),
                _ => None,
            },
        },
    })
}

/// Sweep any other body through a temporary box or capsule model.
fn trace_temp_model(query: &TraceQuery, actor: &SpatialActor, bounds: &Bounds) -> Result<TraceResult, WorldError> {
    let counters = CollisionCounters::new();
    let model = if actor.collision.shape == CollisionShape::Capsule {
        create_capsule_model(*bounds, &counters, None)?
    } else {
        create_box_model(*bounds, &counters, None)?
    };
    let q3_native = matches!(query.policy, TracePolicy::Q3 { .. }) && actor.collision.family == CollisionFamily::Q3;
    let mask = match &query.policy {
        TracePolicy::Q3 { contents_mask, .. } if q3_native => *contents_mask,
        _ => 0x0200_0000,
    };
    let shape = match &query.shape {
        TraceShape::Point => RuntimeTraceShape::Point,
        TraceShape::Box { bounds } => RuntimeTraceShape::Box {
            mins: bounds.min,
            maxs: bounds.max,
        },
        TraceShape::Capsule { bounds } => RuntimeTraceShape::Capsule {
            mins: bounds.min,
            maxs: bounds.max,
        },
    };
    let result = model.transformed_trace_source(
        &RuntimeTraceQuery {
            start: query.start,
            end: query.end,
            shape,
            mask,
            model_index: None,
            curves: None,
            player_curve_clip: None,
        },
        actor.body.state.origin,
        vec3(0.0, 0.0, 0.0),
    )?;
    let view = source_trace_view(&result);
    let contact = match view.contact {
        qa_world::collision::q3::TraceContact::None => TraceContact::None,
        qa_world::collision::q3::TraceContact::Plane(plane) => TraceContact::Plane { plane },
    };
    let contents = if q3_native {
        result.contents
    } else {
        contents_of(&actor.collision, CollisionFamily::Q3)
    };
    let fraction = f64::from(result.fraction);
    let native = TraceResult {
        fraction,
        end: result.end,
        start_solid: result.start_solid,
        all_solid: result.all_solid,
        contact,
        hit: if fraction < 1.0 || result.start_solid {
            TraceHit::Actor {
                actor: actor.body.actor.clone(),
            }
        } else {
            TraceHit::None
        },
        detail: TraceDetail::Q3 {
            contents,
            surface_flags: 0,
            source_plane: BspPlane {
                normal: result.plane.normal,
                distance: result.plane.distance,
                plane_type: result.plane.plane_type,
                signbits: result.plane.signbits,
            },
        },
    };
    Ok(adapt_trace_result(&native, &query.policy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{ActorId, IdentityOwner};
    use qa_core::math::vec3;
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use qa_world::body::{BodyState, LinkedBody};
    use qa_world::spatial::CollisionRole;

    use crate::scene::{Q1MoveRule, QueryTarget};

    fn test_actor(slot: u32) -> ActorId {
        IdentityOwner::create("test").unwrap().actor(slot, 1)
    }

    fn actor(shape: CollisionShape, family: CollisionFamily) -> SpatialActor {
        SpatialActor {
            body: LinkedBody {
                actor: test_actor(1),
                state: BodyState {
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(0.0, -16.0, -16.0),
                        max: vec3(32.0, 16.0, 16.0),
                    },
                    ground: None,
                },
                absolute_bounds: Bounds {
                    min: vec3(0.0, -16.0, -16.0),
                    max: vec3(32.0, 16.0, 16.0),
                },
                link_count: 1,
            },
            collision: ActorCollision {
                family,
                shape,
                contents: 1,
                owner: None,
                role: CollisionRole::Solid,
                monster: false,
                dead_monster: false,
                q1_corpse: false,
                q3_owner: None,
            },
        }
    }

    fn query(policy: TracePolicy, shape: TraceShape) -> TraceQuery {
        TraceQuery {
            start: vec3(-50.0, 0.0, 0.0),
            end: vec3(50.0, 0.0, 0.0),
            shape,
            target: QueryTarget::World,
            policy,
            numeric: Q3_BINARY32_PROFILE,
            pass_actor: None,
        }
    }

    #[test]
    fn q1_body_uses_hull_trace() {
        let hit = trace_actor_body(
            &query(
                TracePolicy::Q1 {
                    move_rule: Q1MoveRule::Normal,
                    hull: None,
                },
                TraceShape::Point,
            ),
            &actor(CollisionShape::Box, CollisionFamily::Q1),
        )
        .expect("trace");
        assert_eq!(hit.fraction, 0.4996874928474426);
        assert_eq!(hit.end.x, -0.03125);
        match &hit.detail {
            TraceDetail::Q1 { contents, .. } => assert_eq!(*contents, None),
            _ => panic!("q1 detail"),
        }
        assert!(matches!(hit.hit, TraceHit::Actor { .. }));
    }

    #[test]
    fn q2_body_uses_epsilon_planes() {
        let hit = trace_actor_body(
            &query(
                TracePolicy::Q2 {
                    contents_mask: 1,
                    leaf_contents: LeafContents::Stored,
                },
                TraceShape::Point,
            ),
            &actor(CollisionShape::Box, CollisionFamily::Q2),
        )
        .expect("trace");
        assert_eq!(hit.fraction, 0.4996874928474426);
        assert_eq!(hit.end.x, -0.03125);
        match &hit.detail {
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane,
                secondary,
            } => {
                assert_eq!(*contents, 1);
                assert_eq!(surface, &None);
                assert_eq!(source_plane.normal, vec3(-1.0, 0.0, 0.0));
                assert_eq!(source_plane.plane_type, 3);
                assert_eq!(secondary, &None);
            }
            _ => panic!("q2 detail"),
        }
    }

    #[test]
    fn other_bodies_use_temp_models() {
        let body_policy = TracePolicy::Q3 {
            contents_mask: 0x0200_0000,
            curves: true,
            player_curve_clip: true,
        };
        let capsule = trace_actor_body(
            &query(body_policy, TraceShape::Point),
            &actor(CollisionShape::Capsule, CollisionFamily::Q3),
        )
        .expect("trace");
        assert!(capsule.fraction < 1.0);
        assert!(matches!(capsule.hit, TraceHit::Actor { .. }));
        let masked = trace_actor_body(
            &query(
                TracePolicy::Q3 {
                    contents_mask: 1,
                    curves: true,
                    player_curve_clip: true,
                },
                TraceShape::Point,
            ),
            &actor(CollisionShape::Capsule, CollisionFamily::Q3),
        )
        .expect("trace");
        assert_eq!(masked.fraction, 1.0);
        assert_eq!(masked.hit, TraceHit::None);
        let adapted = trace_actor_body(
            &query(
                TracePolicy::Q1 {
                    move_rule: Q1MoveRule::Normal,
                    hull: None,
                },
                TraceShape::Capsule {
                    bounds: Bounds {
                        min: vec3(-4.0, -4.0, -4.0),
                        max: vec3(4.0, 4.0, 4.0),
                    },
                },
            ),
            &actor(CollisionShape::Box, CollisionFamily::Q2),
        )
        .expect("trace");
        assert!(matches!(adapted.detail, TraceDetail::Q1 { .. }));
        assert!(matches!(adapted.hit, TraceHit::Actor { .. }));
    }
}
